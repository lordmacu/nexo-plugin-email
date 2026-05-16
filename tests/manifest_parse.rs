//! Step 7 of email-multi-instance — manifest parse coverage.
//!
//! Validates the bundled `nexo-plugin.toml` decodes via
//! `nexo-plugin-manifest::PluginManifest::from_str` and that the
//! multi-tenant sections are wired correctly.

use nexo_plugin_manifest::dashboard::{AuthCheck, InstanceLayout};
use nexo_plugin_manifest::PluginManifest;

const MANIFEST: &str = include_str!("../nexo-plugin.toml");

fn parse() -> PluginManifest {
    PluginManifest::from_str(MANIFEST).expect("manifest parses")
}

#[test]
fn manifest_parses_with_array_shape() {
    let m = parse();
    let cfg = m
        .plugin
        .config_schema
        .as_ref()
        .expect("config_schema present");
    let shape_dbg = format!("{:?}", cfg.shape).to_lowercase();
    assert!(
        shape_dbg.contains("array"),
        "0.5.0 must declare array shape; got {shape_dbg}"
    );
}

#[test]
fn manifest_dashboard_is_workspace_walk_with_email_subdir() {
    let m = parse();
    let dash = m.plugin.dashboard.as_ref().expect("dashboard required");
    match &dash.layout {
        InstanceLayout::WorkspaceWalk { subdir } => {
            assert_eq!(subdir, "email", "workspace_walk subdir must be `email`");
        }
        other => panic!("expected WorkspaceWalk layout, got {other:?}"),
    }
    match &dash.auth_check {
        AuthCheck::SessionDirFiles { candidates } => {
            assert!(
                candidates.iter().any(|c| c == "email_password.txt"),
                "must include the legacy password sentinel; got {candidates:?}"
            );
        }
        other => panic!("expected SessionDirFiles auth_check, got {other:?}"),
    }
}

#[test]
fn manifest_broker_allowlist_covers_admin_http_metrics() {
    let m = parse();
    let caps = &m.plugin.capabilities;
    let broker = caps
        .broker
        .as_ref()
        .expect("[plugin.capabilities.broker] required");
    let must_have = [
        "plugin.email.http.request",
        "plugin.email.admin.>",
        "plugin.email.metrics.scrape",
    ];
    for needed in must_have {
        assert!(
            broker.subscribe.iter().any(|t| t == needed),
            "broker.subscribe missing `{needed}`; declared: {:?}",
            broker.subscribe
        );
    }
}

#[test]
fn manifest_declares_required_auto_discovery_sections() {
    let m = parse();
    let p = &m.plugin;
    assert!(p.http.is_some(), "[plugin.http] required");
    assert!(p.admin.is_some(), "[plugin.admin] required");
    assert!(p.metrics.is_some(), "[plugin.metrics] required");
    assert!(p.dashboard.is_some(), "[plugin.dashboard.*] required");
    assert!(p.config_schema.is_some(), "[plugin.config_schema] required");
}
