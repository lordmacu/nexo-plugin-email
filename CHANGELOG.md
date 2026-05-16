# Changelog

All notable changes to `nexo-plugin-email` are documented here.

## [0.5.0] — 2026-05-16

Multi-tenant release. Operators can declare N tenants in their
YAML, each owning isolated accounts + per-tenant state stores
(`<data_dir>/email/<tenant>/`). Legacy 0.4.x single-map YAML keeps
working via a back-compat shim that resolves to a one-element vec
with `instance: "default"`.

### Breaking

- **Multi-tenant config shape.** `[plugin.config_schema] shape:
  "object"` → `"array"`. `EmailPluginConfigFile.email` now holds
  `EmailPluginShape::{Single, Many}` (untagged enum); both shapes
  normalise via `into_vec()`.
- **`EmailPluginConfig` gains:**
    - `instance: Option<String>` — tenant label (None ⇒ "default").
      Distinct namespace from `EmailAccountConfig.instance` (the
      per-account label inside this tenant).
    - `allow_agents: Vec<String>` — agents permitted to route
      email tools to this tenant. Empty = accept any.
- **`configured_state` stores `Option<Vec<EmailPluginConfig>>`** so
  multi-tenant readers walk all tenants. Single-tenant back-compat
  populates a 1-element vec.
- **Dashboard surface** flipped from `single` / `file_presence` to
  `workspace_walk subdir = "email"` / `session_dir_files`
  candidates including the legacy `email_password.txt` and the
  Chrome-style `.nexo-paired` sentinel.

### Added

- **`instance_registry`** — `OnceLock<Arc<DashMap<String,
  Arc<EmailPlugin>>>>` mirroring browser / telegram / whatsapp
  plugins. Tenant declarations register an `Arc<EmailPlugin>` per
  entry.
- **`boot::apply_configure`** — JSON-RPC `plugin.configure` boot
  loop. Sanitises tenant labels (lowercase ASCII
  `[A-Za-z0-9_-]{1,64}`, rejects path-traversal / unicode / empty),
  resolves per-tenant `data_dir = <base>/email/<tenant>/`, diffs vs
  prior registry, registers via a caller-supplied factory closure
  (tests pass empty credential stores; daemon production wraps the
  real stores).
- **`admin RPC list_tenants`** — new verb returning per-tenant rows
  with `accounts_count`, `registered`, and `allow_agents`. Legacy
  `list_instances` (flat account list) retained for 0.4.x admin UI
  back-compat.
- **Manifest auto-discovery sections** updated: `config_schema.shape`
  bumped, `dashboard.layout` swapped, `auth_check.candidates`
  expanded, `instance` + `allow_agents` added to the JSON Schema.

### Tests

- `tests/configure_boot.rs` (8) — boot loop: register-two,
  diff-aware reload, dup rejection, invalid label rejection,
  label case-normalisation, single-shape back-compat preserves
  empty registry, default fallback, helper API.
- `tests/manifest_parse.rs` (4) — shape=array, workspace_walk
  subdir, session_dir_files candidates, broker allowlist.
- `tests/e2e_multi_instance.rs` (3) — JSON-RPC wire: 2-tenant
  array accepted, legacy single-map still works, unknown field
  rejected.
- Inline: 2 new admin tests in `auto_discovery::tests`.
- Total: 275/275 nextest green.

### Backward compatibility

- Legacy bare-map YAML keeps working via the Single shape.
- `list_instances` admin verb retained.
- Internal `EmailAccountConfig.instance` (account label) kept
  intact — only the new `EmailPluginConfig.instance` (tenant label)
  is added on top.

### Deferred follow-ups

- **Daemon-side subprocess flip.** Daemon currently spawns email
  in-process via the `email_arc.clone()` factory. Wave 2 will swap
  to `register_instance_subprocess_factories("email", ...)` so
  each tenant gets its own subprocess (real cross-tenant process
  isolation). This release ships the plugin-side contract; the
  daemon update is a separate `proyecto` PR.
- **Per-instance metrics labels.** Existing `email_*` counters
  expose `instance` (account-level). Adding a tenant dimension
  touches every event-site; deferred so observability stays
  incremental.
- **`seed_email_subprocess_env_for(&cfg)`.** Daemon helper is
  currently `#[cfg(test)]`-only for the legacy single-instance
  model. Wave 2 promotes it to production + adds per-tenant env
  stamping.

## [0.3.0] — 2026-05-15

### Added

- Manifest declares `[plugin.credentials_schema]` (Phase 93.8.a-daemon)
  with `enabled = true` + `accounts_shape = "array"`. Daemon's
  `SubprocessNexoPlugin::credential_store()` reads this section
  and constructs a `RemoteCredentialStore` round-tripping the
  four `plugin.credentials.*` JSON-RPCs.
- SDK `on_credentials_list` / `on_credentials_issue` /
  `on_credentials_resolve_bytes` / `on_credentials_reload`
  handlers registered in `main.rs`, all backed by
  `configured_state()`. List returns
  `EmailPluginConfig.accounts[*].instance`, issue verifies the
  instance exists, resolve_bytes returns the serde_json-encoded
  `EmailAccountConfig`.
- `EmailPluginConfig` + all sub-structs derive `Serialize` so the
  resolve_bytes handler can round-trip through serde_json.

### Tests

- `tests/credentials_path.rs` — 5 integration tests covering
  list / issue accept-reject paths / no-configured-state /
  resolve_bytes round-trip.

## [0.2.0] — 2026-05-15

### Breaking

- Plugin owns its config types. `nexo_config::types::plugins::{EmailPluginConfig, EmailPluginConfigFile, EmailAccountConfig, LoopPreventionCfg, EmailFolders, EmailFilters, EmailProvider, ImapEndpoint, SmtpEndpoint, TlsMode}` no longer re-imported; equivalents live in `nexo_plugin_email::config`. Field shapes byte-for-byte identical.
- `rust-version` bumped `1.75 → 1.80` so `std::sync::OnceLock<Arc<...>>` static init compiles without `once_cell::sync::Lazy`.

### Added

- Manifest declares `[plugin.config_schema]` (Phase 93.1) with `shape = "object"` (single-instance plugin — `cfg.plugins.email` is a map, not a sequence). JSON Schema covers every operator-visible knob.
- SDK `on_configure(...)` handler (Phase 93.4.a-sdk) receives operator YAML via `plugin.configure` JSON-RPC (Phase 93.2); caches `EmailPluginConfig` via the new `configured_state()` accessor.
- 5 new integration tests in `tests/configure_path.rs`.

### Backward compatibility

- Env-var fallback (`NEXO_PLUGIN_EMAIL_*` vars) keeps working when daemon doesn't deliver `plugin.configure`. Removed in 0.3.0 once Phase 93.5 closes the daemon-side typed-fields window.

## [0.1.3] — 2026-05-10

### Fixed

- **Pin rustls + tokio-rustls to `default-features = false,
  features = ["ring", ...]`** (Phase 27.2-follow-up.b). The
  default feature set on `rustls 0.23` and `tokio-rustls 0.26`
  pulls `aws_lc_rs` (and `prefer-post-quantum` which forwards
  to `aws_lc_rs`). `aws-lc-rs` ships its own bundled BoringSSL
  + jitterentropy C source that fails to cross-compile to
  `aarch64-linux-android` (Termux + the Flutter Android FFI
  target) with `'sys/types.h' file not found` — Bionic libc
  layout differs from POSIX in subtle ways the upstream code
  assumes.

  Pinning `ring` (pure-Rust + asm crypto, no C build chain)
  eliminates the issue entirely and aligns with the rest of
  the workspace which already runs ring via `sqlx`, `reqwest`,
  `lettre`, and `async-nats`.

  Behaviour change: none. Both providers implement the same
  TLS surface area; the runtime choice is invisible to plugin
  consumers.

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
