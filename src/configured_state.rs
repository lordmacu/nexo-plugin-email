//! Phase 93.4.c — operator config slice delivered via
//! `plugin.configure` JSON-RPC (Phase 93.2). The handler in
//! `main.rs::main` records the deserialised `EmailPluginConfig`
//! here; `shared_plugin()` reads it before falling back to the
//! legacy env-var path during the deprecation window.

use std::sync::Arc;
use std::sync::OnceLock;
use tokio::sync::RwLock;

use crate::config::EmailPluginConfig;

static CONFIGURED: OnceLock<Arc<RwLock<Option<EmailPluginConfig>>>> = OnceLock::new();

/// Returns the process-wide configured-state cell, initialising
/// it on first access. The inner `Option` is `None` until the host
/// sends `plugin.configure`. Email is a single-instance plugin
/// (`shape = "object"`), so the cached value is the bare struct
/// rather than a `Vec` (unlike telegram/whatsapp).
pub fn configured_state() -> &'static Arc<RwLock<Option<EmailPluginConfig>>> {
    CONFIGURED.get_or_init(|| Arc::new(RwLock::new(None)))
}
