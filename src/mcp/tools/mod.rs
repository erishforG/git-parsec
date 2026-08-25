//! MCP tool handler modules.
//!
//! Each sub-module corresponds to one or more tools in the catalogue defined
//! in `docs/mcp/spec.md`. Handlers are intentionally thin while the MCP
//! transport matures; concrete CLI integrations land behind this dispatch
//! boundary in later phases (issue #293).
//!
//! ## Adding a new tool
//!
//! 1. Add a `ToolDef` entry to `crate::mcp::TOOLS`.
//! 2. Create (or extend) a sub-module here.
//! 3. Implement `pub fn handle(ctx: &McpContext, input: serde_json::Value) -> anyhow::Result<serde_json::Value>`.
//! 4. Register the handler in [`dispatch`].

use crate::mcp::McpContext;

pub mod ci;
pub mod health;
pub mod pr;
pub mod reviews;
pub mod smartlog;
pub mod sync;
pub mod worktree;

/// Shared MCP tool handler signature.
pub type ToolHandler = fn(&McpContext, serde_json::Value) -> anyhow::Result<serde_json::Value>;

/// Return the executable handler for a registered MCP tool name.
#[must_use]
pub fn handler_for(name: &str) -> Option<ToolHandler> {
    match name {
        "worktree_list" => Some(worktree::list),
        "worktree_start" => Some(worktree::start),
        "worktree_status" => Some(worktree::status),
        "worktree_ship" => Some(worktree::ship),
        "smartlog" => Some(smartlog::run),
        "ci_status" => Some(ci::status),
        "pr_status" => Some(pr::status),
        "health_check" => Some(health::check),
        "reviews" => Some(reviews::list),
        "sync" => Some(sync::run),
        _ => None,
    }
}

/// Dispatch an MCP tool call to its module-level handler.
///
/// # Errors
/// Returns the handler error when the tool is registered but its implementation
/// cannot complete. Unknown tools return an explicit error for defensive use.
pub fn dispatch(
    name: &str,
    ctx: &McpContext,
    input: serde_json::Value,
) -> anyhow::Result<serde_json::Value> {
    let Some(handler) = handler_for(name) else {
        anyhow::bail!("unknown MCP tool '{name}'");
    };

    handler(ctx, input)
}

/// Produce a structured error for tools that are registered but not yet wired.
///
/// All 10 built-in parsec tools are wired as of Phase 31. Use this helper
/// when adding a new tool before its handler is ready.
pub(crate) fn not_implemented(tool: &str) -> anyhow::Error {
    anyhow::anyhow!("{tool}: handler is not yet wired (see docs/mcp/spec.md for phase status)")
}
