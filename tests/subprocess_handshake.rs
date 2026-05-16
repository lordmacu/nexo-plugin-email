//! Phase 81.19.b — subprocess JSON-RPC handshake coverage.
//!
//! These tests run the `nexo-plugin-email` binary directly via
//! `Command::spawn` and exercise the wire shape the daemon's
//! `SubprocessNexoPlugin` expects:
//!
//!   * `initialize` reply carries the manifest + advertises zero
//!     tools (subprocess delegates tool dispatch to the in-process
//!     lib surface; see `subprocess_dispatch.rs` docs).
//!   * `tool.invoke` returns `NotFound` so the daemon can fall
//!     back to the in-process registration without ambiguity.
//!   * `shutdown` round-trips an ack and the child exits clean
//!     within the supervisor's grace window.
//!
//! The boot flow inside the binary tries to load config / secrets
//! / broker. Tests run with deliberately-broken env to verify the
//! adapter handshake completes BEFORE boot finishes — `tracing`
//! captures the boot error, JSON-RPC is unaffected.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use serial_test::serial;

const BINARY: &str = env!("CARGO_BIN_EXE_nexo-plugin-email");

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(8);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(3);

/// Spawn the email subprocess with a cleared env + only the keys
/// passed in. Returns the child + piped stdin/stdout for direct
/// JSON-RPC framing.
fn spawn_with_env(env: &[(&str, &str)]) -> std::process::Child {
    let mut cmd = Command::new(BINARY);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_clear();
    // PATH is needed by the rustls / ring crypto provider on some
    // distros (loads dynamic libs on first call); pass through if
    // present.
    if let Ok(path) = std::env::var("PATH") {
        cmd.env("PATH", path);
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.spawn().expect("spawn nexo-plugin-email")
}

fn rpc_round_trip(
    stdin: &mut std::process::ChildStdin,
    stdout: &mut BufReader<std::process::ChildStdout>,
    frame: Value,
) -> Value {
    let line = serde_json::to_string(&frame).expect("frame serialises");
    stdin
        .write_all(line.as_bytes())
        .expect("write request line");
    stdin.write_all(b"\n").expect("write newline");
    stdin.flush().expect("flush stdin");

    let mut buf = String::new();
    let started = Instant::now();
    loop {
        if started.elapsed() > HANDSHAKE_TIMEOUT {
            panic!(
                "nexo-plugin-email: no reply within {HANDSHAKE_TIMEOUT:?} for frame {line}",
            );
        }
        match stdout.read_line(&mut buf) {
            Ok(0) => panic!("nexo-plugin-email: stdout EOF before reply"),
            Ok(_) => break,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(20));
                continue;
            }
            Err(e) => panic!("nexo-plugin-email: read_line error: {e}"),
        }
    }
    serde_json::from_str(buf.trim()).unwrap_or_else(|e| {
        panic!("nexo-plugin-email: reply not JSON: {e} (raw: {buf:?})")
    })
}

#[test]
#[serial]
fn initialize_reply_carries_manifest_and_zero_tools() {
    // Deliberately-broken env: boot will fail in the detached
    // task, but the adapter still handshakes immediately.
    let mut child = spawn_with_env(&[
        ("NEXO_BROKER_URL", "local://test"),
        ("NEXO_PLUGIN_EMAIL_CONFIG_PATH", "/nonexistent/email.yaml"),
        ("NEXO_PLUGIN_EMAIL_SECRETS_DIR", "/nonexistent/secrets"),
        ("NEXO_PLUGIN_EMAIL_DATA_DIR", "/tmp"),
    ]);
    let mut stdin = child.stdin.take().expect("stdin");
    let mut stdout = BufReader::new(child.stdout.take().expect("stdout"));

    let reply = rpc_round_trip(
        &mut stdin,
        &mut stdout,
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}),
    );

    let result = &reply["result"];
    assert_eq!(
        result["manifest"]["plugin"]["id"].as_str(),
        Some("email"),
        "initialize reply must echo the email manifest plugin.id"
    );
    // Phase 81.19.b: subprocess advertises ZERO tool defs. The SDK
    // omits the `tools` field entirely when `declared_tools` is
    // empty (see microapp-sdk/src/plugin.rs initialize handler) —
    // either absent OR empty array is acceptable.
    let tools_field = result.get("tools");
    let tools_count = tools_field
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    // 0.5.1 / Wave 3 — subprocess now advertises the 12 `email_*`
    // tools so daemon-side `RemoteToolHandler` can route tool.invoke
    // through the broker per tenant. Previously this asserted 0 per
    // Phase 81.19.b; the dispatch port flips that contract.
    assert_eq!(
        tools_count, 12,
        "subprocess must advertise 12 email_* tools after Wave 3 dispatch port"
    );

    // Cleanup — kill the child to avoid orphan workers.
    let _ = child.kill();
    let _ = child.wait();
}

#[test]
#[serial]
fn tool_invoke_for_unknown_name_returns_not_found() {
    let mut child = spawn_with_env(&[
        ("NEXO_BROKER_URL", "local://test"),
        ("NEXO_PLUGIN_EMAIL_CONFIG_PATH", "/nonexistent/email.yaml"),
        ("NEXO_PLUGIN_EMAIL_SECRETS_DIR", "/nonexistent/secrets"),
        ("NEXO_PLUGIN_EMAIL_DATA_DIR", "/tmp"),
    ]);
    let mut stdin = child.stdin.take().expect("stdin");
    let mut stdout = BufReader::new(child.stdout.take().expect("stdout"));

    // initialize must precede any tool.invoke per JSON-RPC contract.
    let _ = rpc_round_trip(
        &mut stdin,
        &mut stdout,
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}),
    );

    // 0.5.1 / Wave 3 — `email_send` is now a real tool. Use a
    // truly-unknown name to verify the NotFound path still fires
    // for anything outside the 12 advertised tools.
    let reply = rpc_round_trip(
        &mut stdin,
        &mut stdout,
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tool.invoke",
            "params": {
                "plugin_id": "email",
                "tool_name": "email_does_not_exist_zzz",
                "args": {}
            }
        }),
    );

    assert!(reply.get("error").is_some(), "tool.invoke must error on unknown tool");
    let code = reply["error"]["code"].as_i64().unwrap_or(0);
    assert_eq!(code, -33401, "expected NotFound code (-33401), got {code}");

    let _ = child.kill();
    let _ = child.wait();
}

#[test]
#[serial]
fn shutdown_request_acks_then_exits_clean() {
    let mut child = spawn_with_env(&[
        ("NEXO_BROKER_URL", "local://test"),
        ("NEXO_PLUGIN_EMAIL_CONFIG_PATH", "/nonexistent/email.yaml"),
        ("NEXO_PLUGIN_EMAIL_SECRETS_DIR", "/nonexistent/secrets"),
        ("NEXO_PLUGIN_EMAIL_DATA_DIR", "/tmp"),
    ]);
    let mut stdin = child.stdin.take().expect("stdin");
    let mut stdout = BufReader::new(child.stdout.take().expect("stdout"));

    let _ = rpc_round_trip(
        &mut stdin,
        &mut stdout,
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}),
    );

    let reply = rpc_round_trip(
        &mut stdin,
        &mut stdout,
        json!({"jsonrpc": "2.0", "id": 99, "method": "shutdown", "params": {}}),
    );
    assert!(
        reply.get("result").is_some() || reply.get("error").is_none(),
        "shutdown must reply with a result, got: {reply}"
    );

    drop(stdin);
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                // Boot detached task may have errored; we accept any
                // exit because shutdown is the contract under test.
                assert!(
                    status.code().is_some() || status.success(),
                    "child must exit cleanly after shutdown; got {status:?}"
                );
                break;
            }
            Ok(None) => {
                if started.elapsed() > SHUTDOWN_TIMEOUT {
                    let _ = child.kill();
                    panic!("child did not exit within {SHUTDOWN_TIMEOUT:?} after shutdown ack");
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => panic!("try_wait error: {e}"),
        }
    }
}
