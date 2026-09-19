//! Phase 34 (#295) + Phase 35 (#294) + Phase 38 (#295) + Phase 41 (#294/#295): Live e2e integration tests for `parsec mcp serve`.
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
    run_serve_session_with_env(sandbox, requests, &[])
}

/// Like [`run_serve_session`] but also injects `(key, value)` pairs into the
/// subprocess environment before spawning. Use this to test env-var-based
/// config paths (e.g., `PARSEC_GITHUB_TOKEN`).
fn run_serve_session_with_env(
    sandbox: &TempDir,
    requests: &[serde_json::Value],
    env_vars: &[(&str, &str)],
) -> Vec<serde_json::Value> {
    let bin = assert_cmd::cargo::cargo_bin("parsec");
    let mut cmd = Command::new(&bin);
    cmd.args(["mcp", "serve"])
        .current_dir(sandbox.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null()); // audit events go to stderr; suppress in tests
    for (key, val) in env_vars {
        cmd.env(key, val);
    }
    let mut child = cmd
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

// ---------------------------------------------------------------------------
// Phase 35 (#294): PARSEC_GITHUB_TOKEN env var → McpContext
// ---------------------------------------------------------------------------

/// When `PARSEC_GITHUB_TOKEN` is set in the subprocess environment, tools that
/// previously returned `AUTH_REQUIRED` must proceed past the token gate.
///
/// The sandbox repo has no remote, so `pr_status` will fail at the `gh` call
/// level (tool_error) rather than at the auth gate (AUTH_REQUIRED). This proves
/// the env var is picked up and forwarded into `McpContext.github_token`.
#[test]
fn mcp_serve_env_token_bypasses_auth_required() {
    let sb = sandbox_repo();

    // Without env var: pr_status should return AUTH_REQUIRED.
    let res_no_token = run_serve_session(
        &sb,
        &[serde_json::json!({
            "jsonrpc": "2.0",
            "id": "no-token",
            "method": "tools/call",
            "params": {
                "name": "pr_status",
                "arguments": {"ticket": "TEST-1"}
            }
        })],
    );
    assert_eq!(res_no_token[0]["result"]["isError"], true);
    let text_no_token = res_no_token[0]["result"]["content"][0]["text"]
        .as_str()
        .expect("text");
    assert!(
        text_no_token.contains("AUTH_REQUIRED"),
        "without token, expected AUTH_REQUIRED; got: {text_no_token}"
    );

    // With env var: pr_status should NOT return AUTH_REQUIRED; it proceeds to
    // the tool handler which fails at the gh CLI level (no remote configured).
    let res_with_token = run_serve_session_with_env(
        &sb,
        &[serde_json::json!({
            "jsonrpc": "2.0",
            "id": "with-token",
            "method": "tools/call",
            "params": {
                "name": "pr_status",
                "arguments": {"ticket": "TEST-1"}
            }
        })],
        &[("PARSEC_GITHUB_TOKEN", "ghp_fake_token_for_e2e_test")],
    );
    assert_eq!(res_with_token[0]["result"]["isError"], true);
    let text_with_token = res_with_token[0]["result"]["content"][0]["text"]
        .as_str()
        .expect("text");
    assert!(
        !text_with_token.contains("AUTH_REQUIRED"),
        "with PARSEC_GITHUB_TOKEN set, AUTH_REQUIRED must not appear; got: {text_with_token}"
    );
}

// ---------------------------------------------------------------------------
// Phase 38 (#295): mcp.toml config-file auth source + env-var precedence e2e
//
// Validates at the *subprocess binary boundary* that:
//   1. A token in `mcp.toml` (via PARSEC_MCP_CONFIG) bypasses AUTH_REQUIRED.
//   2. PARSEC_GITHUB_TOKEN wins over mcp.toml when both are present; in that
//      case the config-file scope restrictions are also superseded so a tool
//      that would have hit INSUFFICIENT_SCOPE under the config-file scopes
//      instead reaches CONFIRMATION_REQUIRED (the mutation gate).
// ---------------------------------------------------------------------------

/// Write `contents` to a named temp file and return the `TempDir` that keeps
/// the file alive for the duration of the test, along with the file path.
fn write_temp_toml(dir: &tempfile::TempDir, filename: &str, contents: &str) -> std::path::PathBuf {
    let path = dir.path().join(filename);
    std::fs::write(&path, contents).expect("write temp toml");
    path
}

/// A token supplied only through `mcp.toml` (via `PARSEC_MCP_CONFIG`) must be
/// loaded by the server subprocess and forwarded to `McpContext.github_token`,
/// which causes tools that previously returned `AUTH_REQUIRED` to proceed past
/// the auth gate (they fail at the `gh` CLI level instead).
#[test]
fn mcp_serve_config_file_token_bypasses_auth_required() {
    let sb = sandbox_repo();
    // Config dir is separate from the git sandbox so we can write freely.
    let cfg_dir = tempfile::TempDir::new().expect("cfg TempDir");
    let cfg_path = write_temp_toml(
        &cfg_dir,
        "mcp.toml",
        "[auth]\ntoken = \"ghp_config_fake_e2e\"\n",
    );

    // Without config token (no env var, no PARSEC_MCP_CONFIG): AUTH_REQUIRED.
    let res_no_cfg = run_serve_session(
        &sb,
        &[serde_json::json!({
            "jsonrpc": "2.0",
            "id": "baseline",
            "method": "tools/call",
            "params": { "name": "pr_status", "arguments": { "ticket": "PHASE38-0" } }
        })],
    );
    assert_eq!(res_no_cfg[0]["result"]["isError"], true);
    let text_baseline = res_no_cfg[0]["result"]["content"][0]["text"]
        .as_str()
        .expect("text");
    assert!(
        text_baseline.contains("AUTH_REQUIRED"),
        "baseline without config: expected AUTH_REQUIRED; got: {text_baseline}"
    );

    // With PARSEC_MCP_CONFIG pointing at the temp mcp.toml: AUTH_REQUIRED must
    // not appear — server picked up the config-file token and passed it through.
    let res_with_cfg = run_serve_session_with_env(
        &sb,
        &[serde_json::json!({
            "jsonrpc": "2.0",
            "id": "cfg-token",
            "method": "tools/call",
            "params": { "name": "pr_status", "arguments": { "ticket": "PHASE38-1" } }
        })],
        &[("PARSEC_MCP_CONFIG", cfg_path.to_str().unwrap())],
    );
    assert_eq!(res_with_cfg[0]["result"]["isError"], true);
    let text_with_cfg = res_with_cfg[0]["result"]["content"][0]["text"]
        .as_str()
        .expect("text");
    assert!(
        !text_with_cfg.contains("AUTH_REQUIRED"),
        "with config-file token, AUTH_REQUIRED must not appear; got: {text_with_cfg}"
    );
}

/// When both `PARSEC_GITHUB_TOKEN` (env var) and `mcp.toml` (via
/// `PARSEC_MCP_CONFIG`) are present, the env var must win.  The env-var path
/// does **not** load scope restrictions from the config file, so a tool that
/// would hit `INSUFFICIENT_SCOPE` under the config-file scopes must instead
/// reach `CONFIRMATION_REQUIRED` — the mutation gate — proving the env var
/// superseded the config-file token *and* its scope list.
#[test]
fn mcp_serve_env_var_wins_over_config_file_scope_restriction() {
    let sb = sandbox_repo();
    let cfg_dir = tempfile::TempDir::new().expect("cfg TempDir");
    // Config file grants only pull_request:read; worktree_ship needs
    // pull_request:write, so with config-only auth it would get INSUFFICIENT_SCOPE.
    let cfg_path = write_temp_toml(
        &cfg_dir,
        "mcp.toml",
        "[auth]\ntoken = \"ghp_read_only_config\"\nscopes = [\"pull_request:read\"]\n",
    );

    // Verify baseline with config-file only: INSUFFICIENT_SCOPE for worktree_ship.
    let res_cfg_only = run_serve_session_with_env(
        &sb,
        &[serde_json::json!({
            "jsonrpc": "2.0",
            "id": "cfg-only",
            "method": "tools/call",
            "params": {
                "name": "worktree_ship",
                "arguments": { "ticket": "PHASE38-A", "confirm": true }
            }
        })],
        &[("PARSEC_MCP_CONFIG", cfg_path.to_str().unwrap())],
    );
    assert_eq!(res_cfg_only[0]["result"]["isError"], true);
    let text_cfg_only = res_cfg_only[0]["result"]["content"][0]["text"]
        .as_str()
        .expect("text");
    assert!(
        text_cfg_only.contains("INSUFFICIENT_SCOPE"),
        "config-file pull_request:read should block worktree_ship; got: {text_cfg_only}"
    );

    // Now add PARSEC_GITHUB_TOKEN: env var takes priority, discarding the
    // config-file token AND its scope restrictions.  worktree_ship proceeds
    // past the scope gate and hits CONFIRMATION_REQUIRED (no dry_run / no
    // explicit confirm flag combination that passes the mutation gate here),
    // which is a different error than INSUFFICIENT_SCOPE.
    let res_env_wins = run_serve_session_with_env(
        &sb,
        &[serde_json::json!({
            "jsonrpc": "2.0",
            "id": "env-wins",
            "method": "tools/call",
            "params": {
                "name": "worktree_ship",
                "arguments": { "ticket": "PHASE38-B" }
            }
        })],
        &[
            ("PARSEC_GITHUB_TOKEN", "ghp_env_full_access"),
            ("PARSEC_MCP_CONFIG", cfg_path.to_str().unwrap()),
        ],
    );
    assert_eq!(res_env_wins[0]["result"]["isError"], true);
    let text_env_wins = res_env_wins[0]["result"]["content"][0]["text"]
        .as_str()
        .expect("text");
    // With env var in play, scope gate passes → must reach CONFIRMATION_REQUIRED,
    // not INSUFFICIENT_SCOPE.
    assert!(
        !text_env_wins.contains("INSUFFICIENT_SCOPE"),
        "env var must supersede config-file scope restriction; got: {text_env_wins}"
    );
    assert!(
        text_env_wins.contains("CONFIRMATION_REQUIRED"),
        "env var path should reach mutation gate (CONFIRMATION_REQUIRED); got: {text_env_wins}"
    );
}

// ---------------------------------------------------------------------------
// Phase 41 (#294 + #295): Sandbox path-boundary enforcement — subprocess e2e
//
// Validates at the *subprocess binary boundary* that the MCP server rejects
// `repo` arguments that point outside the session-boundary directory.
//
// The unit test in `src/mcp/mod.rs::repository_argument_cannot_escape_session_boundary`
// covers this at the `dispatch_json_rpc_with_context` call level. This test
// exercises the same invariant through the full transport stack (spawn →
// stdin JSON-RPC → stdout parse) so CI confirms the sandboxing contract
// survives the stdio codec and subprocess boundary.
//
// Addresses #294 AC: "sandbox: 로컬 worktree 외부 접근 차단"
// Addresses #295 AC: "CI smoke test 통합" (runs via `cargo test`)
// ---------------------------------------------------------------------------

/// A `repo` argument that is an absolute path outside the session boundary
/// must return `SANDBOX_VIOLATION` through the subprocess transport.
///
/// The outside path must not appear in the error text (information leak guard).
#[test]
fn mcp_serve_sandbox_boundary_violation_rejected() {
    let sb = sandbox_repo();
    // Create a distinct temp directory that is *outside* the sandbox repo.
    // The server process is started in `sb`, so any path that is not a
    // descendant of `sb.path()` must trigger the sandbox guard.
    let outside = tempfile::TempDir::new().expect("outside TempDir");
    let outside_str = outside.path().to_str().expect("outside path to str");

    let res = run_serve_session(
        &sb,
        &[serde_json::json!({
            "jsonrpc": "2.0",
            "id": "sandbox-boundary",
            "method": "tools/call",
            "params": {
                "name": "worktree_list",
                "arguments": { "repo": outside_str }
            }
        })],
    );

    assert_eq!(
        res[0]["result"]["isError"], true,
        "out-of-boundary repo argument must be an error; got: {:?}",
        res[0]
    );
    let text = res[0]["result"]["content"][0]["text"]
        .as_str()
        .expect("MCP error envelope must contain text");
    assert!(
        text.contains("SANDBOX_VIOLATION"),
        "out-of-boundary repo must produce SANDBOX_VIOLATION; got: {text}"
    );
    // Information-leak guard: the raw outside path must not appear in the response.
    assert!(
        !text.contains(outside_str),
        "SANDBOX_VIOLATION response must not echo the requested path; got: {text}"
    );
}

/// A `repo` argument that resolves to a directory *inside* the session
/// boundary (a sub-directory of the sandbox root) must not trigger
/// `SANDBOX_VIOLATION` — the path guard is boundary-inclusive.
///
/// The call will still fail (no worktrees under the child path), but the
/// failure must be a tool-level error rather than a `SANDBOX_VIOLATION`.
#[test]
fn mcp_serve_sandbox_descendant_path_allowed() {
    let sb = sandbox_repo();
    // Create a sub-directory inside the sandbox boundary to use as `repo`.
    let child = sb.path().join("inner-worktree");
    std::fs::create_dir(&child).expect("child directory");
    let child_str = child.to_str().expect("child path to str");

    let res = run_serve_session(
        &sb,
        &[serde_json::json!({
            "jsonrpc": "2.0",
            "id": "sandbox-child",
            "method": "tools/call",
            "params": {
                "name": "worktree_list",
                "arguments": { "repo": child_str }
            }
        })],
    );

    let text = res[0]["result"]["content"][0]["text"]
        .as_str()
        .expect("MCP response must contain text");
    assert!(
        !text.contains("SANDBOX_VIOLATION"),
        "descendant path must not trigger SANDBOX_VIOLATION; got: {text}"
    );
}
