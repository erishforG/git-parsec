//! MCP tool stub for `reviews`. Wired in Phase 3 (#293).

use crate::mcp::McpContext;

/// `reviews` — list incoming and outgoing review requests.
#[allow(dead_code)]
pub fn list(_ctx: &McpContext, _input: serde_json::Value) -> anyhow::Result<serde_json::Value> {
    Err(super::not_implemented("reviews"))
}
