//! CLI handler for `parsec mcp serve` (Phase 2, issue #293).
//!
//! Spins up the stdio JSON-RPC server so AI agents (Claude Desktop, Cursor,
//! etc.) can call parsec tools over MCP.

use anyhow::Result;

use crate::mcp::{McpContext, McpServer};

/// Entry point for `parsec mcp serve`.
///
/// Reads the GitHub token from the `GITHUB_TOKEN` environment variable if
/// present (callers may inject it via their MCP config).  Runs until stdin
/// closes.
///
/// # Errors
///
/// Returns an error if the git repository cannot be detected or if stdout
/// becomes unwritable.
pub async fn mcp_serve(repo_path: &std::path::Path, dry_run: bool) -> Result<()> {
    let mut ctx = McpContext {
        repo_path: repo_path.to_path_buf(),
        github_token: None,
        dry_run,
    };

    // Callers (Claude Desktop, Cursor) inject the GitHub PAT via env.
    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        if !token.is_empty() {
            ctx = ctx.with_github_token(token);
        }
    }

    eprintln!("parsec mcp serve — stdio JSON-RPC 2.0 ready (dry_run={dry_run})");

    // Synchronous loop; spawn_blocking so we don't block the tokio runtime.
    tokio::task::spawn_blocking(move || McpServer::new(ctx).serve()).await??;

    Ok(())
}
