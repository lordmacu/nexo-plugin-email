# nexo-plugin-email — operator guide

Standalone repo for the email subprocess plugin extracted from
`proyecto/crates/plugins/email/` in Phase 81.19.b.

## Dual-mode reminder

This repo ships **lib + bin**, mirroring the telegram and whatsapp
extracts:

- `src/lib.rs` re-exports `EmailPlugin`, `EmailPluginConfig`,
  `EmailToolContext`, `register_email_tools_filtered`,
  `EMAIL_TOOL_NAMES`, etc. so the daemon's MCP autonomous worker
  and future Phase 90 (Android embedded) consumers can drop the
  subprocess loop and use the plugin in-process.
- `src/main.rs` is the only subprocess-specific code. It wraps
  `EmailPlugin` in `PluginAdapter`, runs the JSON-RPC loop over
  stdio, and seeds the plugin from the env vars the daemon set
  before spawn.

## Single-process / multi-account

Unlike telegram (one bot per process, multi-instance subprocess)
and whatsapp (one device per process, multi-instance subprocess),
email is **single-process / multi-account-internal**: one boot
covers every account declared in `EmailPluginConfig.accounts`.
The inbound / outbound topics carry the per-account alias directly
(`plugin.inbound.email.<alias>`).

This means the daemon spawns the email subprocess **at most once**
per node, regardless of how many accounts the operator configures.

## Tool dispatch is in-process (81.19.b)

The subprocess advertises **zero tool defs** in its `initialize`
reply. The 12 email tools (`email_send`, `email_reply`, …) share
heavy in-process state — IMAP IDLE workers, SMTP queue, SQLite
stores — that doesn't translate cleanly to JSON-RPC dispatch.

Tool dispatch stays in the in-process lib surface:
`nexo_plugin_email::register_email_tools_filtered(registry, ctx, allow)`
runs in the daemon's address space against its `ToolRegistry`.

Follow-up `81.19.b.tool-dispatch-subprocess` (logged in
`proyecto/FOLLOWUPS.md`) tracks porting the 12 handlers to
subprocess JSON-RPC dispatch when there's a non-daemon consumer.

## Logging

`tracing_subscriber` is pinned to `stderr` in `src/main.rs`.
**Never** add a writer that touches stdout — the daemon-side
`SubprocessNexoPlugin` reads JSON-RPC frames from the child's
stdout and any non-JSON line corrupts the protocol.

## Path-deps interim

`Cargo.toml` path-deps point at `../proyecto/crates/...`. They
ship today before the rest of the proyecto crates publish to
crates.io. Operators outside the monorepo build against published
versions; `Cargo.lock` is committed to pin the resolved versions
for reproducibility.

When the workspace deps publish (tracked in
`project_publishable_helper_crates` user memory under "Tier B"),
swap the `path = "..."` lines for explicit version pins and
delete the local checkout dependency.

## When to bump version

| Change kind                      | Bump |
|----------------------------------|------|
| New tool added to lib surface    | minor (0.1.x → 0.2.0 if breaking SDK)|
| New env var in subprocess        | minor                                |
| IMAP/SMTP wire fix               | patch                                |
| Manifest field added             | minor (operators must re-deploy)     |

Cut a tag on every publish so `cargo install --git ... --tag v0.1.x`
keeps working for operators that pin to a release.
