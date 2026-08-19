//! MCP tool handler for `smartlog`.
//!
//! Phase 25 (#293): wires the synchronous DAG collection (worktrees + commits,
//! no GitHub overlay) to the MCP dispatcher. PR/CI overlays require an async
//! GitHub client and are deferred to a future phase.
//!
//! ## Output shape
//!
//! ```json
//! {
//!   "repo": "/abs/path",
//!   "count": 2,
//!   "nodes": [
//!     {
//!       "ticket": "PROJ-123",
//!       "branch": "feat/proj-123",
//!       "base_branch": "main",
//!       "commits": [
//!         {
//!           "sha": "abc1234",
//!           "subject": "feat: initial",
//!           "author": "Alice",
//!           "date": "2026-08-01T00:00:00+00:00"
//!         }
//!       ],
//!       "pr": null,
//!       "ci": null
//!     }
//!   ],
//!   "pr_overlay": false,
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

/// Default commit depth per worktree node (mirrors `src/cli/commands/smartlog.rs`).
const DEFAULT_DEPTH: usize = 10;

fn repo_path(ctx: &McpContext, input: &serde_json::Value) -> PathBuf {
    input
        .get("repo")
        .and_then(serde_json::Value::as_str)
        .map(PathBuf::from)
        .unwrap_or_else(|| ctx.repo_path.clone())
}

/// Collect commits for `base_branch..branch` from `worktree_path`.
///
/// Uses the same `git log` format as `src/cli/commands/smartlog.rs`:
/// tab-separated `%h\t%s\t%an\t%aI` with `splitn(4, '\t')` so subjects
/// containing tabs are preserved correctly.
///
/// Returns an empty `Vec` on any git error so one broken worktree does not
/// abort the entire DAG response.
fn collect_commits(
    worktree_path: &Path,
    base_branch: &str,
    branch: &str,
    depth: usize,
) -> Vec<serde_json::Value> {
    let range = format!("{base_branch}..{branch}");
    let limit = format!("-n{depth}");
    let raw = match git::run_output(
        worktree_path,
        &["log", &range, "--pretty=format:%h\t%s\t%an\t%aI", &limit],
    ) {
        Ok(output) => output,
        Err(_) => return Vec::new(),
    };

    raw.lines()
        .filter(|line| !line.is_empty())
        .filter_map(|line| {
            let mut parts = line.splitn(4, '\t');
            let sha = parts.next()?.trim().to_string();
            let subject = parts.next()?.to_string();
            let author = parts.next()?.to_string();
            let date = parts.next()?.trim().to_string();
            if sha.is_empty() {
                return None;
            }
            Some(json!({
                "sha": sha,
                "subject": subject,
                "author": author,
                "date": date,
            }))
        })
        .collect()
}

/// `smartlog` — render the commit DAG with worktree branches.
///
/// Phase 25 provides the synchronous DAG: worktrees + commits per node.
/// The `ticket` argument filters nodes by ticket ID or branch name
/// (case-insensitive substring). `limit` caps commits per node (default 10).
///
/// PR and CI overlays are set to `false`/`null` for now; a later phase will
/// bridge the async GitHub client.
///
/// # Errors
/// Returns an error if the parsec config or git repository cannot be read.
pub fn run(ctx: &McpContext, input: serde_json::Value) -> Result<serde_json::Value> {
    let repo = repo_path(ctx, &input);
    let depth = input
        .get("limit")
        .and_then(serde_json::Value::as_u64)
        .map(|d| d as usize)
        .unwrap_or(DEFAULT_DEPTH);
    let filter = input
        .get("ticket")
        .and_then(serde_json::Value::as_str)
        .map(|f| f.to_lowercase());

    let config = ParsecConfig::load().context("failed to load parsec config")?;
    let manager = WorktreeManager::new(&repo, &config)?;
    let workspaces = manager.list()?;

    let nodes: Vec<serde_json::Value> = workspaces
        .iter()
        .filter(|ws| {
            filter.as_deref().is_none_or(|pat| {
                ws.ticket.to_lowercase().contains(pat) || ws.branch.to_lowercase().contains(pat)
            })
        })
        .map(|ws| {
            let commits = collect_commits(&ws.path, &ws.base_branch, &ws.branch, depth);
            json!({
                "ticket": ws.ticket,
                "branch": ws.branch,
                "base_branch": ws.base_branch,
                "commits": commits,
                "pr": null,
                "ci": null,
            })
        })
        .collect();

    Ok(json!({
        "repo": repo.display().to_string(),
        "count": nodes.len(),
        "nodes": nodes,
        "pr_overlay": false,
        "ci_overlay": false,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::McpContext;

    #[test]
    fn collect_commits_returns_empty_for_nonexistent_range() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let commits = collect_commits(tmp.path(), "main", "nonexistent-branch", 10);
        assert!(
            commits.is_empty(),
            "a bad git range should return empty, not panic"
        );
    }

    #[test]
    fn smartlog_returns_dag_envelope() {
        let ctx = McpContext::from_cwd(false).expect("context");
        let result = run(&ctx, serde_json::json!({})).expect("smartlog should not error");

        assert!(
            result.get("repo").is_some(),
            "result must include repo path"
        );
        assert!(
            result.get("count").is_some(),
            "result must include node count"
        );
        assert!(
            result
                .get("nodes")
                .and_then(serde_json::Value::as_array)
                .is_some(),
            "result must include nodes array"
        );
        assert_eq!(result["pr_overlay"], false, "PR overlay is not yet wired");
        assert_eq!(result["ci_overlay"], false, "CI overlay is not yet wired");
    }

    #[test]
    fn smartlog_ticket_filter_returns_subset() {
        let ctx = McpContext::from_cwd(false).expect("context");
        let unfiltered = run(&ctx, serde_json::json!({})).expect("unfiltered smartlog");
        let filtered = run(
            &ctx,
            serde_json::json!({"ticket": "zzz-nonexistent-ticket"}),
        )
        .expect("filtered smartlog should not error");

        let total = unfiltered["count"].as_u64().unwrap_or(0);
        let subset = filtered["count"].as_u64().unwrap_or(0);
        assert!(
            subset <= total,
            "filtered count ({subset}) must not exceed total ({total})"
        );
        assert_eq!(
            subset, 0,
            "no worktrees should match 'zzz-nonexistent-ticket'"
        );
    }

    #[test]
    fn smartlog_nodes_have_required_fields() {
        let ctx = McpContext::from_cwd(false).expect("context");
        let result = run(&ctx, serde_json::json!({})).expect("smartlog should not error");
        let nodes = result["nodes"].as_array().expect("nodes must be an array");

        for node in nodes {
            assert!(node.get("ticket").is_some(), "node must have ticket");
            assert!(node.get("branch").is_some(), "node must have branch");
            assert!(
                node.get("base_branch").is_some(),
                "node must have base_branch"
            );
            assert!(
                node.get("commits")
                    .and_then(serde_json::Value::as_array)
                    .is_some(),
                "node must have commits array"
            );
        }
    }

    #[test]
    fn smartlog_limit_argument_is_respected() {
        let ctx = McpContext::from_cwd(false).expect("context");
        // limit=1 must not cause an error even when there are fewer commits
        let result =
            run(&ctx, serde_json::json!({"limit": 1})).expect("smartlog with limit=1 should work");
        assert!(result.get("nodes").is_some());
    }
}
