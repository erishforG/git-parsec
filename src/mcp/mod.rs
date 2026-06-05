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
//! - **Phase 1** (this): module skeleton + tool registry shape.
//! - **Phase 2** (#293): stdio JSON-RPC echo server (`parsec mcp serve`).
//! - **Phase 3** (#293): wire real implementations to registered tools.

// Phase 2: McpServer (stdio JSON-RPC) is in server.rs.
#![allow(dead_code)]

pub mod server;
pub mod tools;

pub use server::McpServer;

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
/// In Phase 2 this will become a full `async fn` trait with JSON-Schema
/// introspection; for now it carries the metadata that `tools/list` needs.
pub struct ToolDef {
    /// Machine-readable tool name (snake_case, matches spec).
    pub name: &'static str,
    /// Human-readable description returned in `tools/list` responses.
    pub description: &'static str,
}

/// All tools exposed by the parsec MCP server.
///
/// This slice is the single source of truth for `tools/list` responses.
/// Add new tools here **and** implement them in `src/mcp/tools/`.
pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "worktree_list",
        description: "List all active parsec worktrees with ticket, branch, PR, and CI status.",
    },
    ToolDef {
        name: "worktree_start",
        description: "Create an isolated git worktree for a ticket.",
    },
    ToolDef {
        name: "worktree_status",
        description:
            "Show detailed status of a worktree: uncommitted changes, ahead/behind, PR, CI.",
    },
    ToolDef {
        name: "worktree_ship",
        description:
            "Push the worktree branch to origin, create/update a GitHub PR, and optionally clean up.",
    },
    ToolDef {
        name: "smartlog",
        description:
            "Render the smartlog DAG annotated with worktree branches, PR state, and CI status.",
    },
    ToolDef {
        name: "ci_status",
        description: "Fetch GitHub Actions CI check-run status for a worktree branch.",
    },
    ToolDef {
        name: "pr_status",
        description:
            "Return GitHub PR state, review approvals, merge readiness, and review comments.",
    },
    ToolDef {
        name: "health_check",
        description:
            "Run worktree health diagnostics: lock files, uncommitted changes, stale branches.",
    },
    ToolDef {
        name: "reviews",
        description:
            "List PRs where the user is a requested reviewer or their own PRs awaiting review.",
    },
    ToolDef {
        name: "sync",
        description: "Rebase or merge-update stale worktrees against the current base branch.",
    },
];

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
}
