//! Operator-declared tenant registry. Step 3 of email-multi-instance.
//!
//! Maps each tenant label (operator YAML `email[].instance`) to its
//! `Arc<EmailPlugin>` — one full plugin state machine per tenant,
//! each owning its `Vec<EmailAccountConfig>` + IMAP IDLE workers +
//! SMTP outbound queue + attachment / bounce stores.
//!
//! Mirrors the multi-instance pattern from the browser, telegram,
//! and whatsapp plugins. Lives in a process-wide `OnceLock` so the
//! boot loop, dispatch resolver, and admin RPC handlers all share
//! one map.

use std::sync::{Arc, OnceLock};

use dashmap::DashMap;

use crate::plugin::EmailPlugin;

static REGISTRY: OnceLock<Arc<DashMap<String, Arc<EmailPlugin>>>> = OnceLock::new();

fn map() -> Arc<DashMap<String, Arc<EmailPlugin>>> {
    REGISTRY.get_or_init(|| Arc::new(DashMap::new())).clone()
}

/// Register `plugin` under `label`. Overwrites any prior entry —
/// callers that need diff-aware behaviour (the boot loop) compare
/// against [`entries`] before re-registering.
pub fn register(label: &str, plugin: Arc<EmailPlugin>) {
    map().insert(label.to_string(), plugin);
}

/// Drop the entry for `label`. Returns the prior `Arc<EmailPlugin>`
/// so the caller can dispose of its IDLE workers + queues before
/// dropping the reference.
pub fn unregister(label: &str) -> Option<Arc<EmailPlugin>> {
    map().remove(label).map(|(_, v)| v)
}

/// Look up the live plugin for `label`.
pub fn lookup(label: &str) -> Option<Arc<EmailPlugin>> {
    map().get(label).map(|v| v.clone())
}

/// Snapshot every `(label, plugin)` pair. Used by the dispatcher's
/// "exactly-one-declared" compat shim + admin `list_instances`.
pub fn entries() -> Vec<(String, Arc<EmailPlugin>)> {
    map()
        .iter()
        .map(|e| (e.key().clone(), e.value().clone()))
        .collect()
}

/// Number of declared tenants. O(1).
pub fn len() -> usize {
    map().len()
}

/// Drain every entry. Tests + the boot loop's "back to legacy"
/// branch use this; production callers prefer the diff path.
pub fn clear() {
    map().clear();
}
