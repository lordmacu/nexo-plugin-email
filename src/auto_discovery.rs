//! Phase 81.33.b.real Stages 1+2+4+5+6 — auto-discovery broker
//! handlers for the email plugin (v0.6).
//!
//! Phase 81.20.x F1 adds Stage 1 (`pairing.adapter`) as a
//! form-based flow: the operator pastes IMAP/SMTP credentials
//! in the admin UI instead of scanning a QR. The plugin still
//! exposes `pairing_send_qr_image` as a stub returning
//! `not_supported` so the daemon's `GenericBrokerPairingAdapter`
//! can degrade gracefully when the trait call fires.
//!
//! Contract docs in the daemon repo:
//! - Pairing adapter — `proyecto/docs/src/plugins/manifest-pairing-adapter.md`
//! - HTTP routes    — `proyecto/docs/src/plugins/manifest-http.md`
//! - Admin RPC      — `proyecto/docs/src/plugins/manifest-admin.md`
//! - Metrics scrape — `proyecto/docs/src/plugins/manifest-metrics.md`
//! - Dashboard      — `proyecto/docs/src/plugins/manifest-dashboard.md`

use base64::Engine;
use serde_json::{json, Value};

use crate::configured_state;
use crate::runtime_handle;

// ── Stage 1 — pairing adapter ──────────────────────────────────

/// `plugin.email.pairing.normalize_sender` — canonicalize an
/// email address (lowercase, trim whitespace). Gmail's dot-
/// insensitivity is honoured for `@gmail.com` / `@googlemail.com`
/// — `Foo.Bar+spam@Gmail.com` collapses to `foobar@gmail.com`.
/// Returns `{ "normalized": <string> }` on success and
/// `{ "normalized": null }` when the input doesn't look like an
/// email address (daemon-side `GenericBrokerPairingAdapter`
/// caches the result either way).
pub fn pairing_normalize_sender(request: &Value) -> Value {
    let raw = request
        .get("raw")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let normalized = canonicalize_email(raw);
    json!({ "normalized": normalized })
}

/// `plugin.email.pairing.send_reply` — stub. Email pairing
/// validates connectivity via `probe_connectivity` (TCP+TLS+AUTH
/// handshake) in the setup wizard rather than dispatching a real
/// message. The daemon's `PairingChannelAdapter::send_reply`
/// trait call is invoked uniformly across channels but doesn't
/// apply to email pairing; callers should route via the regular
/// `email_send_message` tool when they need to actually send.
pub async fn pairing_send_reply(_request: &Value) -> Value {
    json!({
        "ok": false,
        "error": "email plugin pairing is connectivity-probed; send_reply not used. Use the email_send_message tool for actual sends.",
    })
}

/// `plugin.email.pairing.send_qr_image` — stub. Email is
/// form-paired (operator submits IMAP/SMTP credentials); the
/// daemon's `PairingChannelAdapter::send_qr_image` trait call
/// is invoked uniformly across channels but never applies here.
/// Return a structured `not_supported` so the daemon can log
/// and degrade.
pub async fn pairing_send_qr_image(_request: &Value) -> Value {
    json!({
        "ok": false,
        "error": "email plugin is form-paired; QR pairing not supported",
    })
}

/// Canonicalize an email address. Returns `Some(canonical)` on
/// valid input, `None` otherwise. Rules:
///   * Lowercase domain (DNS is case-insensitive).
///   * Lowercase localpart (RFC 5321 says case-sensitive, but
///     virtually no MTA in 2026 enforces — case-folding here
///     keeps the dedupe cache hot).
///   * Strip Gmail dots + plus-tags ONLY for `@gmail.com` and
///     `@googlemail.com` (their published dot-insensitivity
///     rule; every other domain treats them literally).
fn canonicalize_email(raw: &str) -> Option<String> {
    let (local, domain) = raw.split_once('@')?;
    if local.is_empty() || domain.is_empty() {
        return None;
    }
    if !domain.contains('.') {
        return None;
    }
    let local = local.to_ascii_lowercase();
    let domain = domain.to_ascii_lowercase();
    let canon_local = if matches!(domain.as_str(), "gmail.com" | "googlemail.com") {
        let no_plus = local.split('+').next().unwrap_or("");
        no_plus.replace('.', "")
    } else {
        local
    };
    if canon_local.is_empty() {
        return None;
    }
    Some(format!("{canon_local}@{domain}"))
}

// ── Stage 2 — HTTP routes ──────────────────────────────────────

pub async fn http_request(request: &Value) -> Value {
    let path = request.get("path").and_then(|v| v.as_str()).unwrap_or("/");
    let method = request
        .get("method")
        .and_then(|v| v.as_str())
        .unwrap_or("GET");
    match (method, path) {
        ("GET", "/email/health") => {
            // Phase 81.20.x F1 — health snapshot migrated from the
            // daemon's legacy `render_email_health` helper
            // (`proyecto/src/main.rs:15660`). Returns the same JSON
            // array shape so existing operator dashboards / scrapers
            // see no breakage; the snapshot is rendered inside the
            // subprocess so the daemon no longer owns the typed
            // `HealthMap` handle.
            let body = render_health_snapshot().await;
            respond(
                200,
                "application/json; charset=utf-8",
                body.to_string().as_bytes(),
            )
        }
        ("GET", "/email/status") => {
            let instances = configured_instances().await;
            let body = json!({
                "status": "ok",
                "plugin": "email",
                "version": env!("CARGO_PKG_VERSION"),
                "configured_instances": instances,
            });
            respond(
                200,
                "application/json; charset=utf-8",
                body.to_string().as_bytes(),
            )
        }
        _ => respond(
            404,
            "application/json; charset=utf-8",
            br#"{"error":"not found"}"#,
        ),
    }
}

/// Render the same JSON-array shape that the daemon's legacy
/// `render_email_health` produced. Each row carries the live
/// state of one configured account. Returns `[]` when the plugin
/// hasn't booted yet (runtime handle empty).
async fn render_health_snapshot() -> Value {
    let plugin = {
        let guard = runtime_handle::runtime_handle().read().await;
        guard.as_ref().cloned()
    };
    let Some(plugin) = plugin else {
        return json!([]);
    };
    let Some(map) = plugin.health_map().await else {
        return json!([]);
    };
    let mut keys: Vec<String> = map.iter().map(|e| e.key().clone()).collect();
    keys.sort();
    let mut rows = Vec::with_capacity(keys.len());
    for k in keys {
        if let Some(arc) = map.get(&k).map(|v| v.value().clone()) {
            let h = arc.read().await;
            rows.push(json!({
                "instance": k,
                "state": match h.state {
                    crate::health::WorkerState::Connecting => "connecting",
                    crate::health::WorkerState::Idle => "idle",
                    crate::health::WorkerState::Polling => "polling",
                    crate::health::WorkerState::Down => "down",
                },
                "last_idle_alive_ts": h.last_idle_alive_ts,
                "last_poll_ts": h.last_poll_ts,
                "last_connect_ok_ts": h.last_connect_ok_ts,
                "consecutive_failures": h.consecutive_failures,
                "messages_seen_total": h.messages_seen_total,
                "last_error": h.last_error,
                "outbound_queue_depth": h.outbound_queue_depth,
                "outbound_dlq_depth": h.outbound_dlq_depth,
                "outbound_sent_total": h.outbound_sent_total,
                "outbound_failed_total": h.outbound_failed_total,
            }));
        }
    }
    Value::Array(rows)
}

fn respond(status: u16, content_type: &str, body: &[u8]) -> Value {
    json!({
        "status": status,
        "headers": [["Content-Type", content_type]],
        "body_base64": base64::engine::general_purpose::STANDARD.encode(body),
    })
}

// ── Stage 4 — admin RPC ────────────────────────────────────────

pub async fn admin_handle(request: &Value) -> Value {
    let method = request
        .get("method")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    match method {
        "nexo/admin/email/bot_info" => {
            let instances = configured_instances().await;
            json!({
                "ok": true,
                "result": {
                    "plugin": "email",
                    "version": env!("CARGO_PKG_VERSION"),
                    "configured_instances": instances,
                },
            })
        }
        "nexo/admin/email/list_instances" => {
            // Legacy verb — flattens account-instances across every
            // tenant. Kept for back-compat with 0.4.x admin UI.
            let instances = configured_instances().await;
            json!({ "ok": true, "result": { "instances": instances } })
        }
        "nexo/admin/email/list_tenants" => {
            // 0.5.0: tenant-level enumeration with per-tenant account
            // counts + paired-status proxy (whether accounts are
            // declared at all). Sourced from configured_state +
            // instance_registry.
            let guard = crate::configured_state().read().await;
            let tenants: Vec<Value> = guard
                .as_ref()
                .map(|vec| {
                    vec.iter()
                        .map(|cfg| {
                            let label = cfg
                                .instance
                                .clone()
                                .unwrap_or_else(|| "default".into());
                            let registered =
                                crate::instance_registry::lookup(&label).is_some();
                            json!({
                                "tenant": label,
                                "accounts_count": cfg.accounts.len(),
                                "registered": registered,
                                "allow_agents": cfg.allow_agents,
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            json!({ "ok": true, "result": { "tenants": tenants } })
        }
        other => json!({
            "ok": false,
            "error": format!("unknown admin method: {other}"),
        }),
    }
}

// ── Stage 5 — metrics scrape ───────────────────────────────────

/// Threads the live `EmailPlugin` handle into `render_prometheus`
/// when present so the runtime gauges (`imap_state`,
/// `outbound_queue_depth`, `outbound_dlq_depth`) reflect current
/// state. When the runtime handle isn't yet populated (boot in
/// flight) the scrape returns counters only (gauges fall back to
/// zero — same as `render_prometheus(None)`).
pub async fn metrics_scrape(_request: &Value) -> Value {
    let health = {
        let guard = runtime_handle::runtime_handle().read().await;
        match guard.as_ref() {
            Some(plugin) => plugin.health_map().await,
            None => None,
        }
    };
    let text = crate::metrics::render_prometheus(health.as_ref()).await;
    json!({ "text": text })
}

// ── helpers ────────────────────────────────────────────────────

async fn configured_instances() -> Vec<String> {
    // 0.5.0: flatten account labels across every configured tenant.
    let guard = configured_state().read().await;
    guard
        .as_ref()
        .map(|vec| {
            vec.iter()
                .flat_map(|c| c.accounts.iter().map(|a| a.instance.clone()))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn http_get_health_serves_200() {
        let r = http_request(&json!({ "method": "GET", "path": "/email/health" })).await;
        assert_eq!(r["status"].as_u64(), Some(200));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn http_get_status_returns_plugin_metadata() {
        let r = http_request(&json!({ "method": "GET", "path": "/email/status" })).await;
        assert_eq!(r["status"].as_u64(), Some(200));
        let body_b64 = r["body_base64"].as_str().unwrap();
        let body = base64::engine::general_purpose::STANDARD
            .decode(body_b64)
            .unwrap();
        let body_str = String::from_utf8(body).unwrap();
        assert!(body_str.contains("\"plugin\":\"email\""));
        assert!(body_str.contains("\"version\""));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn http_unknown_returns_404() {
        let r = http_request(&json!({ "method": "GET", "path": "/email/missing" })).await;
        assert_eq!(r["status"].as_u64(), Some(404));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn admin_bot_info_returns_plugin_metadata() {
        let r = admin_handle(&json!({
            "method": "nexo/admin/email/bot_info",
            "params": {},
        }))
        .await;
        assert_eq!(r["ok"].as_bool(), Some(true));
        assert_eq!(r["result"]["plugin"].as_str(), Some("email"));
        assert!(r["result"]["version"].is_string());
        assert!(r["result"]["configured_instances"].is_array());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn admin_list_instances_returns_array() {
        let r = admin_handle(&json!({
            "method": "nexo/admin/email/list_instances",
            "params": {},
        }))
        .await;
        assert_eq!(r["ok"].as_bool(), Some(true));
        assert!(r["result"]["instances"].is_array());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn admin_unknown_method_returns_err() {
        let r = admin_handle(&json!({
            "method": "nexo/admin/email/nonexistent",
            "params": {},
        }))
        .await;
        assert_eq!(r["ok"].as_bool(), Some(false));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn metrics_scrape_returns_email_namespaced_metrics() {
        let r = metrics_scrape(&json!({})).await;
        let text = r["text"].as_str().expect("text");
        // render_prometheus emits email_imap_messages_fetched_total
        // + email_loop_skipped_total etc.; the empty-state shape
        // always includes the HELP/TYPE lines for at least one
        // series.
        assert!(text.contains("email_imap_messages_fetched_total"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial_test::serial]
    async fn admin_list_tenants_enumerates_configured_tenants() {
        // Wire two tenant configs into the cell; admin verb must
        // emit per-tenant rows with account counts + allow_agents.
        let cfg_a = crate::config::EmailPluginConfig {
            instance: Some("empresa_a".into()),
            allow_agents: vec!["ana".into()],
            enabled: true,
            max_body_bytes: 1024,
            max_attachment_bytes: 1024,
            attachment_retention_days: 1,
            max_dlq_lines: 1,
            bounce_retention_days: 1,
            attachments_dir: "/tmp/x".into(),
            outbound_queue_dir: "/tmp/y".into(),
            poll_fallback_seconds: 60,
            idle_reissue_minutes: 25,
            spf_dkim_warn: false,
            loop_prevention: crate::config::LoopPreventionCfg::default(),
            accounts: vec![],
        };
        let mut cfg_b = cfg_a.clone();
        cfg_b.instance = Some("empresa_b".into());
        cfg_b.allow_agents.clear();
        *crate::configured_state().write().await = Some(vec![cfg_a, cfg_b]);

        let r = admin_handle(&json!({
            "method": "nexo/admin/email/list_tenants",
            "params": {},
        }))
        .await;
        assert_eq!(r["ok"].as_bool(), Some(true), "got {r}");
        let tenants = r["result"]["tenants"].as_array().expect("tenants array");
        assert_eq!(tenants.len(), 2);
        assert_eq!(tenants[0]["tenant"].as_str(), Some("empresa_a"));
        assert_eq!(tenants[0]["allow_agents"][0].as_str(), Some("ana"));
        assert_eq!(tenants[1]["tenant"].as_str(), Some("empresa_b"));

        *crate::configured_state().write().await = None;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn admin_unknown_method_reports_error() {
        let r = admin_handle(&json!({
            "method": "nexo/admin/email/nonexistent",
            "params": {},
        }))
        .await;
        assert_eq!(r["ok"].as_bool(), Some(false));
    }
}
