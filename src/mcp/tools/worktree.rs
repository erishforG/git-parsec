//! MCP tool stubs for worktree operations.
//!
//! Tools: `worktree_list`, `worktree_start`, `worktree_status`, `worktree_ship`.
//!
//! These are Phase 1 stubs — signatures and `todo!()` bodies.
//! Real implementations land in Phase 3 (issue #293).
//!
//! The `#[allow(dead_code)]` attributes are intentional: these handlers
//! will be called from the JSON-RPC dispatcher added in Phase 2 (#293).

use crate::mcp::McpContext;

/// `worktree_list` — list all parsec-managed worktrees.
///
/// # Errors
/// Returns an error if the git repository cannot be read.
#[allow(dead_code)]
pub fn list(_ctx: &McpContext, _input: serde_json::Value) -> anyhow::Result<serde_json::Value> {
    // Phase 3: call crate::worktree::manager to enumerate worktrees,
    // optionally fetch PR/CI status via crate::github.
    Err(super::not_implemented("worktree_list"))
}

/// `worktree_start` — create a new worktree for a ticket.
///
/// # Errors
/// Returns an error if the worktree already exists or the ticket is invalid.
#[allow(dead_code)]
pub fn start(_ctx: &McpContext, _input: serde_json::Value) -> anyhow::Result<serde_json::Value> {
    // Phase 3: call crate::worktree::lifecycle::create.
    Err(super::not_implemented("worktree_start"))
}

/// `worktree_status` — detailed status for a single worktree.
///
/// # Errors
/// Returns an error if the ticket has no associated worktree.
#[allow(dead_code)]
pub fn status(_ctx: &McpContext, _input: serde_json::Value) -> anyhow::Result<serde_json::Value> {
    // Phase 3: combine git2 status + github PR/CI queries.
    Err(super::not_implemented("worktree_status"))
}

/// `worktree_ship` — push branch, open/update PR, optionally clean up.
///
/// # Errors
/// Returns an error if the worktree is dirty or the push fails.
#[allow(dead_code)]
pub fn ship(_ctx: &McpContext, _input: serde_json::Value) -> anyhow::Result<serde_json::Value> {
    // Phase 3: call crate::cli::commands::ship internals.
    // Respect ctx.dry_run before any side effects.
    Err(super::not_implemented("worktree_ship"))
}
