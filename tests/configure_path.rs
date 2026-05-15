//! Phase 93.4.c — coverage for the configure(value) hook +
//! configured_state singleton.

use nexo_plugin_email::config::EmailPluginConfig;
use nexo_plugin_email::configured_state;
use serial_test::serial;

fn parse_yaml(s: &str) -> EmailPluginConfig {
    let value: serde_yaml::Value = serde_yaml::from_str(s).expect("yaml parses");
    serde_yaml::from_value(value).expect("config deserialises")
}

#[tokio::test]
#[serial]
async fn configure_deserialises_object_shape() {
    // Single-instance plugin: shape = "object", value is a map (NOT
    // a sequence). Defaults fill every optional field.
    let yaml = "max_body_bytes: 4096\n";
    let cfg = parse_yaml(yaml);
    assert_eq!(cfg.max_body_bytes, 4096);
    assert!(cfg.enabled, "enabled defaults to true");
    *configured_state().write().await = None;
}

#[tokio::test]
#[serial]
async fn configure_unknown_field_errors() {
    let yaml = "bogus_field: 1\n";
    let value: serde_yaml::Value = serde_yaml::from_str(yaml).unwrap();
    let res: Result<EmailPluginConfig, _> = serde_yaml::from_value(value);
    let err = res.expect_err("unknown field must fail");
    assert!(
        err.to_string().to_lowercase().contains("bogus_field"),
        "error should mention bogus_field, got: {err}",
    );
}

#[tokio::test]
#[serial]
async fn configure_overwrites_on_hot_reload_recall() {
    let cfg_a = parse_yaml("attachments_dir: /tmp/a\n");
    *configured_state().write().await = Some(cfg_a);

    let cfg_b = parse_yaml("attachments_dir: /tmp/b\n");
    *configured_state().write().await = Some(cfg_b);

    let guard = configured_state().read().await;
    let current = guard.as_ref().expect("state populated");
    assert_eq!(current.attachments_dir, "/tmp/b");
    drop(guard);
    *configured_state().write().await = None;
}

#[tokio::test]
#[serial]
async fn configure_loop_prevention_defaults_apply() {
    // Operator YAML omits all loop_prevention fields; defaults should
    // all flip to true.
    let cfg = parse_yaml("loop_prevention: {}\n");
    assert!(cfg.loop_prevention.auto_submitted);
    assert!(cfg.loop_prevention.list_headers);
    assert!(cfg.loop_prevention.spam_flag);
}

#[tokio::test]
#[serial]
async fn configured_state_holds_single_instance_not_vec() {
    // Email differs from telegram/whatsapp: single instance ⇒
    // bare struct in the cell rather than Vec<_>. This contract
    // pins that shape for future-me.
    let cfg = parse_yaml("max_body_bytes: 2048\n");
    *configured_state().write().await = Some(cfg);
    let guard = configured_state().read().await;
    let inner = guard.as_ref().expect("populated");
    // Type-check at compile time via direct field access.
    assert_eq!(inner.max_body_bytes, 2048);
    drop(guard);
    *configured_state().write().await = None;
}
