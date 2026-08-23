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
//! - **Phase 2** (#293): stdio JSON-RPC skeleton (`parsec mcp serve`).
//! - **Phase 25** (#293): `smartlog` wired — sync DAG collection (no GitHub overlay).
//! - **Phase 26** (#293): `pr_status` wired via `gh pr view`.
//! - **Phase 27** (#293): `reviews` wired via `gh pr list`.
//! - **Phase 28** (#293): `ci_status` wired via `gh pr view --json statusCheckRollup`.
//! - **Phase 29** (#293): `sync` wired — rebase/merge worktree against base branch (dry_run + confirm gates).
//! - **Phase 30** (#293): `worktree_start` wired — dry_run preview + confirmed worktree creation.
//! - Remaining stubs: `worktree_ship`.

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

    /// Declared scopes for the delegated token.
    /// `None` means the MCP host did not provide scoped metadata yet.
    pub github_scopes: Option<Vec<GithubScope>>,

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
            github_scopes: None,
            dry_run,
        })
    }

    /// Attach a GitHub PAT to the context.
    #[must_use]
    pub fn with_github_token(mut self, token: impl Into<String>) -> Self {
        self.github_token = Some(DelegatedGithubToken::new(token));
        self
    }

    /// Attach a GitHub PAT and its declared scopes to the context.
    #[must_use]
    pub fn with_github_auth(mut self, token: impl Into<String>, scopes: Vec<GithubScope>) -> Self {
        self.github_token = Some(DelegatedGithubToken::new(token));
        self.github_scopes = Some(scopes);
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

/// Privacy-safe outcome recorded for an MCP tool call.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum AuditOutcome {
    Allowed,
    Denied,
    ToolError,
}

/// Schema version emitted with every structured audit event.
const AUDIT_EVENT_VERSION: u64 = 1;

impl AuditOutcome {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Allowed => "allowed",
            Self::Denied => "denied",
            Self::ToolError => "tool_error",
        }
    }
}

/// Build one structured event for the stderr audit sink.
///
/// Arguments, repository paths, credentials, and error messages are
/// deliberately excluded so audit output cannot persist caller secrets.
fn audit_event(
    tool: &ToolDef,
    outcome: AuditOutcome,
    dry_run: bool,
    request_id: &serde_json::Value,
) -> serde_json::Value {
    let mut event = serde_json::json!({
        "event": "mcp.tool_call",
        "version": AUDIT_EVENT_VERSION,
        "tool": tool.name,
        "outcome": outcome.as_str(),
        "mutating": tool.mutating,
        "dryRun": dry_run,
    });
    if let Some(correlation_id) = audit_correlation_id(request_id) {
        event["correlationId"] = correlation_id;
    }
    event
}

fn audit_correlation_id(request_id: &serde_json::Value) -> Option<serde_json::Value> {
    match request_id {
        serde_json::Value::Number(_) => Some(request_id.clone()),
        serde_json::Value::String(value)
            if !value.is_empty()
                && value.len() <= 64
                && value.chars().all(|character| {
                    character.is_ascii_alphanumeric() || "-_.".contains(character)
                }) =>
        {
            Some(request_id.clone())
        }
        _ => None,
    }
}

fn emit_audit_event(
    tool: &ToolDef,
    outcome: AuditOutcome,
    dry_run: bool,
    request_id: &serde_json::Value,
) {
    let stderr = std::io::stderr();
    let mut sink = stderr.lock();
    emit_audit_event_to(&mut sink, tool, outcome, dry_run, request_id);
}

fn emit_audit_event_to(
    sink: &mut impl std::io::Write,
    tool: &ToolDef,
    outcome: AuditOutcome,
    dry_run: bool,
    request_id: &serde_json::Value,
) {
    // Audit failures must not change the JSON-RPC response or expose call data.
    let _ = write_audit_event(sink, tool, outcome, dry_run, request_id);
}

fn write_audit_event(
    sink: &mut impl std::io::Write,
    tool: &ToolDef,
    outcome: AuditOutcome,
    dry_run: bool,
    request_id: &serde_json::Value,
) -> std::io::Result<()> {
    writeln!(sink, "{}", audit_event(tool, outcome, dry_run, request_id))
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
        input_schema: r#"{"type":"object","properties":{"ticket":{"type":"string"},"repo":{"type":"string"},"base":{"type":"string"},"title":{"type":"string"},"on":{"type":"string"},"dry_run":{"type":"boolean","default":false},"confirm":{"type":"boolean","default":false}},"required":["ticket"]}"#,
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
        input_schema: r#"{"type":"object","properties":{"ticket":{"type":"string"},"repo":{"type":"string"},"draft":{"type":"boolean","default":false},"no_cleanup":{"type":"boolean","default":false},"dry_run":{"type":"boolean","default":false},"confirm":{"type":"boolean","default":false}},"required":["ticket"]}"#,
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
        input_schema: r#"{"type":"object","properties":{"ticket":{"type":"string"},"repo":{"type":"string"},"strategy":{"type":"string","enum":["rebase","merge"],"default":"rebase"},"dry_run":{"type":"boolean","default":true},"confirm":{"type":"boolean","default":false},"stale_days":{"type":"integer","default":5}},"required":[]}"#,
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

/// Render parsec-specific capability metadata for MCP initialization.
#[must_use]
pub fn initialize_capabilities_payload() -> serde_json::Value {
    serde_json::json!({
        "tools": {},
        "gitParsec": {
            "githubScopes": [
                GithubScope::PullRequestRead.as_str(),
                GithubScope::ChecksRead.as_str(),
                GithubScope::PullRequestWrite.as_str(),
            ],
        },
    })
}

/// Serve newline-delimited JSON-RPC 2.0 messages over stdio.
///
/// This Phase 2 skeleton establishes the transport boundary used by MCP
/// clients. It supports `initialize`, `ping`, `tools/list`, `tools/call`, and an
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
    let Some(object) = request.as_object() else {
        return Some(json_rpc_error(
            serde_json::Value::Null,
            -32600,
            "Invalid Request",
            "JSON-RPC request must be an object",
        ));
    };

    let id = object.get("id").cloned().unwrap_or(serde_json::Value::Null);
    if !is_valid_json_rpc_id(&id) {
        return Some(json_rpc_error(
            serde_json::Value::Null,
            -32600,
            "Invalid Request",
            "JSON-RPC id must be a string, number, or null",
        ));
    }

    if object.get("jsonrpc").and_then(serde_json::Value::as_str) != Some("2.0") {
        return Some(json_rpc_error(
            id,
            -32600,
            "Invalid Request",
            "JSON-RPC version must be exactly '2.0'",
        ));
    }

    let Some(method) = object.get("method").and_then(serde_json::Value::as_str) else {
        return Some(json_rpc_error(
            id,
            -32600,
            "Invalid Request",
            "JSON-RPC method must be a string",
        ));
    };

    if is_notification_object(object) {
        return None;
    }

    match method {
        "initialize" => Some(json_rpc_result(
            id,
            serde_json::json!({
                "protocolVersion": "2025-03-26",
                "serverInfo": {
                    "name": "git-parsec",
                    "version": env!("CARGO_PKG_VERSION"),
                },
                "capabilities": initialize_capabilities_payload(),
            }),
        )),
        "ping" => Some(json_rpc_result(id, serde_json::json!({}))),
        "shutdown" => Some(json_rpc_result(id, serde_json::Value::Null)),
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
        _ => Some(json_rpc_error(id, -32601, "Method not found", method)),
    }
}

fn is_notification_object(object: &serde_json::Map<String, serde_json::Value>) -> bool {
    !object.contains_key("id")
}

fn is_valid_json_rpc_id(id: &serde_json::Value) -> bool {
    id.is_null() || id.is_string() || id.is_number()
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

    let dry_run = argument_enabled(&arguments, "dry_run") || ctx.dry_run;

    if let Some(error) = preflight_tool_call(tool, &arguments, ctx) {
        emit_audit_event(tool, AuditOutcome::Denied, dry_run, &id);
        return json_rpc_result(id, mcp_content_envelope(error, true));
    }

    match tools::dispatch(name, ctx, arguments) {
        Ok(payload) => {
            emit_audit_event(tool, AuditOutcome::Allowed, dry_run, &id);
            json_rpc_result(id, mcp_content_envelope(payload, false))
        }
        Err(err) => {
            emit_audit_event(tool, AuditOutcome::ToolError, dry_run, &id);
            json_rpc_result(
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
            )
        }
    }
}

fn preflight_tool_call(
    tool: &ToolDef,
    arguments: &serde_json::Value,
    ctx: &McpContext,
) -> Option<serde_json::Value> {
    if !repo_argument_is_within_boundary(arguments, &ctx.repo_path) {
        return Some(tool_error_payload(
            "SANDBOX_VIOLATION",
            format!(
                "Tool '{}' requested a repository outside the MCP session boundary.",
                tool.name
            ),
            "Use the repository bound to this MCP server session.".to_owned(),
            tool.name,
        ));
    }

    let dry_run = argument_enabled(arguments, "dry_run") || ctx.dry_run;
    if tool.mutating && !dry_run && !argument_enabled(arguments, "confirm") {
        return Some(tool_error_payload(
            "CONFIRMATION_REQUIRED",
            format!("Tool '{}' requires explicit confirmation.", tool.name),
            "Preview with dry_run=true, then retry with confirm=true.".to_owned(),
            tool.name,
        ));
    }

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

    if ctx.github_token.is_some() {
        let required_scopes = requested_github_scopes(tool, arguments);
        if !token_has_scopes(ctx.github_scopes.as_deref(), &required_scopes) {
            return Some(tool_error_payload(
                "INSUFFICIENT_SCOPE",
                format!(
                    "Tool '{}' requires delegated GitHub scopes that were not provided.",
                    tool.name
                ),
                format!(
                    "Pass a session token with scopes: {}.",
                    format_github_scopes(&required_scopes)
                ),
                tool.name,
            ));
        }
    }

    if ctx.github_token.is_none() {
        let requested_scopes = requested_optional_github_scopes(tool, arguments);
        if !requested_scopes.is_empty() {
            return Some(tool_error_payload(
                "AUTH_REQUIRED",
                format!(
                    "Tool '{}' requested a GitHub-backed overlay without a delegated token.",
                    tool.name
                ),
                format!(
                    "Pass a session token with scopes: {}.",
                    format_github_scopes(&requested_scopes)
                ),
                tool.name,
            ));
        }
    }

    None
}

fn repo_argument_is_within_boundary(
    arguments: &serde_json::Value,
    boundary: &std::path::Path,
) -> bool {
    let Some(requested) = arguments.get("repo").and_then(serde_json::Value::as_str) else {
        return true;
    };

    let requested = std::path::Path::new(requested);
    let requested = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        boundary.join(requested)
    };

    match (boundary.canonicalize(), requested.canonicalize()) {
        (Ok(boundary), Ok(requested)) => requested.starts_with(boundary),
        _ => false,
    }
}

fn requested_github_scopes(tool: &ToolDef, arguments: &serde_json::Value) -> Vec<GithubScope> {
    let mut scopes = tool.github_scopes.to_vec();
    for scope in requested_optional_github_scopes(tool, arguments) {
        if !scopes.contains(&scope) {
            scopes.push(scope);
        }
    }
    scopes
}

fn requested_optional_github_scopes(
    tool: &ToolDef,
    arguments: &serde_json::Value,
) -> Vec<GithubScope> {
    let mut scopes = Vec::new();

    if tool.github_scopes.contains(&GithubScope::PullRequestRead)
        && argument_enabled(arguments, "include_pr")
    {
        scopes.push(GithubScope::PullRequestRead);
    }
    if tool.github_scopes.contains(&GithubScope::ChecksRead)
        && argument_enabled(arguments, "include_ci")
    {
        scopes.push(GithubScope::ChecksRead);
    }

    scopes
}

fn token_has_scopes(delegated: Option<&[GithubScope]>, required: &[GithubScope]) -> bool {
    let Some(delegated) = delegated else {
        return true;
    };

    required.iter().all(|scope| {
        delegated.contains(scope)
            || (*scope == GithubScope::PullRequestRead
                && delegated.contains(&GithubScope::PullRequestWrite))
    })
}

fn argument_enabled(arguments: &serde_json::Value, name: &str) -> bool {
    arguments
        .get(name)
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
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

    struct FailingAuditSink;

    impl std::io::Write for FailingAuditSink {
        fn write(&mut self, _buffer: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("audit sink unavailable"))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn tools_list_is_non_empty() {
        assert!(!TOOLS.is_empty(), "TOOLS registry must not be empty");
    }

    #[test]
    fn audit_event_contract_excludes_sensitive_call_data() {
        let tool = tool_by_name("worktree_ship").expect("registered tool");
        let event = audit_event(
            tool,
            AuditOutcome::Denied,
            false,
            &serde_json::json!("call-42"),
        );
        let serialized = event.to_string();

        assert_eq!(event["tool"], "worktree_ship");
        assert_eq!(event["outcome"], "denied");
        assert_eq!(event["correlationId"], "call-42");
        for forbidden in ["token", "arguments", "repo_path", "message"] {
            assert!(!serialized.contains(forbidden));
        }
    }

    #[test]
    fn audit_event_matches_versioned_compatibility_fixture() {
        let tool = tool_by_name("worktree_ship").expect("registered tool");
        let actual = audit_event(
            tool,
            AuditOutcome::Denied,
            false,
            &serde_json::json!("call-42"),
        );
        let expected: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/mcp/fixtures/audit_event_v1.json"))
                .expect("valid audit compatibility fixture");

        assert_eq!(actual, expected);
        assert_eq!(actual["version"], AUDIT_EVENT_VERSION);
    }

    #[test]
    fn audit_writer_emits_one_newline_delimited_event() {
        let tool = tool_by_name("worktree_ship").expect("registered tool");
        let mut sink = Vec::new();

        write_audit_event(
            &mut sink,
            tool,
            AuditOutcome::Allowed,
            true,
            &serde_json::json!(42),
        )
        .expect("in-memory audit write");

        let output = String::from_utf8(sink).expect("UTF-8 audit event");
        assert_eq!(output.lines().count(), 1);
        assert!(output.ends_with('\n'));
        let event: serde_json::Value =
            serde_json::from_str(output.trim_end()).expect("valid JSON audit event");
        assert_eq!(event["outcome"], "allowed");
        assert_eq!(event["dryRun"], true);
        assert_eq!(event["correlationId"], 42);
    }

    #[test]
    fn audit_sink_failure_is_best_effort() {
        let tool = tool_by_name("worktree_ship").expect("registered tool");
        let mut sink = FailingAuditSink;

        emit_audit_event_to(
            &mut sink,
            tool,
            AuditOutcome::Allowed,
            false,
            &serde_json::json!(42),
        );
    }

    #[test]
    fn audit_correlation_id_rejects_sensitive_or_oversized_strings() {
        assert!(audit_correlation_id(&serde_json::json!(42)).is_some());
        assert!(audit_correlation_id(&serde_json::json!("call_42.a")).is_some());
        assert!(audit_correlation_id(&serde_json::json!("token secret")).is_none());
        assert!(audit_correlation_id(&serde_json::json!("x".repeat(65))).is_none());
        assert!(audit_correlation_id(&serde_json::Value::Null).is_none());
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
    fn mutating_tools_expose_confirmation() {
        for tool in TOOLS.iter().filter(|tool| tool.mutating) {
            let schema: serde_json::Value =
                serde_json::from_str(tool.input_schema).expect("valid tool schema");
            assert_eq!(
                schema.pointer("/properties/confirm/type"),
                Some(&serde_json::json!("boolean")),
                "Mutating tool '{}' must expose confirm",
                tool.name
            );
        }
    }

    #[test]
    fn mutation_requires_confirmation_after_preview() {
        let ctx = McpContext::from_cwd(false).expect("context");
        let tool = tool_by_name("worktree_start").expect("registered tool");

        let error = preflight_tool_call(tool, &serde_json::json!({"ticket": "ABC-123"}), &ctx)
            .expect("unconfirmed mutation should fail");
        assert_eq!(error["error"]["code"], "CONFIRMATION_REQUIRED");

        assert!(preflight_tool_call(
            tool,
            &serde_json::json!({"ticket": "ABC-123", "dry_run": true}),
            &ctx
        )
        .is_none());
        assert!(preflight_tool_call(
            tool,
            &serde_json::json!({"ticket": "ABC-123", "confirm": true}),
            &ctx
        )
        .is_none());
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
    fn initialize_capabilities_advertise_github_scopes() {
        let payload = initialize_capabilities_payload();

        assert!(payload["tools"].is_object());
        assert_eq!(
            payload["gitParsec"]["githubScopes"],
            serde_json::json!(["pull_request:read", "checks:read", "pull_request:write"])
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
        assert!(ctx.github_scopes.is_none());
        assert_eq!(
            ctx.github_token
                .as_ref()
                .map(DelegatedGithubToken::expose_secret),
            Some("ghp_test")
        );
    }

    #[test]
    fn mcp_context_with_scoped_token() {
        let ctx = McpContext::from_cwd(false)
            .unwrap()
            .with_github_auth("ghp_test", vec![GithubScope::PullRequestRead]);

        assert_eq!(ctx.github_scopes, Some(vec![GithubScope::PullRequestRead]));
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
        assert_eq!(
            response["result"]["capabilities"]["gitParsec"]["githubScopes"],
            serde_json::json!(["pull_request:read", "checks:read", "pull_request:write"])
        );
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
    fn ping_returns_empty_result_for_client_health_checks() {
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": "ping",
            "method": "ping"
        });

        let response = dispatch_json_rpc(request).expect("ping should produce a response");

        assert_eq!(response["jsonrpc"], "2.0");
        assert_eq!(response["id"], "ping");
        assert_eq!(response["result"], serde_json::json!({}));
    }

    #[test]
    fn shutdown_returns_null_result_for_client_lifecycle() {
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": "shutdown",
            "method": "shutdown"
        });

        let response = dispatch_json_rpc(request).expect("shutdown should produce a response");

        assert_eq!(response["jsonrpc"], "2.0");
        assert_eq!(response["id"], "shutdown");
        assert_eq!(response["result"], serde_json::Value::Null);
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
    fn malformed_request_shape_returns_invalid_request() {
        let response =
            dispatch_json_rpc(serde_json::json!([])).expect("invalid request should respond");

        assert_eq!(response["id"], serde_json::Value::Null);
        assert_eq!(response["error"]["code"], -32600);
        assert_eq!(response["error"]["message"], "Invalid Request");
    }

    #[test]
    fn malformed_request_id_returns_invalid_request() {
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": {"nested": true},
            "method": "initialize"
        });

        let response = dispatch_json_rpc(request).expect("invalid id should respond");

        assert_eq!(response["id"], serde_json::Value::Null);
        assert_eq!(response["error"]["code"], -32600);
        assert_eq!(response["error"]["message"], "Invalid Request");
    }

    #[test]
    fn malformed_json_rpc_version_returns_invalid_request() {
        let request = serde_json::json!({
            "jsonrpc": "1.0",
            "id": "bad-version",
            "method": "initialize"
        });

        let response = dispatch_json_rpc(request).expect("invalid version should respond");

        assert_eq!(response["id"], "bad-version");
        assert_eq!(response["error"]["code"], -32600);
        assert_eq!(response["error"]["message"], "Invalid Request");
        assert!(response["error"]["data"]
            .as_str()
            .is_some_and(|text| text.contains("JSON-RPC version")));
    }

    #[test]
    fn malformed_request_method_returns_invalid_request() {
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": "bad-method",
            "method": 123
        });

        let response = dispatch_json_rpc(request).expect("invalid method should respond");

        assert_eq!(response["id"], "bad-method");
        assert_eq!(response["error"]["code"], -32600);
        assert_eq!(response["error"]["message"], "Invalid Request");
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
    fn optional_github_overlay_requires_token_when_requested() {
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": "overlay-auth",
            "method": "tools/call",
            "params": {
                "name": "worktree_list",
                "arguments": {"include_pr": true, "include_ci": true}
            }
        });

        let response = dispatch_json_rpc(request).expect("tools/call should produce a response");
        let text = response["result"]["content"][0]["text"]
            .as_str()
            .expect("MCP error envelope should contain text");

        assert_eq!(response["id"], "overlay-auth");
        assert_eq!(response["result"]["isError"], true);
        assert!(text.contains("AUTH_REQUIRED"));
        assert!(text.contains("pull_request:read"));
        assert!(text.contains("checks:read"));
    }

    #[test]
    fn optional_github_overlay_allows_delegated_token() {
        let ctx = McpContext::from_cwd(false)
            .expect("context")
            .with_github_token("ghp_test");
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": "overlay-auth-ok",
            "method": "tools/call",
            "params": {
                "name": "worktree_list",
                "arguments": {"include_pr": true}
            }
        });

        let response = dispatch_json_rpc_with_context(request, &ctx)
            .expect("tools/call should produce a response");

        assert_eq!(response["id"], "overlay-auth-ok");
        assert!(
            response.get("error").is_none(),
            "delegated token should pass overlay preflight"
        );
    }

    #[test]
    fn scoped_delegated_token_rejects_missing_scope() {
        let ctx = McpContext::from_cwd(false)
            .expect("context")
            .with_github_auth("ghp_test", vec![GithubScope::PullRequestRead]);
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": "ci-scope",
            "method": "tools/call",
            "params": {
                "name": "ci_status",
                "arguments": {"ticket": "ABC-123"}
            }
        });

        let response = dispatch_json_rpc_with_context(request, &ctx)
            .expect("tools/call should produce a response");
        let text = response["result"]["content"][0]["text"]
            .as_str()
            .expect("MCP error envelope should contain text");

        assert_eq!(response["id"], "ci-scope");
        assert_eq!(response["result"]["isError"], true);
        assert!(text.contains("INSUFFICIENT_SCOPE"));
        assert!(text.contains("checks:read"));
    }

    #[test]
    fn pull_request_write_scope_satisfies_read_requirement() {
        let ctx = McpContext::from_cwd(false)
            .expect("context")
            .with_github_auth("ghp_test", vec![GithubScope::PullRequestWrite]);
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": "pr-scope",
            "method": "tools/call",
            "params": {
                "name": "pr_status",
                "arguments": {"ticket": "ABC-123"}
            }
        });

        let response = dispatch_json_rpc_with_context(request, &ctx)
            .expect("tools/call should produce a response");

        assert_eq!(response["id"], "pr-scope");
        assert!(
            response["result"]["content"][0]["text"]
                .as_str()
                .is_some_and(|text| !text.contains("INSUFFICIENT_SCOPE")),
            "write scope should satisfy pull request read preflight"
        );
    }

    #[test]
    fn repository_argument_cannot_escape_session_boundary() {
        let boundary = tempfile::tempdir().expect("temporary repository boundary");
        let outside = tempfile::tempdir().expect("outside directory");
        let ctx = McpContext {
            repo_path: boundary.path().to_path_buf(),
            github_token: None,
            github_scopes: None,
            dry_run: false,
        };
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": "sandbox",
            "method": "tools/call",
            "params": {
                "name": "worktree_list",
                "arguments": {"repo": outside.path()}
            }
        });

        let response = dispatch_json_rpc_with_context(request, &ctx)
            .expect("tools/call should produce a response");
        let text = response["result"]["content"][0]["text"]
            .as_str()
            .expect("MCP error envelope should contain text");

        assert_eq!(response["result"]["isError"], true);
        assert!(text.contains("SANDBOX_VIOLATION"));
        assert!(!text.contains(&outside.path().display().to_string()));
    }

    #[test]
    fn repository_argument_allows_descendant_of_session_boundary() {
        let boundary = tempfile::tempdir().expect("temporary repository boundary");
        let child = boundary.path().join("worktree");
        std::fs::create_dir(&child).expect("child directory");

        assert!(repo_argument_is_within_boundary(
            &serde_json::json!({"repo": child}),
            boundary.path(),
        ));
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
        let mut fixture_names = std::collections::HashSet::new();

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
            validate_stdio_fixture_record(name, &record, line_no + 1, &mut fixture_names);
            let request = record
                .get("request")
                .cloned()
                .unwrap_or_else(|| panic!("fixture '{name}' missing request"));
            let ctx = context_from_stdio_fixture_record(name, &record);
            let response = dispatch_json_rpc_with_context(request, &ctx);
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

    #[test]
    fn stdio_recording_fixtures_are_redacted() {
        let fixture_dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/mcp/fixtures");
        let forbidden_fragments = [
            "ghp_",
            "github_pat_",
            "Bearer ",
            "Authorization",
            "/Users/",
            "/home/",
            "C:\\Users\\",
        ];

        for entry in std::fs::read_dir(&fixture_dir)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", fixture_dir.display()))
        {
            let path = entry
                .expect("fixture directory entry should be readable")
                .path();
            if path.extension().and_then(std::ffi::OsStr::to_str) != Some("jsonl") {
                continue;
            }

            let fixture = std::fs::read_to_string(&path)
                .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
            for fragment in forbidden_fragments {
                assert!(
                    !fixture.contains(fragment),
                    "fixture {} contains unredacted sensitive fragment '{}'",
                    path.display(),
                    fragment
                );
            }
        }
    }

    fn validate_stdio_fixture_record(
        name: &str,
        record: &serde_json::Value,
        line_no: usize,
        fixture_names: &mut std::collections::HashSet<String>,
    ) {
        assert_ne!(
            name, "<unnamed>",
            "fixture line {line_no} must declare a string name"
        );
        assert!(
            fixture_names.insert(name.to_owned()),
            "fixture name '{name}' is duplicated"
        );
        assert!(
            record.get("request").is_some(),
            "fixture '{name}' must declare a request"
        );
        if let Some(context) = record.get("context") {
            assert!(
                context.is_object(),
                "fixture '{name}' context must be an object"
            );
        }

        let expects_no_response = record
            .get("no_response")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let assertions = record
            .get("assertions")
            .and_then(serde_json::Value::as_array);

        if expects_no_response {
            assert!(
                assertions.is_none(),
                "fixture '{name}' cannot combine no_response with assertions"
            );
        } else {
            assert!(
                assertions.is_some_and(|items| !items.is_empty()),
                "fixture '{name}' must declare at least one assertion"
            );
        }
    }

    fn context_from_stdio_fixture_record(name: &str, record: &serde_json::Value) -> McpContext {
        let mut ctx = McpContext::from_cwd(false).expect("fixture context should load cwd");
        let Some(github) = record
            .get("context")
            .and_then(|context| context.get("github"))
        else {
            return ctx;
        };

        let token = github
            .get("token")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_else(|| panic!("fixture '{name}' github context missing token"));
        let scopes = github
            .get("scopes")
            .and_then(serde_json::Value::as_array)
            .unwrap_or_else(|| panic!("fixture '{name}' github context missing scopes"))
            .iter()
            .map(|scope| {
                let scope = scope
                    .as_str()
                    .unwrap_or_else(|| panic!("fixture '{name}' github scope must be a string"));
                github_scope_from_fixture(name, scope)
            })
            .collect();

        ctx.github_token = Some(DelegatedGithubToken::new(token));
        ctx.github_scopes = Some(scopes);
        ctx
    }

    fn github_scope_from_fixture(name: &str, scope: &str) -> GithubScope {
        match scope {
            "pull_request:read" => GithubScope::PullRequestRead,
            "checks:read" => GithubScope::ChecksRead,
            "pull_request:write" => GithubScope::PullRequestWrite,
            _ => panic!("fixture '{name}' has unsupported GitHub scope '{scope}'"),
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
        let has_matcher = assertion.get("equals").is_some()
            || assertion.get("min_len").is_some()
            || assertion.get("kind").is_some()
            || assertion.get("contains_text").is_some()
            || assertion.get("contains_tool").is_some();
        assert!(
            has_matcher,
            "fixture '{name}' assertion for {pointer} must declare a matcher"
        );

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
