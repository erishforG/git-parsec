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
//! - **Phase 3** (#293): wire real implementations to registered tools.

// Phase 1 skeleton: items are defined but the JSON-RPC dispatcher (Phase 2)
// has not been wired yet. Suppress dead_code until Phase 2 lands.
#![allow(dead_code)]

pub mod tools;

use std::io::{BufRead, Write};

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
    pub github_token: Option<String>,

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
        self.github_token = Some(token.into());
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
    },
    ToolDef {
        name: "worktree_start",
        description: "Create an isolated git worktree for a ticket.",
        input_schema: r#"{"type":"object","properties":{"ticket":{"type":"string"},"repo":{"type":"string"},"base":{"type":"string"},"title":{"type":"string"},"on":{"type":"string"},"dry_run":{"type":"boolean","default":false}},"required":["ticket"]}"#,
        mutating: true,
        requires_github: false,
    },
    ToolDef {
        name: "worktree_status",
        description:
            "Show detailed status of a worktree: uncommitted changes, ahead/behind, PR, CI.",
        input_schema: r#"{"type":"object","properties":{"ticket":{"type":"string"},"repo":{"type":"string"}},"required":["ticket"]}"#,
        mutating: false,
        requires_github: false,
    },
    ToolDef {
        name: "worktree_ship",
        description:
            "Push the worktree branch to origin, create/update a GitHub PR, and optionally clean up.",
        input_schema: r#"{"type":"object","properties":{"ticket":{"type":"string"},"repo":{"type":"string"},"draft":{"type":"boolean","default":false},"no_cleanup":{"type":"boolean","default":false},"dry_run":{"type":"boolean","default":false}},"required":["ticket"]}"#,
        mutating: true,
        requires_github: true,
    },
    ToolDef {
        name: "smartlog",
        description:
            "Render the smartlog DAG annotated with worktree branches, PR state, and CI status.",
        input_schema: r#"{"type":"object","properties":{"repo":{"type":"string"},"ticket":{"type":"string"},"limit":{"type":"integer","default":50,"minimum":1,"maximum":500},"no_color":{"type":"boolean","default":true}},"required":[]}"#,
        mutating: false,
        requires_github: false,
    },
    ToolDef {
        name: "ci_status",
        description: "Fetch GitHub Actions CI check-run status for a worktree branch.",
        input_schema: r#"{"type":"object","properties":{"ticket":{"type":"string"},"repo":{"type":"string"},"limit":{"type":"integer","default":10}},"required":["ticket"]}"#,
        mutating: false,
        requires_github: true,
    },
    ToolDef {
        name: "pr_status",
        description:
            "Return GitHub PR state, review approvals, merge readiness, and review comments.",
        input_schema: r#"{"type":"object","properties":{"ticket":{"type":"string"},"repo":{"type":"string"}},"required":["ticket"]}"#,
        mutating: false,
        requires_github: true,
    },
    ToolDef {
        name: "health_check",
        description:
            "Run worktree health diagnostics: lock files, uncommitted changes, stale branches.",
        input_schema: r#"{"type":"object","properties":{"ticket":{"type":"string"},"repo":{"type":"string"},"include_ci":{"type":"boolean","default":false}},"required":[]}"#,
        mutating: false,
        requires_github: false,
    },
    ToolDef {
        name: "reviews",
        description:
            "List PRs where the user is a requested reviewer or their own PRs awaiting review.",
        input_schema: r#"{"type":"object","properties":{"repo":{"type":"string"},"filter":{"type":"string","enum":["incoming","outgoing","all"],"default":"all"},"limit":{"type":"integer","default":20}},"required":[]}"#,
        mutating: false,
        requires_github: true,
    },
    ToolDef {
        name: "sync",
        description: "Rebase or merge-update stale worktrees against the current base branch.",
        input_schema: r#"{"type":"object","properties":{"ticket":{"type":"string"},"repo":{"type":"string"},"strategy":{"type":"string","enum":["rebase","merge"],"default":"rebase"},"dry_run":{"type":"boolean","default":true},"stale_days":{"type":"integer","default":5}},"required":[]}"#,
        mutating: true,
        requires_github: false,
    },
];

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
            }))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    Ok(serde_json::json!({ "tools": tools }))
}

/// Serve newline-delimited JSON-RPC 2.0 messages over stdio.
///
/// This Phase 2 skeleton establishes the transport boundary used by MCP
/// clients. It supports `initialize`, `tools/list`, and an explicit `echo`
/// method for smoke tests; real tool dispatch lands in the next phase.
pub fn serve_stdio(dry_run: bool) -> anyhow::Result<()> {
    serve(std::io::stdin().lock(), std::io::stdout().lock(), dry_run)
}

fn serve<R, W>(reader: R, writer: W, dry_run: bool) -> anyhow::Result<()>
where
    R: BufRead,
    W: Write,
{
    let _ctx = McpContext::from_cwd(dry_run)?;
    let mut writer = writer;

    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        let response = match serde_json::from_str::<serde_json::Value>(&line) {
            Ok(request) => dispatch_json_rpc(request),
            Err(err) => json_rpc_error(serde_json::Value::Null, -32700, "Parse error", err),
        };

        serde_json::to_writer(&mut writer, &response)?;
        writeln!(writer)?;
        writer.flush()?;
    }

    Ok(())
}

fn dispatch_json_rpc(request: serde_json::Value) -> serde_json::Value {
    let id = request
        .get("id")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let method = request
        .get("method")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();

    match method {
        "initialize" => json_rpc_result(
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
        ),
        "tools/list" => match tools_list_payload() {
            Ok(payload) => json_rpc_result(id, payload),
            Err(err) => json_rpc_error(id, -32603, "Internal error", err),
        },
        "echo" => json_rpc_result(
            id,
            request
                .get("params")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
        ),
        "tools/call" => json_rpc_error(
            id,
            -32601,
            "Method not implemented",
            "tools/call dispatch is planned for the next MCP phase",
        ),
        "" => json_rpc_error(id, -32600, "Invalid Request", "missing method"),
        _ => json_rpc_error(id, -32601, "Method not found", method),
    }
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
        assert_eq!(ctx.github_token.as_deref(), Some("ghp_test"));
    }

    #[test]
    fn initialize_returns_server_capabilities() {
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {}
        });

        let response = dispatch_json_rpc(request);

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

        let response = dispatch_json_rpc(request);
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

        let response = dispatch_json_rpc(request);

        assert_eq!(response["result"], serde_json::json!({"ok": true}));
    }

    #[test]
    fn unknown_method_returns_json_rpc_error() {
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 9,
            "method": "parsec/nope"
        });

        let response = dispatch_json_rpc(request);

        assert_eq!(response["error"]["code"], -32601);
        assert_eq!(response["error"]["message"], "Method not found");
    }
}
