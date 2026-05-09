# Changelog

All notable changes to `nexo-plugin-email` are documented here.

## [0.1.2] — 2026-05-09

### Added
- **Standalone repo** extracted from `nexo-rs/proyecto/crates/plugins/email/`
  per Phase 81.19.b. Ships dual-mode: `[[bin]] nexo-plugin-email`
  for subprocess discovery + `lib` for in-process / embedded
  consumers (MCP autonomous worker, future Phase 90 mobile build).
- `src/env_config.rs` — `email_config_from_env()` re-loads
  `email.yaml` and `google-auth.yaml` per-spawn from paths
  supplied by the daemon's `seed_email_subprocess_env_for`.
- `src/subprocess_dispatch.rs` — JSON-RPC dispatch shell. The
  initialize reply currently advertises **zero tool defs**;
  tool dispatch stays in-process via `register_email_tools_filtered`.
- `src/main.rs` — subprocess entrypoint with eager-boot detached
  task: JSON-RPC adapter handshakes immediately, IMAP IDLE
  workers + SMTP outbound subscribe come up in the background.
- `tests/subprocess_handshake.rs` — three serial tests covering
  initialize wire shape (`tools` field absent / empty), tool.invoke
  fall-through to `NotFound`, and clean shutdown.

### Changed
- Manifest version bumped from `0.1.1` → `0.1.2` to mark the
  extract.
- DORMANT marker removed from `nexo-plugin.toml` — operators can
  now place the manifest in `plugins.discovery.search_paths`
  once Phase 81.12.e flips the daemon's wire (see main repo).
- `tracing` writer pinned to `stderr` in subprocess mode
  (preserves stdout for JSON-RPC framing).

### Removed
- The legacy `register_arc` block in
  `proyecto/src/main.rs:2632-2655` is dropped in the same wave
  (separate commit in the main repo) so discovery is the only
  channel-plugin path.

## [0.1.1] — 2026-04-XX (in-tree)

Last release inside the monorepo; see `nexo-rs` git log up to
the extract commit for sub-phase notes (48.x — IMAP/SMTP
implementation, 49.x — DSN/threading hardening, 81.12.d —
factory_registry plumbing).
