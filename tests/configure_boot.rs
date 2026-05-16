//! Step 4 of email-multi-instance — coverage for `boot::apply_configure`.
//!
//! Drives the boot loop with a stub factory so no real IMAP/SMTP
//! connection is needed. Each test reset()s the global state cell +
//! registry to keep nextest parallelism honest.

use std::sync::Arc;

use nexo_plugin_email::boot::{apply_configure, resolve_tenant_data_dir, sanitize_tenant_label};
use nexo_plugin_email::config::EmailPluginConfig;
use nexo_plugin_email::plugin::EmailPlugin;
use nexo_plugin_email::{configured_state, instance_registry};
use serial_test::serial;
use std::path::{Path, PathBuf};

fn yaml(s: &str) -> serde_yaml::Value {
    serde_yaml::from_str(s).expect("yaml parses")
}

async fn reset() {
    *configured_state().write().await = None;
    instance_registry::clear();
}

fn stub_factory(
    cfg: EmailPluginConfig,
    data_dir: PathBuf,
) -> Result<Arc<EmailPlugin>, String> {
    use nexo_auth::email::EmailCredentialStore;
    use nexo_auth::google::GoogleCredentialStore;
    Ok(Arc::new(EmailPlugin::new(
        cfg,
        Arc::new(EmailCredentialStore::empty()),
        Arc::new(GoogleCredentialStore::empty()),
        data_dir,
    )))
}

#[tokio::test]
#[serial]
async fn configure_with_two_tenants_registers_both() {
    reset().await;
    let v = yaml(
        "- instance: empresa_a\n  accounts: []\n- instance: empresa_b\n  accounts: []\n",
    );
    apply_configure(v, Path::new("/tmp/nexo-email-test"), stub_factory)
        .await
        .expect("configure ok");

    assert_eq!(instance_registry::len(), 2);
    assert!(instance_registry::lookup("empresa_a").is_some());
    assert!(instance_registry::lookup("empresa_b").is_some());

    let guard = configured_state().read().await;
    let snap = guard.as_ref().expect("populated");
    assert_eq!(snap.len(), 2);
    drop(guard);
    reset().await;
}

#[tokio::test]
#[serial]
async fn configure_reload_drops_removed_tenant() {
    reset().await;
    apply_configure(
        yaml("- instance: a\n  accounts: []\n- instance: b\n  accounts: []\n- instance: c\n  accounts: []\n"),
        Path::new("/tmp/nexo-email-test"),
        stub_factory,
    )
    .await
    .expect("first configure");
    assert_eq!(instance_registry::len(), 3);

    apply_configure(
        yaml("- instance: a\n  accounts: []\n- instance: c\n  accounts: []\n"),
        Path::new("/tmp/nexo-email-test"),
        stub_factory,
    )
    .await
    .expect("reload");
    assert_eq!(instance_registry::len(), 2);
    assert!(instance_registry::lookup("a").is_some());
    assert!(instance_registry::lookup("b").is_none());
    assert!(instance_registry::lookup("c").is_some());
    reset().await;
}

#[tokio::test]
#[serial]
async fn configure_duplicate_tenant_label_errors() {
    reset().await;
    let err = apply_configure(
        yaml("- instance: dup\n  accounts: []\n- instance: dup\n  accounts: []\n"),
        Path::new("/tmp/nexo-email-test"),
        stub_factory,
    )
    .await
    .err()
    .expect("duplicate must fail");
    assert!(
        err.to_lowercase().contains("duplicate"),
        "error must mention duplicate: {err}"
    );
    // Failed configure must not leave half-populated registry.
    assert_eq!(instance_registry::len(), 0);
    reset().await;
}

#[tokio::test]
#[serial]
async fn configure_invalid_tenant_label_errors() {
    reset().await;
    let err = apply_configure(
        yaml("- instance: \"bad/label\"\n  accounts: []\n"),
        Path::new("/tmp/nexo-email-test"),
        stub_factory,
    )
    .await
    .err()
    .expect("invalid label must fail");
    assert!(
        err.to_lowercase().contains("invalid"),
        "error must mention invalid: {err}"
    );
    assert_eq!(instance_registry::len(), 0);
    reset().await;
}

#[tokio::test]
#[serial]
async fn configure_resolved_label_lowercased_and_written_back() {
    reset().await;
    apply_configure(
        yaml("- instance: Empresa_A\n  accounts: []\n"),
        Path::new("/tmp/nexo-email-test"),
        stub_factory,
    )
    .await
    .expect("ok");

    let plugin = instance_registry::lookup("empresa_a").expect("lowercased lookup");
    assert_eq!(
        plugin.config().instance.as_deref(),
        Some("empresa_a"),
        "boot loop must write the resolved label back into EmailPluginConfig.instance"
    );
    reset().await;
}

#[tokio::test]
#[serial]
async fn configure_legacy_single_object_skips_registry() {
    reset().await;
    // Single-map shape ⇒ legacy in-process path. Registry stays
    // empty; only configured_state is populated.
    apply_configure(
        yaml("max_body_bytes: 1024\naccounts: []\n"),
        Path::new("/tmp/nexo-email-test"),
        stub_factory,
    )
    .await
    .expect("configure ok");

    assert_eq!(instance_registry::len(), 0);
    let guard = configured_state().read().await;
    let snap = guard.as_ref().expect("populated");
    assert_eq!(snap.len(), 1);
    assert_eq!(snap[0].max_body_bytes, 1024);
    drop(guard);
    reset().await;
}

#[tokio::test]
#[serial]
async fn configure_default_label_when_instance_absent() {
    reset().await;
    apply_configure(
        yaml("- accounts: []\n"),
        Path::new("/tmp/nexo-email-test"),
        stub_factory,
    )
    .await
    .expect("ok");
    assert_eq!(instance_registry::len(), 1);
    assert!(instance_registry::lookup("default").is_some());
    reset().await;
}

#[test]
fn sanitize_helpers_exposed() {
    assert_eq!(sanitize_tenant_label("Empresa-1").unwrap(), "empresa-1");
    let p = resolve_tenant_data_dir(Path::new("/x"), "y");
    assert_eq!(p, PathBuf::from("/x/email/y"));
}
