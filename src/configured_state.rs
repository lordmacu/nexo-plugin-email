//! Operator config slice delivered via `plugin.configure` JSON-RPC.
//!
//! 0.5.0: holds `Option<Vec<EmailPluginConfig>>` so the multi-tenant
//! array shape and the legacy 0.4.x single-map shape (normalised to
//! a 1-element vec by `EmailPluginShape::into_vec`) flow through the
//! same readers downstream.

use std::sync::Arc;
use std::sync::OnceLock;
use tokio::sync::RwLock;

use crate::config::EmailPluginConfig;

static CONFIGURED: OnceLock<Arc<RwLock<Option<Vec<EmailPluginConfig>>>>> = OnceLock::new();

/// Process-wide configured-state cell. `None` until the host sends
/// `plugin.configure`.
pub fn configured_state() -> &'static Arc<RwLock<Option<Vec<EmailPluginConfig>>>> {
    CONFIGURED.get_or_init(|| Arc::new(RwLock::new(None)))
}
