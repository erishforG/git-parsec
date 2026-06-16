//! MCP tool stub for `ci_status`. Wired in Phase 3 (#293).

use crate::mcp::McpContext;

/// `ci_status` — fetch GitHub Actions check-run results for a worktree branch.
#[allow(dead_code)]
pub fn status(_ctx: &McpContext, _input: serde_json::Value) -> anyhow::Result<serde_json::Value> {
    Err(super::not_implemented("ci_status"))
}
