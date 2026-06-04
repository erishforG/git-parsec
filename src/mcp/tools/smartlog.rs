//! MCP tool stub for `smartlog`.
//!
//! Phase 1 stub — see `docs/mcp/spec.md` for the full schema.
//! Wired in Phase 3 (#293).

use crate::mcp::McpContext;

/// `smartlog` — render the commit DAG with PR/CI overlays.
#[allow(dead_code)]
pub fn run(_ctx: &McpContext, _input: serde_json::Value) -> anyhow::Result<serde_json::Value> {
    todo!("smartlog: implement in Phase 3 (#293)")
}
