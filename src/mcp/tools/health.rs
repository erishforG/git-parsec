//! MCP tool handler for `health_check`.
//!
//! Phase 24 (#293): wires the synchronous health-check data (lock files,
//! uncommitted changes, stale branches) to the MCP dispatcher. CI-status
//! overlay (`include_ci`) is deferred to a later phase that can bridge the
//! async GitHub client.
//!
//! ## Output shape
//!
//! ```json
//! {
//!   "repo": "/abs/path",
//!   "count": 2,
//!   "records": [
//!     {
//!       "ticket": "PROJ-123",
//!       "healthy": true,
//!       "has_lock": false,
//!       "uncommitted": 0,
//!       "stale_days": 1,
//!       "stale_threshold_days": 7,
//!       "ci_status": null
//!     }
//!   ],
//!   "ci_overlay": false
//! }
//! ```

use anyhow::{Context, Result};
use serde_json::json;
use std::path::{Path, PathBuf};

use crate::config::ParsecConfig;
use crate::git;
use crate::mcp::McpContext;
use crate::worktree::WorktreeManager;

/// Default stale-branch threshold in days.
const DEFAULT_STALE_DAYS: i64 = 7;

fn repo_path(ctx: &McpContext, input: &serde_json::Value) -> PathBuf {
    input
        .get("repo")
        .and_then(serde_json::Value::as_str)
        .map(PathBuf::from)
        .unwrap_or_else(|| ctx.repo_path.clone())
}

/// Resolve the stale threshold from the caller's `stale_days` argument.
fn stale_threshold(input: &serde_json::Value) -> i64 {
    input
        .get("stale_days")
        .and_then(serde_json::Value::as_i64)
        .filter(|&d| d > 0)
        .unwrap_or(DEFAULT_STALE_DAYS)
}

/// Compute days-since-last-commit for a worktree path.
///
/// Returns `None` when git history is unreadable (empty repo, detached HEAD).
fn days_since_last_commit(path: &Path) -> Option<i64> {
    let ts = git::run_output(path, &["log", "-1", "--format=%ct"])
        .ok()
        .and_then(|s| s.trim().parse::<i64>().ok())?;
    let now = chrono::Utc::now().timestamp();
    Some((now - ts) / 86_400)
}

/// Check whether a worktree has an `.git/index.lock` file.
///
/// Handles both regular and worktree-style `.git` files (the `gitdir:` pointer
/// format used by `git worktree add`).
fn has_index_lock(worktree_path: &Path) -> bool {
    let git_entry = worktree_path.join(".git");
    let lock_path = if git_entry.is_file() {
        // Worktree-style: `.git` is a file containing "gitdir: <path>"
        std::fs::read_to_string(&git_entry)
            .ok()
            .and_then(|s| s.strip_prefix("gitdir: ").map(|p| PathBuf::from(p.trim())))
            .unwrap_or_else(|| git_entry.clone())
            .join("index.lock")
    } else {
        git_entry.join("index.lock")
    };
    lock_path.exists()
}

/// Build a single health record JSON value for one worktree.
fn health_record(ws: &crate::worktree::Workspace, threshold: i64) -> serde_json::Value {
    let has_lock = has_index_lock(&ws.path);
    let uncommitted = git::get_uncommitted_files(&ws.path)
        .unwrap_or_default()
        .len();
    let stale_days = days_since_last_commit(&ws.path);
    let is_stale = stale_days.is_some_and(|d| d > threshold);
    let healthy = !has_lock && uncommitted == 0 && !is_stale;

    json!({
        "ticket": ws.ticket,
        "healthy": healthy,
        "has_lock": has_lock,
        "uncommitted": uncommitted,
        "stale_days": stale_days,
        "stale_threshold_days": threshold,
        "ci_status": null,
    })
}

/// `health_check` — run worktree health diagnostics.
///
/// Checks every active worktree (or a single one when `ticket` is supplied)
/// for lock files, uncommitted changes, and stale branches. CI-status overlay
/// is not yet wired; `ci_overlay` is always `false` until Phase 25.
///
/// # Errors
/// Returns an error if the git repository cannot be read or the ticket does not
/// exist.
pub fn check(ctx: &McpContext, input: serde_json::Value) -> Result<serde_json::Value> {
    let config = ParsecConfig::load().context("failed to load parsec config")?;
    let repo = repo_path(ctx, &input);
    let manager = WorktreeManager::new(&repo, &config)?;
    let threshold = stale_threshold(&input);

    let workspaces = if let Some(ticket) = input
        .get("ticket")
        .and_then(serde_json::Value::as_str)
        .filter(|t| !t.is_empty())
    {
        crate::worktree::validate_ticket_id(ticket)?;
        vec![manager.get(ticket)?]
    } else {
        manager.list()?
    };

    let records: Vec<serde_json::Value> = workspaces
        .iter()
        .map(|ws| health_record(ws, threshold))
        .collect();

    Ok(json!({
        "repo": manager.repo_root().display().to_string(),
        "count": records.len(),
        "records": records,
        "ci_overlay": false,
    }))
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
            ticket: "PROJ-1".to_owned(),
            path: dir.path().join("../git-parsec.PROJ-1"),
            branch: "feat/PROJ-1".to_owned(),
            base_branch: "main".to_owned(),
            created_at: chrono::Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap(),
            ticket_title: Some("Health check MCP".to_owned()),
            status: WorkspaceStatus::Active,
            parent_ticket: None,
        });
        state.save(dir.path()).expect("state save");

        let ctx = McpContext {
            repo_path: dir.path().to_path_buf(),
            github_token: None,
            github_scopes: None,
            dry_run: false,
        };
        (dir, ctx)
    }

    #[test]
    fn health_check_returns_all_worktrees_when_no_ticket() {
        let (_dir, ctx) = fixture_repo();

        let payload = check(&ctx, json!({})).expect("health_check all");

        assert_eq!(payload["count"], 1);
        assert!(payload["records"].as_array().is_some());
        assert_eq!(payload["records"][0]["ticket"], "PROJ-1");
        assert_eq!(payload["ci_overlay"], false);
        assert!(payload["records"][0].get("ci_status").is_some());
    }

    #[test]
    fn health_check_returns_empty_when_no_worktrees() {
        let dir = tempfile::tempdir().expect("temp repo");
        crate::git::run(dir.path(), &["init"]).expect("git init");
        // No worktrees registered
        ParsecState::default().save(dir.path()).expect("state save");

        let ctx = McpContext {
            repo_path: dir.path().to_path_buf(),
            github_token: None,
            github_scopes: None,
            dry_run: false,
        };

        let payload = check(&ctx, json!({})).expect("health_check empty");

        assert_eq!(payload["count"], 0);
        assert_eq!(payload["records"].as_array().map(Vec::len), Some(0));
        assert_eq!(payload["ci_overlay"], false);
    }

    #[test]
    fn health_record_no_lock_no_uncommitted_is_healthy() {
        let (_dir, ctx) = fixture_repo();

        let payload = check(&ctx, json!({})).expect("health_check payload");
        let record = &payload["records"][0];

        // The worktree path doesn't exist as a real dir, so lock = false,
        // uncommitted = 0 (get_uncommitted_files fails safely).
        assert_eq!(record["has_lock"], false);
        assert_eq!(record["uncommitted"], 0);
        assert_eq!(record["stale_threshold_days"], DEFAULT_STALE_DAYS);
    }

    #[test]
    fn health_check_respects_custom_stale_days() {
        let (_dir, ctx) = fixture_repo();

        let payload = check(&ctx, json!({"stale_days": 30})).expect("health_check custom stale");

        assert_eq!(payload["records"][0]["stale_threshold_days"], 30);
    }

    #[test]
    fn health_check_requires_valid_ticket_when_supplied() {
        let (_dir, ctx) = fixture_repo();

        let err =
            check(&ctx, json!({"ticket": "bad ticket!"})).expect_err("invalid ticket should fail");

        assert!(err.to_string().contains("ticket") || err.to_string().contains("invalid"));
    }
}
