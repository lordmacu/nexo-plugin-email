//! Phase 81.19.b — subprocess JSON-RPC dispatch helpers.
//!
//! Email is structurally different from telegram / whatsapp: tools
//! that send mail / search IMAP / move attachments share heavy
//! in-process state (IMAP IDLE workers, SMTP queue, SQLite stores
//! for cursor / threading / bounce / attachment-refs). Re-exposing
//! all 12 email tools as subprocess JSON-RPC handlers means
//! re-implementing the dispatchers without their `ToolRegistry`
//! plumbing — a substantial, distinct effort.
//!
//! For 81.19.b the subprocess advertises **zero tool defs** in its
//! `initialize` reply. Tool dispatch stays in-process: callers that
//! need email tools (the daemon's `register_email_tools_filtered`,
//! the MCP autonomous worker) consume `EmailPlugin` directly via
//! the lib surface and register tools against their local
//! `ToolRegistry`.
//!
//! What the subprocess DOES handle:
//!   * inbound IDLE polling per account (broker publish)
//!   * outbound SMTP dispatch (broker subscribe)
//!   * SQLite cursor / threading / bounce / attachment stores
//!     rooted at `NEXO_PLUGIN_EMAIL_DATA_DIR`
//!
//! Follow-up `81.19.b.tool-dispatch-subprocess` tracks porting the
//! 12 tool handlers to subprocess JSON-RPC dispatch when there's a
//! concrete consumer (e.g. mobile embedded client without daemon).

use nexo_microapp_sdk::plugin::{ToolDef as SdkToolDef, ToolInvocation, ToolInvocationError};
use serde_json::Value;

/// The list of tool defs advertised in the `initialize` reply.
/// Currently empty — see module docs.
pub fn email_tool_defs() -> Vec<SdkToolDef> {
    Vec::new()
}

/// Phase 81.19.b — every `tool.invoke` reaches here returns
/// `Unsupported`. The daemon's discovery walker spawns the
/// subprocess for inbound/outbound only; tool dispatch travels
/// through the in-process lib surface.
///
/// Follow-up `81.19.b.tool-dispatch-subprocess` will replace this
/// with a real matcher when there's a non-daemon consumer.
pub async fn dispatch_email_tool(
    invocation: ToolInvocation,
) -> Result<Value, ToolInvocationError> {
    Err(ToolInvocationError::NotFound(format!(
        "email subprocess plugin advertises zero tools — tool '{}' must \
         be invoked through the in-process `nexo_plugin_email::tool` \
         surface registered against the daemon's ToolRegistry",
        invocation.tool_name
    )))
}
