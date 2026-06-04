//! MCP tool handler modules.
//!
//! Each sub-module corresponds to one or more tools in the catalogue defined
//! in `docs/mcp/spec.md`. Handlers are stubs in Phase 1; they will be wired
//! to real implementations in Phase 3 (issue #293).
//!
//! ## Adding a new tool
//!
//! 1. Add a `ToolDef` entry to `crate::mcp::TOOLS`.
//! 2. Create (or extend) a sub-module here.
//! 3. Implement `pub fn handle(ctx: &McpContext, input: serde_json::Value) -> anyhow::Result<serde_json::Value>`.
//! 4. Register the handler in `McpServer::dispatch` (Phase 2).

// Phase 1: modules are declared but contain only stub signatures.
// Implementations land in Phase 3 when the JSON-RPC server is wired up.
pub mod ci;
pub mod health;
pub mod pr;
pub mod reviews;
pub mod smartlog;
pub mod sync;
pub mod worktree;
