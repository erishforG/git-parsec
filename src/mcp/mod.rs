//! MCP (Model Context Protocol) server implementation for git-parsec.
//!
//! This module exposes parsec's worktree-native git workflow as an MCP server
//! over stdio JSON-RPC 2.0, allowing AI agents (Claude Desktop, Cursor, etc.)
//! to manage worktrees programmatically.
//!
//! ## Architecture
//!
//! ```text
//! parsec mcp serve
//!   └─ McpServer (stdio JSON-RPC loop)
//!        └─ ToolRegistry
//!             ├─ worktree_list    → tools::worktree::list
//!             ├─ worktree_start   → tools::worktree::start
//!             ├─ worktree_status  → tools::worktree::status
//!             ├─ worktree_ship    → tools::worktree::ship
//!             ├─ smartlog         → tools::smartlog::run
//!             ├─ ci_status        → tools::ci::status
//!             ├─ pr_status        → tools::pr::status
//!             ├─ health_check     → tools::health::check
//!             ├─ reviews          → tools::reviews::list
//!             └─ sync             → tools::sync::run
//! ```
//!
//! See `docs/mcp/spec.md` for the full tool catalogue and JSON schemas.
//!
//! ## Phases
//!
//! - **Phase 1**: module skeleton + tool registry shape.
//! - **Phase 2** (this, #293): stdio JSON-RPC skeleton (`parsec mcp serve`).
//! - **Phase 3** (#293): replace structured stubs with real tool implementations.

// Phase 1 skeleton: items are defined but the JSON-RPC dispatcher (Phase 2)
// has not been wired yet. Suppress dead_code until Phase 2 lands.
#![allow(dead_code)]

pub mod tools;

use std::io::{BufRead, Write};

/// GitHub API capability requested by an MCP tool.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum GithubScope {
    /// Read pull request metadata, reviews, and mergeability state.
    PullRequestRead,
    /// Read GitHub Actions check runs and workflow status.
    ChecksRead,
    /// Push branches and create or update pull requests.
    PullRequestWrite,
}

impl GithubScope {
    /// Stable identifier used by MCP metadata and future auth errors.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PullRequestRead => "pull_request:read",
            Self::ChecksRead => "checks:read",
            Self::PullRequestWrite => "pull_request:write",
        }
    }
}

/// Delegated GitHub token that redacts its secret in debug output.
#[derive(Clone, Eq, PartialEq)]
pub struct DelegatedGithubToken {
    token: String,
}

impl DelegatedGithubToken {
    /// Create a delegated token from client-provided session auth.
    #[must_use]
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
        }
    }

    /// Borrow the raw token for the GitHub client boundary.
    #[must_use]
    pub fn expose_secret(&self) -> &str {
        &self.token
    }
}

impl std::fmt::Debug for DelegatedGithubToken {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DelegatedGithubToken")
            .field("token", &"<redacted>")
            .finish()
    }
}

/// Context passed to every MCP tool handler.
///
/// Carries repository path, auth tokens (delegated by the caller), and
/// runtime flags like `dry_run`. Tools must not read auth from environment
/// directly; they must use this context.
#[derive(Debug, Clone)]
pub struct McpContext {
    /// Absolute path to the git repository root.
    pub repo_path: std::path::PathBuf,

    /// GitHub Personal Access Token delegated by the MCP caller.
    /// `None` when the tool does not require GitHub API access.
    pub github_token: Option<DelegatedGithubToken>,

    /// When `true`, all mutating operations are previewed without
    /// side effects. Tools must check this before any state change.
    pub dry_run: bool,
}

impl McpContext {
    /// Create a context from the current working directory.
    pub fn from_cwd(dry_run: bool) -> anyhow::Result<Self> {
        let repo_path = std::env::current_dir()?;
        Ok(Self {
            repo_path,
            github_token: None,
            dry_run,
        })
    }

    /// Attach a GitHub PAT to the context.
    #[must_use]
    pub fn with_github_token(mut self, token: impl Into<String>) -> Self {
        self.github_token = Some(DelegatedGithubToken::new(token));
        self
    }
}

/// A registered MCP tool: name, description, and a handler stub.
///
/// This carries the stable metadata that `tools/list` needs before the
/// JSON-RPC dispatcher is wired. Handler functions remain in `src/mcp/tools/`.
pub struct ToolDef {
    /// Machine-readable tool name (snake_case, matches spec).
    pub name: &'static str,
    /// Human-readable description returned in `tools/list` responses.
    pub description: &'static str,
    /// JSON Schema draft-07 input schema for this tool.
    pub input_schema: &'static str,
    /// Whether this tool can mutate repository or remote state.
    pub mutating: bool,
    /// Whether this tool requires GitHub API credentials for normal operation.
    pub requires_github: bool,
    /// Minimum GitHub API scopes needed before dispatch.
    pub github_scopes: &'static [GithubScope],
}

/// All tools exposed by the parsec MCP server.
///
/// This slice is the single source of truth for `tools/list` responses.
/// Add new tools here **and** implement them in `src/mcp/tools/`.
pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "worktree_list",
        description: "List all active parsec worktrees with ticket, branch, PR, and CI status.",
        input_schema: r#"{"type":"object","properties":{"repo":{"type":"string"},"include_pr":{"type":"boolean","default":true},"include_ci":{"type":"boolean","default":false}},"required":[]}"#,
        mutating: false,
        requires_github: false,
        github_scopes: &[GithubScope::PullRequestRead, GithubScope::ChecksRead],
    },
    ToolDef {
        name: "worktree_start",
        description: "Create an isolated git worktree for a ticket.",
        input_schema: r#"{"type":"object","properties":{"ticket":{"type":"string"},"repo":{"type":"string"},"base":{"type":"string"},"title":{"type":"string"},"on":{"type":"string"},"dry_run":{"type":"boolean","default":false}},"required":["ticket"]}"#,
        mutating: true,
        requires_github: false,
        github_scopes: &[],
    },
    ToolDef {
        name: "worktree_status",
        description:
            "Show detailed status of a worktree: uncommitted changes, ahead/behind, PR, CI.",
        input_schema: r#"{"type":"object","properties":{"ticket":{"type":"string"},"repo":{"type":"string"}},"required":["ticket"]}"#,
        mutating: false,
        requires_github: false,
        github_scopes: &[GithubScope::PullRequestRead],
    },
    ToolDef {
        name: "worktree_ship",
        description:
            "Push the worktree branch to origin, create/update a GitHub PR, and optionally clean up.",
        input_schema: r#"{"type":"object","properties":{"ticket":{"type":"string"},"repo":{"type":"string"},"draft":{"type":"boolean","default":false},"no_cleanup":{"type":"boolean","default":false},"dry_run":{"type":"boolean","default":false}},"required":["ticket"]}"#,
        mutating: true,
        requires_github: true,
        github_scopes: &[GithubScope::PullRequestWrite],
    },
    ToolDef {
        name: "smartlog",
        description:
            "Render the smartlog DAG annotated with worktree branches, PR state, and CI status.",
        input_schema: r#"{"type":"object","properties":{"repo":{"type":"string"},"ticket":{"type":"string"},"limit":{"type":"integer","default":50,"minimum":1,"maximum":500},"no_color":{"type":"boolean","default":true}},"required":[]}"#,
        mutating: false,
        requires_github: false,
        github_scopes: &[GithubScope::PullRequestRead, GithubScope::ChecksRead],
    },
    ToolDef {
        name: "ci_status",
        description: "Fetch GitHub Actions CI check-run status for a worktree branch.",
        input_schema: r#"{"type":"object","properties":{"ticket":{"type":"string"},"repo":{"type":"string"},"limit":{"type":"integer","default":10}},"required":["ticket"]}"#,
        mutating: false,
        requires_github: true,
        github_scopes: &[GithubScope::ChecksRead],
    },
    ToolDef {
        name: "pr_status",
        description:
            "Return GitHub PR state, review approvals, merge readiness, and review comments.",
        input_schema: r#"{"type":"object","properties":{"ticket":{"type":"string"},"repo":{"type":"string"}},"required":["ticket"]}"#,
        mutating: false,
        requires_github: true,
        github_scopes: &[GithubScope::PullRequestRead],
    },
    ToolDef {
        name: "health_check",
        description:
            "Run worktree health diagnostics: lock files, uncommitted changes, stale branches.",
        input_schema: r#"{"type":"object","properties":{"ticket":{"type":"string"},"repo":{"type":"string"},"include_ci":{"type":"boolean","default":false}},"required":[]}"#,
        mutating: false,
        requires_github: false,
        github_scopes: &[GithubScope::ChecksRead],
    },
    ToolDef {
        name: "reviews",
        description:
            "List PRs where the user is a requested reviewer or their own PRs awaiting review.",
        input_schema: r#"{"type":"object","properties":{"repo":{"type":"string"},"filter":{"type":"string","enum":["incoming","outgoing","all"],"default":"all"},"limit":{"type":"integer","default":20}},"required":[]}"#,
        mutating: false,
        requires_github: true,
        github_scopes: &[GithubScope::PullRequestRead],
    },
    ToolDef {
        name: "sync",
        description: "Rebase or merge-update stale worktrees against the current base branch.",
        input_schema: r#"{"type":"object","properties":{"ticket":{"type":"string"},"repo":{"type":"string"},"strategy":{"type":"string","enum":["rebase","merge"],"default":"rebase"},"dry_run":{"type":"boolean","default":true},"stale_days":{"type":"integer","default":5}},"required":[]}"#,
        mutating: true,
        requires_github: false,
        github_scopes: &[],
    },
];

/// Return the registered definition for a tool name.
#[must_use]
pub fn tool_by_name(name: &str) -> Option<&'static ToolDef> {
    TOOLS.iter().find(|tool| tool.name == name)
}

/// Render tool metadata in the shape required by MCP `tools/list`.
///
/// # Errors
/// Returns an error if a checked-in schema string is invalid JSON.
pub fn tools_list_payload() -> anyhow::Result<serde_json::Value> {
    let tools = TOOLS
        .iter()
        .map(|tool| {
            let input_schema: serde_json::Value = serde_json::from_str(tool.input_schema)?;
            Ok(serde_json::json!({
                "name": tool.name,
                "description": tool.description,
                "inputSchema": input_schema,
                "annotations": {
                    "mutating": tool.mutating,
                    "requiresGithub": tool.requires_github,
                    "githubScopes": tool.github_scopes
                        .iter()
                        .map(|scope| scope.as_str())
                        .collect::<Vec<_>>(),
                },
            }))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    Ok(serde_json::json!({ "tools": tools }))
}

/// Serve newline-delimited JSON-RPC 2.0 messages over stdio.
///
/// This Phase 2 skeleton establishes the transport boundary used by MCP
/// clients. It supports `initialize`, `tools/list`, `tools/call`, and an
/// explicit `echo` method for smoke tests.
pub fn serve_stdio(dry_run: bool) -> anyhow::Result<()> {
    serve(std::io::stdin().lock(), std::io::stdout().lock(), dry_run)
}

fn serve<R, W>(reader: R, writer: W, dry_run: bool) -> anyhow::Result<()>
where
    R: BufRead,
    W: Write,
{
    let ctx = McpContext::from_cwd(dry_run)?;
    let mut writer = writer;

    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        let response = match serde_json::from_str::<serde_json::Value>(&line) {
            Ok(request) => dispatch_json_rpc_with_context(request, &ctx),
            Err(err) => Some(json_rpc_error(
                serde_json::Value::Null,
                -32700,
                "Parse error",
                err,
            )),
        };

        if let Some(response) = response {
            serde_json::to_writer(&mut writer, &response)?;
            writeln!(writer)?;
            writer.flush()?;
        }
    }

    Ok(())
}

fn dispatch_json_rpc(request: serde_json::Value) -> Option<serde_json::Value> {
    let ctx = McpContext::from_cwd(false).ok()?;
    dispatch_json_rpc_with_context(request, &ctx)
}

fn dispatch_json_rpc_with_context(
    request: serde_json::Value,
    ctx: &McpContext,
) -> Option<serde_json::Value> {
    if is_notification(&request) {
        return None;
    }

    let id = request
        .get("id")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let method = request
        .get("method")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();

    match method {
        "initialize" => Some(json_rpc_result(
            id,
            serde_json::json!({
                "protocolVersion": "2025-03-26",
                "serverInfo": {
                    "name": "git-parsec",
                    "version": env!("CARGO_PKG_VERSION"),
                },
                "capabilities": {
                    "tools": {},
                },
            }),
        )),
        "tools/list" => match tools_list_payload() {
            Ok(payload) => Some(json_rpc_result(id, payload)),
            Err(err) => Some(json_rpc_error(id, -32603, "Internal error", err)),
        },
        "echo" => Some(json_rpc_result(
            id,
            request
                .get("params")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
        )),
        "tools/call" => Some(handle_tools_call(id, request.get("params"), ctx)),
        "" => Some(json_rpc_error(
            id,
            -32600,
            "Invalid Request",
            "missing method",
        )),
        _ => Some(json_rpc_error(id, -32601, "Method not found", method)),
    }
}

fn is_notification(request: &serde_json::Value) -> bool {
    request
        .as_object()
        .is_some_and(|object| !object.contains_key("id"))
}

fn json_rpc_result(id: serde_json::Value, result: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    })
}

fn json_rpc_error(
    id: serde_json::Value,
    code: i64,
    message: &'static str,
    data: impl std::fmt::Display,
) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message,
            "data": data.to_string(),
        },
    })
}

fn handle_tools_call(
    id: serde_json::Value,
    params: Option<&serde_json::Value>,
    ctx: &McpContext,
) -> serde_json::Value {
    let Some(params) = params.and_then(serde_json::Value::as_object) else {
        return json_rpc_error(
            id,
            -32602,
            "Invalid params",
            "tools/call params must be an object",
        );
    };
    let Some(name) = params.get("name").and_then(serde_json::Value::as_str) else {
        return json_rpc_error(
            id,
            -32602,
            "Invalid params",
            "tools/call params.name must be a string",
        );
    };
    if !params
        .get("arguments")
        .is_none_or(serde_json::Value::is_object)
    {
        return json_rpc_error(
            id,
            -32602,
            "Invalid params",
            "tools/call params.arguments must be an object when present",
        );
    }
    if tool_by_name(name).is_none() {
        return json_rpc_error(
            id,
            -32602,
            "Invalid params",
            format!("unknown MCP tool '{name}'"),
        );
    }
    let tool = tool_by_name(name).expect("tool was checked above");

    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));

    if let Some(error) = preflight_tool_call(tool, ctx) {
        return json_rpc_result(id, mcp_content_envelope(error, true));
    }

    match tools::dispatch(name, ctx, arguments) {
        Ok(payload) => json_rpc_result(id, mcp_content_envelope(payload, false)),
        Err(err) => json_rpc_result(
            id,
            mcp_content_envelope(
                serde_json::json!({
                    "error": {
                        "code": "tool_error",
                        "message": err.to_string(),
                        "tool": name,
                    }
                }),
                true,
            ),
        ),
    }
}

fn preflight_tool_call(tool: &ToolDef, ctx: &McpContext) -> Option<serde_json::Value> {
    if tool.requires_github && ctx.github_token.is_none() {
        return Some(tool_error_payload(
            "AUTH_REQUIRED",
            format!("Tool '{}' requires a delegated GitHub token.", tool.name),
            format!(
                "Pass a session token with scopes: {}.",
                format_github_scopes(tool.github_scopes)
            ),
            tool.name,
        ));
    }

    None
}

fn tool_error_payload(
    code: &'static str,
    message: String,
    detail: String,
    tool: &'static str,
) -> serde_json::Value {
    serde_json::json!({
        "error": {
            "code": code,
            "message": message,
            "detail": detail,
            "tool": tool,
        }
    })
}

fn format_github_scopes(scopes: &[GithubScope]) -> String {
    if scopes.is_empty() {
        "none".to_owned()
    } else {
        scopes
            .iter()
            .map(|scope| scope.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn mcp_content_envelope(payload: serde_json::Value, is_error: bool) -> serde_json::Value {
    serde_json::json!({
        "content": [{
            "type": "text",
            "text": payload.to_string(),
        }],
        "isError": is_error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_list_is_non_empty() {
        assert!(!TOOLS.is_empty(), "TOOLS registry must not be empty");
    }

    #[test]
    fn tool_names_are_snake_case_and_unique() {
        let mut seen = std::collections::HashSet::new();
        for tool in TOOLS {
            // snake_case: only lowercase letters, digits, underscores
            assert!(
                tool.name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
                "Tool name '{}' is not snake_case",
                tool.name
            );
            assert!(
                seen.insert(tool.name),
                "Duplicate tool name: '{}'",
                tool.name
            );
        }
    }

    #[test]
    fn all_tools_have_descriptions() {
        for tool in TOOLS {
            assert!(
                !tool.description.is_empty(),
                "Tool '{}' has an empty description",
                tool.name
            );
        }
    }

    #[test]
    fn all_tool_schemas_are_valid_json_objects() {
        for tool in TOOLS {
            let schema: serde_json::Value = serde_json::from_str(tool.input_schema)
                .unwrap_or_else(|err| panic!("Tool '{}' has invalid schema: {err}", tool.name));
            assert_eq!(
                schema.get("type").and_then(serde_json::Value::as_str),
                Some("object"),
                "Tool '{}' schema must be an object",
                tool.name
            );
            assert!(
                schema.get("properties").is_some(),
                "Tool '{}' schema must declare properties",
                tool.name
            );
            assert!(
                schema.get("required").is_some(),
                "Tool '{}' schema must declare required fields",
                tool.name
            );
        }
    }

    #[test]
    fn tools_list_payload_matches_registry() {
        let payload = tools_list_payload().expect("tools/list payload should render");
        let tools = payload
            .get("tools")
            .and_then(serde_json::Value::as_array)
            .expect("tools/list payload must contain an array");

        assert_eq!(tools.len(), TOOLS.len());
        for (payload_tool, registry_tool) in tools.iter().zip(TOOLS) {
            assert_eq!(payload_tool["name"], registry_tool.name);
            assert_eq!(payload_tool["description"], registry_tool.description);
            assert!(
                payload_tool.get("inputSchema").is_some(),
                "Tool '{}' must include inputSchema",
                registry_tool.name
            );
        }
    }

    #[test]
    fn mutating_tools_expose_dry_run() {
        for tool in TOOLS.iter().filter(|tool| tool.mutating) {
            let schema: serde_json::Value = serde_json::from_str(tool.input_schema).unwrap();
            assert!(
                schema
                    .pointer("/properties/dry_run")
                    .and_then(|value| value.get("type"))
                    .and_then(serde_json::Value::as_str)
                    == Some("boolean"),
                "Mutating tool '{}' must expose dry_run",
                tool.name
            );
        }
    }

    #[test]
    fn github_tools_are_marked() {
        let github_tools: std::collections::HashSet<_> = TOOLS
            .iter()
            .filter(|tool| tool.requires_github)
            .map(|tool| tool.name)
            .collect();

        assert!(github_tools.contains("worktree_ship"));
        assert!(github_tools.contains("ci_status"));
        assert!(github_tools.contains("pr_status"));
        assert!(github_tools.contains("reviews"));
    }

    #[test]
    fn github_tools_declare_scope_metadata() {
        for tool in TOOLS.iter().filter(|tool| tool.requires_github) {
            assert!(
                !tool.github_scopes.is_empty(),
                "GitHub tool '{}' must declare minimum scopes",
                tool.name
            );
        }
    }

    #[test]
    fn tools_list_exposes_auth_annotations() {
        let payload = tools_list_payload().expect("tools/list payload should render");
        let tools = payload["tools"]
            .as_array()
            .expect("tools/list payload must contain an array");
        let pr_status = tools
            .iter()
            .find(|tool| tool["name"] == "pr_status")
            .expect("pr_status should be listed");

        assert_eq!(pr_status["annotations"]["requiresGithub"], true);
        assert_eq!(
            pr_status["annotations"]["githubScopes"],
            serde_json::json!(["pull_request:read"])
        );
    }

    #[test]
    fn mcp_context_from_cwd() {
        let ctx = McpContext::from_cwd(false).expect("should build context from cwd");
        assert!(!ctx.dry_run);
        assert!(ctx.github_token.is_none());
    }

    #[test]
    fn mcp_context_with_token() {
        let ctx = McpContext::from_cwd(true)
            .unwrap()
            .with_github_token("ghp_test");
        assert!(ctx.dry_run);
        assert_eq!(
            ctx.github_token
                .as_ref()
                .map(DelegatedGithubToken::expose_secret),
            Some("ghp_test")
        );
    }

    #[test]
    fn delegated_token_debug_is_redacted() {
        let token = DelegatedGithubToken::new("ghp_do_not_log");

        let debug = format!("{token:?}");

        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains("ghp_do_not_log"));
    }

    #[test]
    fn initialize_returns_server_capabilities() {
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {}
        });

        let response = dispatch_json_rpc(request).expect("initialize should produce a response");

        assert_eq!(response["jsonrpc"], "2.0");
        assert_eq!(response["id"], 1);
        assert_eq!(response["result"]["serverInfo"]["name"], "git-parsec");
        assert!(response["result"]["capabilities"]["tools"].is_object());
    }

    #[test]
    fn tools_list_method_uses_registry_payload() {
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": "tools",
            "method": "tools/list"
        });

        let response = dispatch_json_rpc(request).expect("tools/list should produce a response");
        let tools = response["result"]["tools"]
            .as_array()
            .expect("tools/list should return an array");

        assert_eq!(tools.len(), TOOLS.len());
        assert_eq!(tools[0]["name"], TOOLS[0].name);
    }

    #[test]
    fn echo_round_trips_params_for_stdio_smoke_tests() {
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "echo",
            "params": {"ok": true}
        });

        let response = dispatch_json_rpc(request).expect("echo should produce a response");

        assert_eq!(response["result"], serde_json::json!({"ok": true}));
    }

    #[test]
    fn unknown_method_returns_json_rpc_error() {
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 9,
            "method": "parsec/nope"
        });

        let response =
            dispatch_json_rpc(request).expect("unknown method should produce a response");

        assert_eq!(response["error"]["code"], -32601);
        assert_eq!(response["error"]["message"], "Method not found");
    }

    #[test]
    fn tools_call_validates_registered_tool_name() {
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": "call",
            "method": "tools/call",
            "params": {
                "name": "nope",
                "arguments": {}
            }
        });

        let response = dispatch_json_rpc(request).expect("tools/call should produce a response");

        assert_eq!(response["error"]["code"], -32602);
        assert_eq!(response["error"]["message"], "Invalid params");
    }

    #[test]
    fn tools_call_returns_mcp_error_envelope_until_dispatch_lands() {
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": "call",
            "method": "tools/call",
            "params": {
                "name": "worktree_status",
                "arguments": {"ticket": "ABC-123"}
            }
        });

        let response = dispatch_json_rpc(request).expect("tools/call should produce a response");

        assert_eq!(response["id"], "call");
        assert_eq!(response["result"]["isError"], true);
        assert_eq!(response["result"]["content"][0]["type"], "text");
        assert!(response["result"]["content"][0]["text"]
            .as_str()
            .is_some_and(|text| text.contains("\"tool\":\"worktree_status\"")));
    }

    #[test]
    fn notifications_do_not_produce_responses() {
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
            "params": {}
        });

        assert!(dispatch_json_rpc(request).is_none());
    }

    #[test]
    fn stdio_recording_fixtures_match_dispatcher() {
        let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/mcp/fixtures/stdio_smoke.jsonl");
        let fixture = std::fs::read_to_string(&fixture_path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", fixture_path.display()));

        for (line_no, line) in fixture.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }

            let record: serde_json::Value = serde_json::from_str(trimmed)
                .unwrap_or_else(|err| panic!("invalid fixture line {}: {err}", line_no + 1));
            let name = record
                .get("name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("<unnamed>");
            let request = record
                .get("request")
                .cloned()
                .unwrap_or_else(|| panic!("fixture '{name}' missing request"));
            let response = dispatch_json_rpc(request);
            if record
                .get("no_response")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
            {
                assert!(
                    response.is_none(),
                    "fixture '{name}' expected no JSON-RPC response"
                );
                continue;
            }

            let response =
                response.unwrap_or_else(|| panic!("fixture '{name}' expected a JSON-RPC response"));
            let assertions = record
                .get("assertions")
                .and_then(serde_json::Value::as_array)
                .unwrap_or_else(|| panic!("fixture '{name}' missing assertions array"));

            for assertion in assertions {
                assert_fixture_assertion(name, &response, assertion);
            }
        }
    }

    fn assert_fixture_assertion(
        name: &str,
        response: &serde_json::Value,
        assertion: &serde_json::Value,
    ) {
        let pointer = assertion
            .get("pointer")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_else(|| panic!("fixture '{name}' assertion missing pointer"));
        let actual = response
            .pointer(pointer)
            .unwrap_or_else(|| panic!("fixture '{name}' pointer '{pointer}' did not match"));

        if let Some(expected) = assertion.get("equals") {
            assert_eq!(
                actual, expected,
                "fixture '{name}' expected {pointer} to equal {expected}"
            );
        }

        if let Some(min_len) = assertion.get("min_len").and_then(serde_json::Value::as_u64) {
            let actual_len = actual
                .as_array()
                .unwrap_or_else(|| panic!("fixture '{name}' pointer '{pointer}' is not an array"))
                .len() as u64;
            assert!(
                actual_len >= min_len,
                "fixture '{name}' expected {pointer} length >= {min_len}, got {actual_len}"
            );
        }

        if let Some(kind) = assertion.get("kind").and_then(serde_json::Value::as_str) {
            let matches = match kind {
                "array" => actual.is_array(),
                "boolean" => actual.is_boolean(),
                "null" => actual.is_null(),
                "number" => actual.is_number(),
                "object" => actual.is_object(),
                "string" => actual.is_string(),
                _ => panic!("fixture '{name}' has unsupported kind '{kind}'"),
            };
            assert!(
                matches,
                "fixture '{name}' expected {pointer} to be {kind}, got {actual}"
            );
        }

        if let Some(needle) = assertion
            .get("contains_text")
            .and_then(serde_json::Value::as_str)
        {
            let actual_text = actual
                .as_str()
                .unwrap_or_else(|| panic!("fixture '{name}' pointer '{pointer}' is not a string"));
            assert!(
                actual_text.contains(needle),
                "fixture '{name}' expected {pointer} to contain '{needle}', got '{actual_text}'"
            );
        }

        if let Some(tool_name) = assertion
            .get("contains_tool")
            .and_then(serde_json::Value::as_str)
        {
            let tools = actual
                .as_array()
                .unwrap_or_else(|| panic!("fixture '{name}' pointer '{pointer}' is not an array"));
            assert!(
                tools
                    .iter()
                    .any(|tool| tool.get("name").and_then(serde_json::Value::as_str)
                        == Some(tool_name)),
                "fixture '{name}' expected {pointer} to contain tool '{tool_name}'"
            );
        }
    }
}
