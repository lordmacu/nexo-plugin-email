//! Subprocess entrypoint for `nexo-plugin-email` (Phase 81.19.b).
//!
//! Wires:
//!   - [`PluginAdapter`] — child-side JSON-RPC dispatch loop.
//!   - [`email_tool_defs`] — currently empty (subprocess does not
//!     dispatch tools; see `subprocess_dispatch` module docs).
//!   - [`dispatch_email_tool`] — fall-through that returns
//!     `NotFound` for any tool the daemon mistakenly routes here.
//!   - one [`EmailPlugin`] per process, eagerly booted at startup
//!     so IMAP IDLE workers + SMTP outbound dispatcher come up
//!     before any inbound mail / outbound command arrives on the
//!     broker.
//!
//! Configuration flows from the daemon via env vars set by
//! `proyecto/src/main.rs::seed_email_subprocess_env_for`:
//!
//!   * `NEXO_BROKER_URL`                       — broker URL
//!   * `NEXO_PLUGIN_EMAIL_CONFIG_PATH`         — `email.yaml`
//!   * `NEXO_PLUGIN_EMAIL_GOOGLE_AUTH_PATH`    — `google-auth.yaml`
//!                                                (empty if unused)
//!   * `NEXO_PLUGIN_EMAIL_SECRETS_DIR`         — `secrets/`
//!   * `NEXO_PLUGIN_EMAIL_DATA_DIR`            — SQLite root
//!
//! Unlike telegram / whatsapp, email is single-process /
//! multi-account-internal: one boot covers every account
//! declared in the YAML.

use std::sync::Arc;

use nexo_broker::AnyBroker;
use nexo_core::agent::plugin::Plugin;
use nexo_microapp_sdk::plugin::{PluginAdapter, ToolInvocation};
use nexo_plugin_email::{
    dispatch_email_tool, email_config_from_env, email_tool_defs, EmailPlugin,
};
use once_cell::sync::Lazy;
use tokio::sync::OnceCell;

const MANIFEST: &str = include_str!("../nexo-plugin.toml");

/// Process-wide [`EmailPlugin`]. Eagerly booted in `main` so IMAP
/// IDLE workers + SMTP queue come up before broker subscribers
/// attach. The `OnceCell` avoids re-boot if the daemon ever sends
/// a tool.invoke (which the dispatcher rejects with NotFound).
static PLUGIN: Lazy<OnceCell<Arc<EmailPlugin>>> = Lazy::new(OnceCell::new);

async fn boot_plugin() -> anyhow::Result<Arc<EmailPlugin>> {
    let boot = email_config_from_env()
        .map_err(|e| anyhow::anyhow!("env config: {e}"))?;

    let broker_url = std::env::var("NEXO_BROKER_URL")
        .map_err(|_| anyhow::anyhow!("NEXO_BROKER_URL not set — daemon must seed it"))?;

    // Build a `BrokerInner` from the seeded URL. Auth / persistence
    // / limits / fallback all default — the daemon already chose
    // those for the parent process and the subprocess just needs
    // the connection URL to reach the same NATS server.
    let broker_inner = nexo_config::types::broker::BrokerInner {
        kind: if broker_url.starts_with("nats://") {
            nexo_config::types::broker::BrokerKind::Nats
        } else {
            nexo_config::types::broker::BrokerKind::Local
        },
        url: broker_url,
        auth: nexo_config::types::broker::BrokerAuthConfig::default(),
        persistence: nexo_config::types::broker::BrokerPersistenceConfig::default(),
        limits: nexo_config::types::broker::BrokerLimitsConfig::default(),
        fallback: nexo_config::types::broker::BrokerFallbackConfig::default(),
    };

    let broker = AnyBroker::from_config(&broker_inner)
        .await
        .map_err(|e| anyhow::anyhow!("broker connect failed: {e}"))?;

    let plugin = Arc::new(EmailPlugin::new(
        boot.cfg,
        boot.creds,
        boot.google,
        boot.data_dir,
    ));

    // `start` opens IMAP IDLE workers per account, arms the SMTP
    // outbound dispatcher, and subscribes to broker topics. Any
    // partial failure surfaces here so the daemon supervisor can
    // restart with backoff. Once booted, IMAP / SMTP outages are
    // handled by per-worker retry policies inside `EmailPlugin`.
    plugin
        .start(broker.clone())
        .await
        .map_err(|e| anyhow::anyhow!("email plugin start failed: {e}"))?;

    // Phase 81.33.b.real v0.4 — populate the runtime handle so
    // `auto_discovery::metrics_scrape` can read live HealthMap
    // gauges instead of falling back to None.
    nexo_plugin_email::runtime_handle::set_runtime_handle(plugin.clone()).await;

    // Phase 81.33.b.real v0.4 — spawn auto-discovery broker
    // subscribers (HTTP routes, admin RPC, metrics scrape) on the
    // same broker the plugin uses. Failure isolation per task; a
    // dropped subscriber doesn't take down the plugin process.
    spawn_auto_discovery_subscribers(broker);

    tracing::info!(
        target = "nexo_plugin_email",
        accounts = plugin.config().accounts.len(),
        "email subprocess plugin ready"
    );
    Ok(plugin)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Phase 81.20.x F1 — Stage 8 cargo-install ergonomics. When
    // the daemon's binary-mode discovery walker probes us with
    // `nexo-plugin-email --print-manifest` we emit the bundled
    // TOML to stdout and exit 0 BEFORE tracing init / broker
    // wiring — the walker needs only the manifest bytes.
    nexo_microapp_sdk::plugin::print_manifest_if_requested(MANIFEST);

    // CRITICAL: tracing MUST write to stderr — stdout is reserved
    // for JSON-RPC framing. Without `with_writer(io::stderr)` the
    // default subscriber would emit to stdout and corrupt every
    // reply with ANSI escape codes + log lines.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // rustls 0.23 requires an explicit process-wide CryptoProvider
    // before `ClientConfig::builder()` can return successfully.
    // Same dance as the proyecto daemon (see proyecto/src/main.rs).
    let _ = rustls::crypto::ring::default_provider().install_default();

    // Build the JSON-RPC adapter first so `initialize` replies
    // immediately even if the broker / IMAP / SMTP boot is slow.
    // The plugin boot kicks off in a detached task that populates
    // the static `OnceCell` once IMAP/SMTP are live.
    let adapter = PluginAdapter::new(MANIFEST)?
        .declare_tools(email_tool_defs())
        // Phase 93.4.c — receive the operator YAML slice via the
        // host's `plugin.configure` JSON-RPC (Phase 93.2). Single-
        // instance shape per manifest `[plugin.config_schema]
        // shape = "object"`.
        .on_configure(|value: serde_yaml::Value| async move {
            // 0.5.0: accept both the legacy bare-map shape and the
            // multi-tenant sequence via the untagged Shape enum.
            // configured_state stores Vec<EmailPluginConfig>; the
            // legacy reader picks the first entry.
            let shape: nexo_plugin_email::config::EmailPluginShape =
                serde_yaml::from_value(value)
                    .map_err(|e| format!("invalid email config: {e}"))?;
            *nexo_plugin_email::configured_state().write().await =
                Some(shape.into_vec());
            Ok(())
        })
        // Phase 93.8.c — credential store contribution. Daemon's
        // `RemoteCredentialStore` round-trips through these four
        // handlers (per `[plugin.credentials_schema]`). Accounts
        // map to `EmailPluginConfig.accounts[*].instance`.
        .on_credentials_list(|| async move {
            // 0.5.0: flatten accounts across every configured tenant.
            let guard = nexo_plugin_email::configured_state().read().await;
            let accounts: Vec<String> = guard
                .as_ref()
                .map(|vec| {
                    vec.iter()
                        .flat_map(|c| c.accounts.iter().map(|a| a.instance.clone()))
                        .collect()
                })
                .unwrap_or_default();
            Ok(nexo_microapp_sdk::plugin::CredentialsListReply {
                accounts,
                warnings: Vec::new(),
            })
        })
        .on_credentials_issue(|account_id: String, _agent_id: String| async move {
            let guard = nexo_plugin_email::configured_state().read().await;
            let Some(vec) = guard.as_ref() else {
                return Err("not_found".to_string());
            };
            if vec
                .iter()
                .any(|c| c.accounts.iter().any(|a| a.instance == account_id))
            {
                Ok(())
            } else {
                Err("not_found".to_string())
            }
        })
        .on_credentials_resolve_bytes(
            |account_id: String, _agent_id: String, _fingerprint: String| async move {
                let guard = nexo_plugin_email::configured_state().read().await;
                let Some(vec) = guard.as_ref() else {
                    return Err("not_found".to_string());
                };
                let acct = vec
                    .iter()
                    .flat_map(|c| c.accounts.iter())
                    .find(|a| a.instance == account_id)
                    .ok_or_else(|| "not_found".to_string())?;
                serde_json::to_vec(acct).map_err(|e| format!("serialize failed: {e}"))
            },
        )
        .on_credentials_reload(|| async move { Ok(()) })
        .on_tool(|invocation: ToolInvocation| async move {
            // Lazy access — if boot hasn't finished, this branch
            // still returns NotFound (subprocess advertises zero
            // tools in 81.19.b; see subprocess_dispatch.rs docs).
            dispatch_email_tool(invocation).await
        });

    // Detach the eager-boot. If it fails the supervisor sees a
    // process exit on the next inbound/outbound mail attempt
    // (broker subscribers were never armed). For the initialize
    // handshake the adapter is already ready, so the daemon can
    // discover and configure us.
    tokio::spawn(async {
        match boot_plugin().await {
            Ok(p) => {
                let _ = PLUGIN.set(p);
            }
            Err(e) => {
                tracing::error!(target = "nexo_plugin_email", error = %e, "boot failed");
            }
        }
    });

    adapter.run_stdio().await?;
    Ok(())
}

/// Phase 81.33.b.real v0.4 — auto-discovery broker subscriber
/// loop. Spawns one tokio task per request-reply topic family.
/// Each task subscribes, parses `Message` from each inbound
/// `Event.payload`, dispatches to the matching async handler,
/// and publishes the reply back to `msg.reply_to`.
fn spawn_auto_discovery_subscribers(broker: AnyBroker) {
    use nexo_plugin_email::auto_discovery as ad;

    // Phase 81.20.x F1 — Stage 1 pairing adapter subscribers.
    spawn_one(
        broker.clone(),
        "plugin.email.pairing.normalize_sender",
        |_b, p| async move { ad::pairing_normalize_sender(&p) },
    );
    spawn_one(
        broker.clone(),
        "plugin.email.pairing.send_reply",
        |_b, p| async move { ad::pairing_send_reply(&p).await },
    );
    spawn_one(
        broker.clone(),
        "plugin.email.pairing.send_qr_image",
        |_b, p| async move { ad::pairing_send_qr_image(&p).await },
    );

    spawn_one(broker.clone(), "plugin.email.http.request", |_b, p| async move {
        ad::http_request(&p).await
    });
    spawn_one(broker.clone(), "plugin.email.metrics.scrape", |_b, p| async move {
        ad::metrics_scrape(&p).await
    });
    spawn_one(broker, "plugin.email.admin.>", |_b, p| async move {
        ad::admin_handle(&p).await
    });
}

fn spawn_one<F, Fut>(broker: AnyBroker, topic: &'static str, handler: F)
where
    F: Fn(AnyBroker, serde_json::Value) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = serde_json::Value> + Send + 'static,
{
    use nexo_broker::{BrokerHandle, Event, Message};
    tokio::spawn(async move {
        let mut sub = match broker.subscribe(topic).await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(
                    target = "email.auto_discovery",
                    topic,
                    error = %e,
                    "subscribe failed; topic will not receive requests"
                );
                return;
            }
        };
        tracing::info!(target = "email.auto_discovery", topic, "subscriber up");
        while let Some(event) = sub.next().await {
            let Ok(msg) = serde_json::from_value::<Message>(event.payload) else {
                continue;
            };
            let Some(reply_to) = msg.reply_to.clone() else {
                continue;
            };
            let reply_payload = handler(broker.clone(), msg.payload.clone()).await;
            let reply_msg = Message::new(reply_to.clone(), reply_payload);
            let reply_event = Event::new(
                reply_to.clone(),
                "email",
                match serde_json::to_value(&reply_msg) {
                    Ok(v) => v,
                    Err(_) => continue,
                },
            );
            if let Err(e) = broker.publish(&reply_to, reply_event).await {
                tracing::warn!(
                    target = "email.auto_discovery",
                    topic,
                    reply_to = %reply_to,
                    error = %e,
                    "failed to publish reply"
                );
            }
        }
        tracing::debug!(target = "email.auto_discovery", topic, "subscriber stream ended");
    });
}
