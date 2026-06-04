//! MCP tool stub for `health_check`. Wired in Phase 3 (#293).

use crate::mcp::McpContext;

/// `health_check` — run worktree health diagnostics.
#[allow(dead_code)]
pub fn check(_ctx: &McpContext, _input: serde_json::Value) -> anyhow::Result<serde_json::Value> {
    todo!("health_check: implement in Phase 3 (#293)")
}
