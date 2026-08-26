//! Phase 34 (#295): Live e2e integration tests for `parsec mcp serve`.
//!
//! Spawns the real parsec binary in a sandboxed temporary git repository
//! and exchanges JSON-RPC 2.0 messages over stdin/stdout. This validates
//! the full subprocess boundary — transport, dispatcher, and tool handlers —
//! beyond the in-process fixture tests in `src/mcp/mod.rs`.
//!
//! All tests use read-only tools or `dry_run=true`; no network calls are made.

use std::io::Write;
use std::process::{Command, Stdio};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Sandbox helpers
// ---------------------------------------------------------------------------

/// Minimal git repo with one empty commit on `main` (no remote).
fn sandbox_repo() -> TempDir {
    let dir = TempDir::new().expect("sandbox TempDir");
    let run = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .current_dir(dir.path())
            .output()
            .expect("git");
    };
    run(&["init"]);
    run(&["config", "user.name", "Sandbox"]);
    run(&["config", "user.email", "sandbox@example.invalid"]);
    run(&["checkout", "-b", "main"]);
    run(&["commit", "--allow-empty", "-m", "sandbox init"]);
    dir
}

/// Spawn `parsec mcp serve`, write requests to stdin, close stdin, wait for
/// exit, and return newline-delimited stdout as parsed JSON values.
fn run_serve_session(sandbox: &TempDir, requests: &[serde_json::Value]) -> Vec<serde_json::Value> {
    let bin = assert_cmd::cargo::cargo_bin("parsec");
    let mut child = Command::new(&bin)
        .args(["mcp", "serve"])
        .current_dir(sandbox.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null()) // audit events go to stderr; suppress in tests
        .spawn()
        .unwrap_or_else(|e| panic!("spawn {}: {e}", bin.display()));

    {
        let stdin = child.stdin.as_mut().expect("stdin");
        for req in requests {
            writeln!(stdin, "{req}").expect("write");
        }
        // EOF closes the server's BufRead loop
    }

    let out = child.wait_with_output().expect("wait_with_output");
    assert!(out.status.success(), "serve exited {:?}", out.status);

    String::from_utf8(out.stdout)
        .expect("UTF-8")
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("non-JSON: {l:?}\n{e}")))
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Full MCP session lifecycle: initialize → notification (no response) → ping → shutdown.
#[test]
fn mcp_serve_protocol_lifecycle() {
    let sb = sandbox_repo();
    let res = run_serve_session(
        &sb,
        &[
            serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"live-e2e","version":"0.0.0"}}}),
            serde_json::json!({"jsonrpc":"2.0","method":"notifications/initialized","params":{}}),
            serde_json::json!({"jsonrpc":"2.0","id":"ping","method":"ping"}),
            serde_json::json!({"jsonrpc":"2.0","id":"shutdown","method":"shutdown"}),
        ],
    );
    // notification yields no response → 3 responses total
    assert_eq!(res[0]["id"], 1);
    assert_eq!(res[0]["result"]["protocolVersion"], "2025-03-26");
    assert_eq!(res[0]["result"]["serverInfo"]["name"], "git-parsec");
    assert_eq!(
        res[0]["result"]["capabilities"]["gitParsec"]["githubScopes"],
        serde_json::json!(["pull_request:read", "checks:read", "pull_request:write"])
    );
    assert_eq!(res[1]["id"], "ping");
    assert!(res[1]["result"].is_object());
    assert_eq!(res[2]["id"], "shutdown");
    assert_eq!(res[2]["result"], serde_json::Value::Null);
}

/// `tools/list` must expose all 10 registered parsec MCP tools.
#[test]
fn mcp_serve_tools_list_full_registry() {
    let sb = sandbox_repo();
    let res = run_serve_session(
        &sb,
        &[serde_json::json!({"jsonrpc":"2.0","id":"tl","method":"tools/list"})],
    );
    let tools = res[0]["result"]["tools"].as_array().expect("tools array");
    assert!(tools.len() >= 10, "expected ≥10 tools, got {}", tools.len());
    for name in [
        "worktree_list",
        "worktree_start",
        "worktree_status",
        "worktree_ship",
        "smartlog",
        "ci_status",
        "pr_status",
        "health_check",
        "reviews",
        "sync",
    ] {
        assert!(
            tools.iter().any(|t| t["name"] == name),
            "missing tool '{name}'"
        );
    }
}

/// `worktree_list` (no overlay) runs locally and returns the sandbox worktree.
#[test]
fn mcp_serve_worktree_list_and_health_check() {
    let sb = sandbox_repo();
    let res = run_serve_session(
        &sb,
        &[
            serde_json::json!({"jsonrpc":"2.0","id":"wl","method":"tools/call","params":{"name":"worktree_list","arguments":{}}}),
            serde_json::json!({"jsonrpc":"2.0","id":"hc","method":"tools/call","params":{"name":"health_check","arguments":{}}}),
        ],
    );

    // worktree_list
    assert_eq!(res[0]["result"]["isError"], false);
    let wl: serde_json::Value = serde_json::from_str(
        res[0]["result"]["content"][0]["text"]
            .as_str()
            .expect("text"),
    )
    .expect("wl json");
    assert!(wl["worktrees"].is_array());
    assert_eq!(wl["pr_overlay"], false);
    assert_eq!(wl["ci_overlay"], false);

    // health_check
    assert_eq!(res[1]["result"]["isError"], false);
    let hc: serde_json::Value = serde_json::from_str(
        res[1]["result"]["content"][0]["text"]
            .as_str()
            .expect("text"),
    )
    .expect("hc json");
    assert!(hc["records"].is_array());
}

/// GitHub-backed tools return structured auth errors when no token is present.
#[test]
fn mcp_serve_github_tools_require_auth() {
    let sb = sandbox_repo();
    let res = run_serve_session(
        &sb,
        &[
            serde_json::json!({"jsonrpc":"2.0","id":"pr","method":"tools/call","params":{"name":"pr_status","arguments":{"ticket":"T-1"}}}),
            serde_json::json!({"jsonrpc":"2.0","id":"ci","method":"tools/call","params":{"name":"ci_status","arguments":{"ticket":"T-1"}}}),
        ],
    );
    for (i, scope) in [("pr", "pull_request:read"), ("ci", "checks:read")] {
        let idx = if i == "pr" { 0 } else { 1 };
        assert_eq!(res[idx]["result"]["isError"], true, "{i} should fail");
        let text = res[idx]["result"]["content"][0]["text"]
            .as_str()
            .expect("text");
        assert!(
            text.contains("AUTH_REQUIRED"),
            "{i}: expected AUTH_REQUIRED"
        );
        assert!(text.contains(scope), "{i}: expected scope {scope}");
    }
}

/// Mutating tools enforce the dry_run / confirmation gate at the subprocess level.
#[test]
fn mcp_serve_mutation_gates() {
    let sb = sandbox_repo();
    let res = run_serve_session(
        &sb,
        &[
            // No dry_run, no confirm → CONFIRMATION_REQUIRED
            serde_json::json!({"jsonrpc":"2.0","id":"no-confirm","method":"tools/call","params":{"name":"worktree_start","arguments":{"ticket":"LIVE-1"}}}),
            // dry_run=true → preview without side effects
            serde_json::json!({"jsonrpc":"2.0","id":"dry-run","method":"tools/call","params":{"name":"worktree_start","arguments":{"ticket":"LIVE-2","dry_run":true}}}),
        ],
    );
    // no-confirm
    assert_eq!(res[0]["result"]["isError"], true);
    let text0 = res[0]["result"]["content"][0]["text"]
        .as_str()
        .expect("text");
    assert!(text0.contains("CONFIRMATION_REQUIRED"), "got: {text0}");

    // dry-run preview
    assert_eq!(res[1]["result"]["isError"], false, "dry_run should succeed");
    let text1 = res[1]["result"]["content"][0]["text"]
        .as_str()
        .expect("text");
    assert!(
        text1.contains("LIVE-2"),
        "dry_run should reference ticket: {text1}"
    );
}
