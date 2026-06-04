//! MCP tool stub for `pr_status`. Wired in Phase 3 (#293).

use crate::mcp::McpContext;

/// `pr_status` — GitHub PR state, review approvals, and merge readiness.
#[allow(dead_code)]
pub fn status(_ctx: &McpContext, _input: serde_json::Value) -> anyhow::Result<serde_json::Value> {
    todo!("pr_status: implement in Phase 3 (#293)")
}
