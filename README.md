# nexo-plugin-email

Multi-account IMAP / SMTP channel plugin for the [Nexo agent
framework][nexo]. Runs as a subprocess of the daemon, exchanging
JSON-RPC frames over stdio and routing inbound mail / outbound
commands through NATS broker topics.

Out-of-tree per **Phase 81.19.b**: extracted from
`proyecto/crates/plugins/email/` so the plugin can ship and
upgrade independently of the framework, and so a future embedded
build (Phase 90 — Android) can pull `EmailPlugin` straight out of
the lib surface without dragging the subprocess loop along.

[nexo]: https://github.com/lordmacu/nexo-rs

## What ships

- **IMAP IDLE inbound** with a 60 s polling fallback per account.
- **SMTP outbound** under a CircuitBreaker, with bounce + DSN
  parsing routed to `email.bounce.<instance>`.
- **MIME parse + multipart build** (`mail-parser` + `mail-builder`).
- **12 agent-callable tools** (`email_send`, `email_reply`,
  `email_archive`, `email_move_to`, `email_label`, `email_search`,
  `email_get`, `email_thread`, `email_bounces_summary`,
  `email_attachment_get`, `email_health`, `email_instances_list`).
- **Threading** via `Message-ID` / `In-Reply-To` / `References`
  (UUIDv5 session ids).
- **Loop-prevention** against auto-replies / list mail / self-bounces.
- **SQLite stores** (sqlx) for cursor / threading / bounce /
  attachment-refs, rooted at `NEXO_PLUGIN_EMAIL_DATA_DIR`.

## Layout

```
nexo-rs-plugin-email/
├── Cargo.toml                      # lib + [[bin]], path-deps interim
├── nexo-plugin.toml                # manifest (id="email")
├── src/
│   ├── lib.rs                      # re-exports for embedded consumers
│   ├── main.rs                     # subprocess entrypoint (PluginAdapter loop)
│   ├── plugin.rs                   # EmailPlugin: IMAP/SMTP supervisor
│   ├── env_config.rs               # email_config_from_env()
│   ├── subprocess_dispatch.rs      # tool dispatch (currently empty — see below)
│   ├── inbound.rs                  # IDLE workers, per-account state
│   ├── outbound.rs                 # SMTP dispatcher
│   ├── mime_build.rs / mime_parse.rs
│   ├── threading.rs / dsn.rs / loop_prevent.rs
│   ├── attachment_store.rs / bounce_store.rs / cursor.rs
│   ├── spf_dkim.rs                 # alignment checks
│   └── tool/                       # 12 tool handlers (in-process)
└── tests/
    ├── subprocess_handshake.rs     # JSON-RPC contract
    └── pipeline_in_process.rs      # end-to-end inbound→tool→outbound
```

## Build

```bash
cargo build --release
```

`Cargo.lock` is committed — binary repo convention, reproducible
builds from `git checkout v0.1.2 && cargo install --path .`.

## Subprocess vs lib mode

This crate ships **dual-mode**:

| Mode       | Used for                                           | Entrypoint                       |
|------------|----------------------------------------------------|----------------------------------|
| Subprocess | Discovery walker spawn — daemon talks JSON-RPC     | `[[bin]] nexo-plugin-email`      |
| Lib        | MCP autonomous worker, embedded consumers, tests   | `nexo_plugin_email::EmailPlugin` |

The subprocess advertises **zero tool defs** in its `initialize`
reply (`subprocess_dispatch::email_tool_defs` returns empty). Tool
dispatch happens through the in-process lib surface
(`register_email_tools_filtered`) registered against the daemon's
`ToolRegistry`. The subprocess owns IMAP IDLE polling + SMTP
outbound + the SQLite stores; tool invocations stay in-process.

Follow-up `81.19.b.tool-dispatch-subprocess` tracks porting the 12
tool handlers to subprocess JSON-RPC dispatch when there's a
non-daemon consumer (mobile embedded client without daemon).

## Subprocess env contract

The daemon's `seed_email_subprocess_env_for(cfg, broker_url, secrets_dir, data_dir)` sets:

| Var                                  | Required | Meaning                                   |
|--------------------------------------|:--------:|-------------------------------------------|
| `NEXO_BROKER_URL`                    | yes      | NATS or `local://…`                       |
| `NEXO_PLUGIN_EMAIL_CONFIG_PATH`      | yes      | absolute path to `email.yaml`             |
| `NEXO_PLUGIN_EMAIL_SECRETS_DIR`      | yes      | base `secrets/` directory (mode 0o600)    |
| `NEXO_PLUGIN_EMAIL_DATA_DIR`         | yes      | SQLite root (writable)                    |
| `NEXO_PLUGIN_EMAIL_GOOGLE_AUTH_PATH` | no       | `google-auth.yaml` for Gmail OAuth        |

Empty `NEXO_PLUGIN_EMAIL_GOOGLE_AUTH_PATH` ⇒ Gmail OAuth refresh is
disabled but classic IMAP/SMTP auth still works.

## Logs

`tracing` is wired to **stderr** in subprocess mode — stdout is
reserved for JSON-RPC framing. Set `RUST_LOG=nexo_plugin_email=debug`
to see per-account IDLE state transitions.

## License

MIT or Apache-2.0, at your option.
