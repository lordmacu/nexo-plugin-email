//! Phase 93.4.c — plugin-owned config types.
//!
//! Until 0.1.6 this plugin re-exported `nexo_config::types::plugins::Email*`.
//! Phase 93 inverts: each plugin owns its config contract
//! (manifest's `[plugin.config_schema]` + this module's Rust
//! definitions); the daemon delivers the operator YAML opaquely
//! via `plugin.configure` JSON-RPC.
//!
//! Field shapes mirror `nexo_config::types::plugins::Email*`
//! verbatim — operator YAML keeps working unchanged.

use serde::{Deserialize, Serialize};

/// Operator YAML wire shape. The top-level `email:` key accepts
/// either a single map (legacy 0.4.x single-instance multi-account)
/// or a sequence of maps (0.5.0+ multi-tenant, each entry hosting
/// its own `accounts: Vec<EmailAccountConfig>`).
#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct EmailPluginConfigFile {
    pub email: EmailPluginShape,
}

/// 0.4.x back-compat alias. Existing tests + downstream callers
/// keep using this name; the wrapper above already takes a Shape.
pub type EmailPluginConfigFileLegacy = EmailPluginConfigFile;

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(untagged)]
pub enum EmailPluginShape {
    /// 0.4.x legacy bare map. Normalises to a 1-element vec with
    /// `instance: None` (resolved to `"default"` at boot time).
    Single(EmailPluginConfig),
    /// 0.5.0+ multi-tenant sequence. Each entry hosts its own
    /// `accounts` slice and owns isolated state.
    Many(Vec<EmailPluginConfig>),
}

impl EmailPluginShape {
    /// Flatten to the canonical `Vec<EmailPluginConfig>` the boot
    /// loop consumes. Single-map shape produces a 1-element vec so
    /// downstream code is shape-agnostic.
    pub fn into_vec(self) -> Vec<EmailPluginConfig> {
        match self {
            Self::Single(c) => vec![c],
            Self::Many(v) => v,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct EmailPluginConfig {
    /// Tenant label. `None` ⇒ legacy single-instance behaviour
    /// (resolved to `"default"` at boot time). NOT the same
    /// namespace as `EmailAccountConfig.instance` (which is the
    /// per-account label inside this tenant).
    #[serde(default)]
    pub instance: Option<String>,
    /// Agents permitted to route email tools to this tenant.
    /// Empty = accept any agent (back-compat with 0.4.x).
    #[serde(default)]
    pub allow_agents: Vec<String>,
    #[serde(default = "default_email_enabled")]
    pub enabled: bool,
    #[serde(default = "default_max_body_bytes")]
    pub max_body_bytes: usize,
    #[serde(default = "default_max_attachment_bytes")]
    pub max_attachment_bytes: usize,
    #[serde(default = "default_attachment_retention_days")]
    pub attachment_retention_days: u64,
    #[serde(default = "default_max_dlq_lines")]
    pub max_dlq_lines: usize,
    #[serde(default = "default_bounce_retention_days")]
    pub bounce_retention_days: u64,
    #[serde(default = "default_attachments_dir")]
    pub attachments_dir: String,
    #[serde(default = "default_outbound_queue_dir")]
    pub outbound_queue_dir: String,
    #[serde(default = "default_poll_fallback_seconds")]
    pub poll_fallback_seconds: u64,
    #[serde(default = "default_idle_reissue_minutes")]
    pub idle_reissue_minutes: u64,
    #[serde(default = "default_spf_dkim_warn")]
    pub spf_dkim_warn: bool,
    #[serde(default)]
    pub loop_prevention: LoopPreventionCfg,
    #[serde(default)]
    pub accounts: Vec<EmailAccountConfig>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct LoopPreventionCfg {
    #[serde(default = "default_true")]
    pub auto_submitted: bool,
    #[serde(default = "default_true")]
    pub list_headers: bool,
    #[serde(default = "default_true")]
    pub self_from: bool,
    #[serde(default = "default_true")]
    pub spam_flag: bool,
    #[serde(default = "default_true")]
    pub feedback_id: bool,
    #[serde(default = "default_true")]
    pub esp_mailer: bool,
}

impl Default for LoopPreventionCfg {
    fn default() -> Self {
        Self {
            auto_submitted: true,
            list_headers: true,
            self_from: true,
            spam_flag: true,
            feedback_id: true,
            esp_mailer: true,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct EmailAccountConfig {
    pub instance: String,
    pub address: String,
    #[serde(default = "default_provider")]
    pub provider: EmailProvider,
    pub imap: ImapEndpoint,
    pub smtp: SmtpEndpoint,
    #[serde(default)]
    pub folders: EmailFolders,
    #[serde(default)]
    pub filters: EmailFilters,
    #[serde(default)]
    pub bootstrap_limit: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EmailProvider {
    Gmail,
    Outlook,
    Yahoo,
    Icloud,
    Custom,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct ImapEndpoint {
    pub host: String,
    pub port: u16,
    #[serde(default = "default_imap_tls")]
    pub tls: TlsMode,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct SmtpEndpoint {
    pub host: String,
    pub port: u16,
    #[serde(default = "default_smtp_tls")]
    pub tls: TlsMode,
}

#[derive(Debug, Deserialize, Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TlsMode {
    Plain,
    Starttls,
    ImplicitTls,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct EmailFolders {
    #[serde(default = "default_folder_inbox")]
    pub inbox: String,
    #[serde(default = "default_folder_sent")]
    pub sent: String,
    #[serde(default = "default_folder_archive")]
    pub archive: String,
}

impl Default for EmailFolders {
    fn default() -> Self {
        Self {
            inbox: default_folder_inbox(),
            sent: default_folder_sent(),
            archive: default_folder_archive(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct EmailFilters {
    #[serde(default)]
    pub from_allowlist: Vec<String>,
    #[serde(default)]
    pub from_denylist: Vec<String>,
}

fn default_true() -> bool {
    true
}
fn default_email_enabled() -> bool {
    true
}
fn default_max_body_bytes() -> usize {
    32 * 1024
}
fn default_max_attachment_bytes() -> usize {
    25 * 1024 * 1024
}
fn default_attachment_retention_days() -> u64 {
    90
}
fn default_max_dlq_lines() -> usize {
    10_000
}
fn default_bounce_retention_days() -> u64 {
    365
}
fn default_attachments_dir() -> String {
    "data/email-attachments".to_string()
}
fn default_outbound_queue_dir() -> String {
    "data/email-outbound".to_string()
}
fn default_poll_fallback_seconds() -> u64 {
    60
}
fn default_idle_reissue_minutes() -> u64 {
    28
}
fn default_spf_dkim_warn() -> bool {
    true
}
fn default_provider() -> EmailProvider {
    EmailProvider::Custom
}
fn default_imap_tls() -> TlsMode {
    TlsMode::ImplicitTls
}
fn default_smtp_tls() -> TlsMode {
    TlsMode::Starttls
}
fn default_folder_inbox() -> String {
    "INBOX".to_string()
}
fn default_folder_sent() -> String {
    "Sent".to_string()
}
fn default_folder_archive() -> String {
    "Archive".to_string()
}
