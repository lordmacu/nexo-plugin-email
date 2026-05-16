//! Process-wide live-plugin handle for the auto-discovery
//! handlers in `auto_discovery.rs`. `main.rs::boot_plugin` writes
//! the cell after `EmailPlugin::start` succeeds; auto-discovery
//! `metrics_scrape` reads it to thread the current `HealthMap`
//! into `render_prometheus` so gauges (`imap_state`,
//! `outbound_queue_depth`, `outbound_dlq_depth`) reflect runtime
//! state instead of the legacy `None` placeholder.

use std::sync::Arc;
use std::sync::OnceLock;
use tokio::sync::RwLock;

use crate::plugin::EmailPlugin;

static RUNTIME: OnceLock<Arc<RwLock<Option<Arc<EmailPlugin>>>>> = OnceLock::new();

pub fn runtime_handle() -> &'static Arc<RwLock<Option<Arc<EmailPlugin>>>> {
    RUNTIME.get_or_init(|| Arc::new(RwLock::new(None)))
}

pub async fn set_runtime_handle(plugin: Arc<EmailPlugin>) {
    *runtime_handle().write().await = Some(plugin);
}
