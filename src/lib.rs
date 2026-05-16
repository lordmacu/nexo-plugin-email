//! `nexo-plugin-email` — Email (IMAP/SMTP) channel plugin.
//!
//! Phase 48 — sub-phase 48.1 ships the scaffold + config schema only.
//! Inbound IDLE worker, SMTP outbound, MIME parse, threading, tools,
//! loop-prevention, DSN handling and SPF/DKIM checks land in 48.2..48.10.

pub mod attachment_store;
pub mod auto_discovery;
pub mod boot;
pub mod bounce_store;
pub mod config;
pub mod configured_state;
pub mod cursor;
pub mod dsn;
pub mod env_config;
pub mod events;
pub mod health;
pub mod inbound;
pub mod instance_registry;
pub mod loop_prevent;
pub mod metrics;
pub mod mime_build;
pub mod mime_parse;
pub mod outbound;
pub mod outbound_queue;
pub mod plugin;
pub mod reload;
pub mod runtime_handle;
pub mod subprocess_dispatch;
pub mod threading;
pub mod tool;

// Phase 81.20.x F0 — IMAP/SMTP probe + provider hints + SPF/DKIM
// alignment check extracted to `nexo-email-probe`. Re-export the
// modules so existing `crate::imap_conn::*` / `crate::smtp_conn::*`
// / `crate::provider_hint::*` / `crate::spf_dkim::*` callers
// inside this plugin keep compiling unchanged.
pub use nexo_email_probe::imap_conn;
pub use nexo_email_probe::provider_hint;
pub use nexo_email_probe::smtp_conn;
pub use nexo_email_probe::spf_dkim;

pub use attachment_store::AttachmentStore;
pub use bounce_store::{BounceStore, RecipientStatus};
pub use config::{
    EmailAccountConfig, EmailFilters, EmailFolders, EmailPluginConfig, EmailPluginConfigFile,
    EmailPluginShape, EmailProvider, ImapEndpoint, LoopPreventionCfg, SmtpEndpoint, TlsMode,
};
pub use configured_state::configured_state;
pub use cursor::{CursorStore, UidCursor};
pub use dsn::{parse_bounce, BounceClassification, BounceEvent, ParsedBounce};
pub use env_config::{email_config_from_env, EmailSubprocessBoot, EnvConfigError};
pub use events::{AckStatus, InboundEvent, OutboundAck, OutboundCommand};
pub use health::{AccountHealth, WorkerState};
pub use loop_prevent::{should_skip, SkipReason};
pub use mime_build::{build_mime, generate_message_id, BuildContext};
pub use mime_parse::{parse_eml, ParseConfig, ParsedMessage};
pub use outbound::OutboundDispatcher;
pub use outbound_queue::{OutboundJob, OutboundQueue, SmtpEnvelope};
pub use plugin::{EmailPlugin, TOPIC_INBOUND, TOPIC_OUTBOUND};
// Phase 81.20.x F0 — `SmtpClient`, `check_alignment`, et al. now
// live in `nexo-email-probe`. The crate-level `pub use` re-exports
// of `imap_conn` / `smtp_conn` / `provider_hint` / `spf_dkim`
// modules above already expose them under the same paths. The
// function `provider_hint::provider_hint` collides at the root
// with the module re-export, so it stays reachable only as
// `crate::provider_hint::provider_hint` (callers using the bare
// function name should `use nexo_plugin_email::provider_hint::provider_hint`
// or migrate to `nexo_email_probe` directly).
pub use nexo_email_probe::{
    check_alignment, decide_warns, parse_spf_includes, AlignmentReport, ProviderHint, SmtpClient,
    SmtpSendOutcome,
};
pub use reload::{compute_account_diff, AccountDiff};
pub use subprocess_dispatch::{dispatch_email_tool, email_tool_defs};
pub use threading::{
    canonicalize_message_id, enrich_reply_threading, is_self_thread, resolve_thread_root,
    session_id_for_thread, truncate_references, EMAIL_NS,
};
pub use tool::{
    filter_from_allowed_patterns, imap_date, imap_quote, register_email_tools,
    register_email_tools_filtered, run_imap_op, DispatcherHandle, EmailToolContext,
    EMAIL_TOOL_NAMES,
};

use std::path::PathBuf;
use std::sync::Arc;

use nexo_auth::email::EmailCredentialStore;
use nexo_auth::google::GoogleCredentialStore;
use nexo_core::agent::nexo_plugin_registry::PluginFactory;
use nexo_core::agent::plugin_host::NexoPlugin;

/// Phase 81.12.d — factory builder for the email plugin. Email is a
/// **single-plugin / multi-account-internal** model: one factory call
/// returns one plugin handle that fans out across `cfg.accounts`. The
/// closure clones the four constructor arguments per call (`cfg` is
/// `Clone`; `creds` and `google` are `Arc<>` so the clone is just a
/// refcount bump; `data_dir` is a small `PathBuf`).
///
/// Today (81.12.d): exported but no caller registers it — main.rs's
/// legacy block at `src/main.rs:1914-1937` keeps constructing
/// `EmailPlugin` directly via `register_arc`. 81.12.e flips that block
/// to use this factory.
pub fn email_plugin_factory(
    cfg: EmailPluginConfig,
    creds: Arc<EmailCredentialStore>,
    google: Arc<GoogleCredentialStore>,
    data_dir: PathBuf,
) -> PluginFactory {
    Box::new(move |_manifest| {
        let plugin: Arc<dyn NexoPlugin> = Arc::new(EmailPlugin::new(
            cfg.clone(),
            creds.clone(),
            google.clone(),
            data_dir.clone(),
        ));
        Ok(plugin)
    })
}
