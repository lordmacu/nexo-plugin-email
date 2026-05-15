//! Phase 93.8.c — coverage for the on_credentials_* handler logic
//! (handlers live inside `main.rs` and aren't directly callable
//! from integration tests; these tests exercise the same
//! `configured_state()`-backed lookup logic inline).

use nexo_plugin_email::config::{EmailAccountConfig, EmailPluginConfig};
use nexo_plugin_email::configured_state;
use serial_test::serial;

fn parse_cfg(yaml: &str) -> EmailPluginConfig {
    let wrapper: nexo_plugin_email::config::EmailPluginConfigFile =
        serde_yaml::from_str(yaml).unwrap();
    wrapper.email
}

async fn list_handler() -> Vec<String> {
    let guard = configured_state().read().await;
    guard
        .as_ref()
        .map(|c| c.accounts.iter().map(|a| a.instance.clone()).collect())
        .unwrap_or_default()
}

async fn issue_handler(account_id: &str) -> Result<(), String> {
    let guard = configured_state().read().await;
    let Some(cfg) = guard.as_ref() else {
        return Err("not_found".to_string());
    };
    if cfg.accounts.iter().any(|a| a.instance == account_id) {
        Ok(())
    } else {
        Err("not_found".to_string())
    }
}

async fn resolve_bytes_handler(account_id: &str) -> Result<Vec<u8>, String> {
    let guard = configured_state().read().await;
    let Some(cfg) = guard.as_ref() else {
        return Err("not_found".to_string());
    };
    let acct = cfg
        .accounts
        .iter()
        .find(|a| a.instance == account_id)
        .ok_or_else(|| "not_found".to_string())?;
    serde_json::to_vec(acct).map_err(|e| format!("serialize failed: {e}"))
}

const FIXTURE: &str = r#"
email:
  enabled: true
  max_body_bytes: 1048576
  max_attachment_bytes: 26214400
  attachment_retention_days: 7
  max_dlq_lines: 1000
  bounce_retention_days: 14
  attachments_dir: ./data/email/attachments
  outbound_queue_dir: ./data/email/queue
  poll_fallback_seconds: 60
  idle_reissue_minutes: 25
  spf_dkim_warn: true
  loop_prevention:
    auto_submitted: true
    list_headers: true
    self_from: true
    spam_flag: true
    feedback_id: true
    esp_mailer: true
  accounts:
    - instance: primary
      address: a@example.com
      provider: custom
      imap:
        host: imap.example.com
        port: 993
        tls: implicit_tls
      smtp:
        host: smtp.example.com
        port: 587
        tls: starttls
      folders: {}
      filters: {}
    - instance: secondary
      address: b@example.com
      provider: custom
      imap:
        host: imap.example.com
        port: 993
        tls: implicit_tls
      smtp:
        host: smtp.example.com
        port: 587
        tls: starttls
      folders: {}
      filters: {}
"#;

#[tokio::test]
#[serial]
async fn list_returns_account_instance_names() {
    *configured_state().write().await = Some(parse_cfg(FIXTURE));
    let accounts = list_handler().await;
    assert_eq!(accounts.len(), 2);
    assert!(accounts.contains(&"primary".to_string()));
    assert!(accounts.contains(&"secondary".to_string()));
    *configured_state().write().await = None;
}

#[tokio::test]
#[serial]
async fn issue_accepts_known_instance() {
    *configured_state().write().await = Some(parse_cfg(FIXTURE));
    issue_handler("primary").await.expect("accepted");
    *configured_state().write().await = None;
}

#[tokio::test]
#[serial]
async fn issue_rejects_unknown_instance() {
    *configured_state().write().await = Some(parse_cfg(FIXTURE));
    let err = issue_handler("ghost").await.expect_err("expected not_found");
    assert_eq!(err, "not_found");
    *configured_state().write().await = None;
}

#[tokio::test]
#[serial]
async fn issue_rejects_when_no_configured_state() {
    *configured_state().write().await = None;
    let err = issue_handler("primary")
        .await
        .expect_err("expected not_found");
    assert_eq!(err, "not_found");
}

#[tokio::test]
#[serial]
async fn resolve_bytes_returns_serde_json_encoded_account() {
    *configured_state().write().await = Some(parse_cfg(FIXTURE));
    let bytes = resolve_bytes_handler("primary").await.expect("resolve ok");
    let decoded: EmailAccountConfig = serde_json::from_slice(&bytes).expect("round-trip");
    assert_eq!(decoded.instance, "primary");
    assert_eq!(decoded.address, "a@example.com");
    *configured_state().write().await = None;
}
