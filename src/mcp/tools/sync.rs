//! MCP tool stub for `sync`. Wired in Phase 3 (#293).

use crate::mcp::McpContext;

/// `sync` — rebase/merge stale worktrees against base branch.
#[allow(dead_code)]
pub fn run(_ctx: &McpContext, _input: serde_json::Value) -> anyhow::Result<serde_json::Value> {
    Err(super::not_implemented("sync"))
}
