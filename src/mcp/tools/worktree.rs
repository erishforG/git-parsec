//! MCP tool stubs for worktree operations.
//!
//! Tools: `worktree_list`, `worktree_start`, `worktree_status`, `worktree_ship`.
//!
//! `worktree_list` and `worktree_status` are wired as read-only Phase 4
//! handlers. Mutating operations remain stubs until later #293 phases.

use crate::config::ParsecConfig;
use crate::mcp::McpContext;
use crate::worktree::WorktreeManager;
use anyhow::{Context, Result};
use serde_json::json;
use std::path::PathBuf;

fn repo_path(ctx: &McpContext, input: &serde_json::Value) -> PathBuf {
    input
        .get("repo")
        .and_then(serde_json::Value::as_str)
        .map(PathBuf::from)
        .unwrap_or_else(|| ctx.repo_path.clone())
}

fn manager(ctx: &McpContext, input: &serde_json::Value) -> Result<WorktreeManager> {
    let config = ParsecConfig::load().context("failed to load parsec config")?;
    WorktreeManager::new(&repo_path(ctx, input), &config)
}

/// `worktree_list` — list all parsec-managed worktrees.
///
/// # Errors
/// Returns an error if the git repository cannot be read.
pub fn list(ctx: &McpContext, input: serde_json::Value) -> Result<serde_json::Value> {
    let manager = manager(ctx, &input)?;
    let worktrees = manager.list()?;

    Ok(json!({
        "repo": manager.repo_root(),
        "count": worktrees.len(),
        "worktrees": worktrees,
        "pr_overlay": false,
        "ci_overlay": false,
    }))
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
pub fn status(ctx: &McpContext, input: serde_json::Value) -> Result<serde_json::Value> {
    let ticket = input
        .get("ticket")
        .and_then(serde_json::Value::as_str)
        .filter(|ticket| !ticket.is_empty())
        .context("worktree_status requires a non-empty ticket")?;
    crate::worktree::validate_ticket_id(ticket)?;

    let manager = manager(ctx, &input)?;
    let workspace = manager.get(ticket)?;

    Ok(json!({
        "repo": manager.repo_root(),
        "workspace": workspace,
        "pr_overlay": null,
        "ci_overlay": null,
    }))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worktree::{ParsecState, Workspace, WorkspaceStatus};
    use chrono::TimeZone;

    fn fixture_repo() -> (tempfile::TempDir, McpContext) {
        let dir = tempfile::tempdir().expect("temp repo");
        crate::git::run(dir.path(), &["init"]).expect("git init");

        let mut state = ParsecState::default();
        state.add_workspace(Workspace {
            ticket: "ABC-123".to_owned(),
            path: dir.path().join("../git-parsec.ABC-123"),
            branch: "feat/ABC-123".to_owned(),
            base_branch: "main".to_owned(),
            created_at: chrono::Utc.with_ymd_and_hms(2026, 6, 17, 0, 0, 0).unwrap(),
            ticket_title: Some("Wire MCP worktree status".to_owned()),
            status: WorkspaceStatus::Active,
            parent_ticket: None,
        });
        state.save(dir.path()).expect("state save");

        let ctx = McpContext {
            repo_path: dir.path().to_path_buf(),
            github_token: None,
            dry_run: false,
        };

        (dir, ctx)
    }

    #[test]
    fn worktree_list_returns_registered_workspaces() {
        let (_dir, ctx) = fixture_repo();

        let payload = list(&ctx, json!({})).expect("worktree list");

        assert_eq!(payload["count"], 1);
        assert_eq!(payload["worktrees"][0]["ticket"], "ABC-123");
        assert_eq!(payload["worktrees"][0]["branch"], "feat/ABC-123");
        assert_eq!(payload["pr_overlay"], false);
        assert_eq!(payload["ci_overlay"], false);
    }

    #[test]
    fn worktree_status_returns_matching_workspace() {
        let (_dir, ctx) = fixture_repo();

        let payload = status(&ctx, json!({"ticket": "ABC-123"})).expect("worktree status payload");

        assert_eq!(payload["workspace"]["ticket"], "ABC-123");
        assert_eq!(payload["workspace"]["base_branch"], "main");
        assert!(payload["pr_overlay"].is_null());
        assert!(payload["ci_overlay"].is_null());
    }

    #[test]
    fn worktree_status_requires_ticket() {
        let (_dir, ctx) = fixture_repo();

        let err = status(&ctx, json!({})).expect_err("missing ticket should fail");

        assert!(err.to_string().contains("requires a non-empty ticket"));
    }
}
