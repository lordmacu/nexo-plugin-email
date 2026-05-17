//! Subprocess JSON-RPC dispatch for email tools.
//!
//! 0.5.1 / Wave 3 — email subprocess now serves the 12 `email_*`
//! tools over JSON-RPC, mirroring the browser plugin's pattern.
//! Per-tool handler bodies in `tool/*.rs` are re-used unchanged —
//! we just build `EmailToolContext` on demand from the live
//! `Arc<EmailPlugin>` exposed via `runtime_handle::runtime_handle()`,
//! then call `Tool::new(ctx).run(args).await`.
//!
//! What this gives us:
//!   * Multi-tenant subprocess (Wave 2): each tenant's subprocess
//!     hosts its own `EmailToolContext`; daemon-side
//!     `RemoteToolHandler` routes `tool.invoke` calls to the
//!     correct subprocess via the `instance_registry`.
//!   * Drops the daemon-side `register_email_tools_filtered`
//!     in-process Arc factory (Wave 3 W3-E).
//!
//! What's still pending (separate follow-ups):
//!   * Per-tenant metrics labels in the plugin's `metrics.rs`.

use nexo_microapp_sdk::plugin::{ToolDef as SdkToolDef, ToolInvocation, ToolInvocationError};
use serde_json::Value;

use crate::runtime_handle::runtime_handle;
use crate::tool::archive::EmailArchiveTool;
use crate::tool::attachment_get::EmailAttachmentGetTool;
use crate::tool::bounces_summary::EmailBouncesSummaryTool;
use crate::tool::get::EmailGetTool;
use crate::tool::health::EmailHealthTool;
use crate::tool::instances_list::EmailInstancesListTool;
use crate::tool::label::EmailLabelTool;
use crate::tool::move_to::EmailMoveToTool;
use crate::tool::reply::EmailReplyTool;
use crate::tool::search::EmailSearchTool;
use crate::tool::send::EmailSendTool;
use crate::tool::thread::EmailThreadTool;

/// Convert the daemon-side `nexo_core` `ToolDef` to the SDK's
/// `ToolDef` shape. Renames `parameters` → `input_schema`; the
/// rest is field-identical.
fn convert_tool_def(t: nexo_llm::ToolDef) -> SdkToolDef {
    SdkToolDef {
        name: t.name,
        description: t.description,
        input_schema: t.parameters,
    }
}

/// Tool defs advertised in the `initialize` reply.
pub fn email_tool_defs() -> Vec<SdkToolDef> {
    vec![
        EmailSendTool::tool_def(),
        EmailReplyTool::tool_def(),
        EmailArchiveTool::tool_def(),
        EmailMoveToTool::tool_def(),
        EmailLabelTool::tool_def(),
        EmailSearchTool::tool_def(),
        EmailGetTool::tool_def(),
        EmailThreadTool::tool_def(),
        EmailBouncesSummaryTool::tool_def(),
        EmailAttachmentGetTool::tool_def(),
        EmailHealthTool::tool_def(),
        EmailInstancesListTool::tool_def(),
    ]
    .into_iter()
    .map(convert_tool_def)
    .collect()
}

/// Subprocess `tool.invoke` dispatcher. Looks up the live
/// `Arc<EmailPlugin>`, builds an `EmailToolContext`, and routes
/// to the per-tool handler.
pub async fn dispatch_email_tool(invocation: ToolInvocation) -> Result<Value, ToolInvocationError> {
    // Match name FIRST so unknown tools return NotFound (-33401)
    // even when the plugin hasn't booted yet — preserves the wire
    // contract.
    let name = invocation.tool_name.clone();
    let known = matches!(
        name.as_str(),
        "email_send"
            | "email_reply"
            | "email_archive"
            | "email_move_to"
            | "email_label"
            | "email_search"
            | "email_get"
            | "email_thread"
            | "email_bounces_summary"
            | "email_attachment_get"
            | "email_health"
            | "email_instances_list"
    );
    if !known {
        return Err(ToolInvocationError::NotFound(format!(
            "unknown email tool: `{name}`"
        )));
    }

    let plugin = {
        let guard = runtime_handle().read().await;
        guard.as_ref().cloned().ok_or_else(|| {
            ToolInvocationError::Unavailable(
                "email plugin not booted yet — broker / IMAP / SMTP \
                 wiring still in flight; retry shortly"
                    .into(),
            )
        })?
    };
    let ctx = plugin.build_tool_context().await.ok_or_else(|| {
        ToolInvocationError::Unavailable(
            "email plugin booted without outbound dispatcher — accounts \
             may be empty or SMTP unavailable"
                .into(),
        )
    })?;

    let args = invocation.args;
    let reply = match name.as_str() {
        "email_send" => EmailSendTool::new(ctx).run(args).await,
        "email_reply" => EmailReplyTool::new(ctx).run(args).await,
        "email_archive" => EmailArchiveTool::new(ctx).run(args).await,
        "email_move_to" => EmailMoveToTool::new(ctx).run(args).await,
        "email_label" => EmailLabelTool::new(ctx).run(args).await,
        "email_search" => EmailSearchTool::new(ctx).run(args).await,
        "email_get" => EmailGetTool::new(ctx).run(args).await,
        "email_thread" => EmailThreadTool::new(ctx).run(args).await,
        "email_bounces_summary" => EmailBouncesSummaryTool::new(ctx).run(args).await,
        "email_attachment_get" => EmailAttachmentGetTool::new(ctx).run(args).await,
        "email_health" => EmailHealthTool::new(ctx).run(args).await,
        "email_instances_list" => EmailInstancesListTool::new(ctx).run(args).await,
        // Unreachable — name validated above.
        other => {
            return Err(ToolInvocationError::NotFound(format!(
                "unknown email tool: `{other}`"
            )));
        }
    };
    Ok(reply)
}
