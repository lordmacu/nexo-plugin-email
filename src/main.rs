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
        .start(broker)
        .await
        .map_err(|e| anyhow::anyhow!("email plugin start failed: {e}"))?;

    tracing::info!(
        target = "nexo_plugin_email",
        accounts = plugin.config().accounts.len(),
        "email subprocess plugin ready"
    );
    Ok(plugin)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
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
            let parsed: nexo_plugin_email::config::EmailPluginConfig =
                serde_yaml::from_value(value)
                    .map_err(|e| format!("invalid email config: {e}"))?;
            *nexo_plugin_email::configured_state().write().await = Some(parsed);
            Ok(())
        })
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
