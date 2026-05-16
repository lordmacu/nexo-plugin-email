//! Step 4 of email-multi-instance — boot-loop for the
//! `plugin.configure` JSON-RPC payload.
//!
//! Two paths:
//! - `Single` shape ⇒ legacy back-compat: populate `configured_state`
//!   only; existing daemon-side in-process factory still services
//!   the single EmailPlugin. Any prior registry entries are
//!   unregistered (transition from multi-tenant back to legacy).
//! - `Many` shape ⇒ multi-tenant declarative path: sanitise each
//!   tenant label, resolve per-tenant `data_dir`, register an
//!   `Arc<EmailPlugin>` per entry via the caller-supplied factory.
//!   Diff vs prior labels and drop removed ones.
//!
//! `apply_configure` is generic over a factory closure so tests +
//! callers in the daemon can construct EmailPlugin with their own
//! credential stores. The subprocess `main.rs` wraps this with a
//! closure that builds stores from env-var-seeded paths.

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use crate::config::{EmailPluginConfig, EmailPluginShape};
use crate::plugin::EmailPlugin;
use crate::{configured_state, instance_registry};

/// Sanitise a tenant label. Mirrors the rules used elsewhere in the
/// nexo ecosystem: ASCII alphanumeric + `_` + `-`, lowercased, max
/// 64 chars. Rejects empty / control / path-traversal inputs.
pub fn sanitize_tenant_label(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("tenant label is empty".to_string());
    }
    if trimmed.chars().count() > 64 {
        return Err(format!(
            "tenant label `{trimmed}` exceeds 64 characters"
        ));
    }
    for c in trimmed.chars() {
        if !(c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            return Err(format!(
                "tenant label `{trimmed}` has invalid characters (allowed: [A-Za-z0-9_-])"
            ));
        }
    }
    Ok(trimmed.to_ascii_lowercase())
}

/// Resolve the per-tenant `data_dir` rooted at `base`. Tenants get
/// `<base>/email/<tenant>/`; the legacy single-tenant default is
/// `<base>/email/`.
pub fn resolve_tenant_data_dir(base: &Path, tenant: &str) -> std::path::PathBuf {
    base.join("email").join(tenant)
}

/// Apply an `plugin.configure` payload.
///
/// `factory` constructs an `EmailPlugin` from a (resolved) cfg +
/// per-tenant data_dir. Tests pass a stub; production wraps the
/// daemon-side credential stores via closure capture.
pub async fn apply_configure<F>(
    value: serde_yaml::Value,
    data_root: &Path,
    factory: F,
) -> Result<(), String>
where
    F: Fn(EmailPluginConfig, std::path::PathBuf) -> Result<Arc<EmailPlugin>, String>,
{
    let shape: EmailPluginShape = serde_yaml::from_value(value)
        .map_err(|e| format!("invalid email config: {e}"))?;

    let configs: Vec<EmailPluginConfig> = match shape {
        EmailPluginShape::Single(c) => {
            // Legacy: transition back to single-tenant. Drop any
            // declared-tenant entries from a prior multi-tenant
            // configure call.
            shutdown_all_registered().await;
            *configured_state().write().await = Some(vec![c]);
            return Ok(());
        }
        EmailPluginShape::Many(v) => v,
    };

    // Resolve labels + per-tenant data_dirs up front so a duplicate
    // or invalid label aborts before we touch the registry.
    let mut resolved: Vec<(String, EmailPluginConfig, std::path::PathBuf)> =
        Vec::with_capacity(configs.len());
    let mut seen: HashSet<String> = HashSet::new();
    for mut cfg in configs.into_iter() {
        let raw_label = cfg.instance.clone().unwrap_or_else(|| "default".into());
        let label = sanitize_tenant_label(&raw_label)?;
        if !seen.insert(label.clone()) {
            return Err(format!("duplicate email tenant label: `{label}`"));
        }
        // Write the resolved label back so admin RPC + metrics see
        // the canonical form.
        cfg.instance = Some(label.clone());
        let data_dir = resolve_tenant_data_dir(data_root, &label);
        resolved.push((label, cfg, data_dir));
    }

    // Diff vs prior registry: unregister labels no longer present.
    let prev: HashSet<String> = instance_registry::entries()
        .into_iter()
        .map(|(k, _)| k)
        .collect();
    let now: HashSet<String> = resolved.iter().map(|(k, _, _)| k.clone()).collect();
    for stale in prev.difference(&now) {
        if let Some(_p) = instance_registry::unregister(stale) {
            tracing::info!(
                target: "plugin.email",
                tenant = %stale,
                "unregistered stale tenant"
            );
        }
    }

    // Construct + register each declared tenant.
    let mut snapshot: Vec<EmailPluginConfig> = Vec::with_capacity(resolved.len());
    for (label, cfg, data_dir) in resolved {
        snapshot.push(cfg.clone());
        let plugin = factory(cfg, data_dir).map_err(|e| {
            format!("factory failed for tenant `{label}`: {e}")
        })?;
        instance_registry::register(&label, plugin);
        tracing::info!(
            target: "plugin.email",
            tenant = %label,
            "registered declared tenant"
        );
    }
    *configured_state().write().await = Some(snapshot);
    Ok(())
}

async fn shutdown_all_registered() {
    for (label, _) in instance_registry::entries() {
        if let Some(_p) = instance_registry::unregister(&label) {
            tracing::info!(
                target: "plugin.email",
                tenant = %label,
                "single-shape transition: unregistered tenant"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_accepts_canonical_label() {
        assert_eq!(sanitize_tenant_label("empresa_a").unwrap(), "empresa_a");
        assert_eq!(sanitize_tenant_label("Marketing").unwrap(), "marketing");
        assert_eq!(sanitize_tenant_label("t-01").unwrap(), "t-01");
    }

    #[test]
    fn sanitize_rejects_empty_and_dotdot_and_unicode() {
        assert!(sanitize_tenant_label("").is_err());
        assert!(sanitize_tenant_label("   ").is_err());
        assert!(sanitize_tenant_label("..").is_err());
        assert!(sanitize_tenant_label("a/b").is_err());
        assert!(sanitize_tenant_label("ana.es").is_err());
        assert!(sanitize_tenant_label("ana-ñ").is_err());
    }

    #[test]
    fn sanitize_rejects_too_long() {
        let s = "a".repeat(65);
        assert!(sanitize_tenant_label(&s).is_err());
    }

    #[test]
    fn resolve_tenant_data_dir_joins_email_then_tenant() {
        let p = resolve_tenant_data_dir(Path::new("/var/lib/nexo"), "empresa_a");
        assert_eq!(p, std::path::PathBuf::from("/var/lib/nexo/email/empresa_a"));
    }
}
