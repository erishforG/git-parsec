//! `parsec health` — quick sanity-check for all active worktrees (#299).
//!
//! Iterates every active worktree and reports health indicators:
//!
//! | Indicator       | Signal                                              |
//! |-----------------|-----------------------------------------------------|
//! | **lock**        | `.git/index.lock` exists → hung git process         |
//! | **uncommitted** | unstaged or staged files not yet committed          |
//! | **stale**       | last commit older than `--stale-days` (default: 7) |
//! | **ci_status**   | Phase 2: CI overall status for open PR (best-effort)|
//!
//! Phase 2 additions:
//! - CI-status overlay via GitHub PR lookup (per-worktree branch).
//! - Configurable stale-threshold via `--stale-days` CLI flag.
//! - Opt-out via `--no-overlay` for fully offline mode.
//!
//! All checks are read-only; no worktree state is modified.

use std::path::Path;

use anyhow::Result;

use crate::config::ParsecConfig;
use crate::git;
use crate::github::GitHubClient;
use crate::output::{self, HealthRecord, Mode};
use crate::worktree::WorktreeManager;

/// Run health checks for all active worktrees and print a summary.
///
/// # Arguments
/// * `repo`       — path to the repository root
/// * `mode`       — output mode (human / json / quiet)
/// * `stale_days` — number of days before a branch is flagged stale
/// * `no_overlay` — when `true`, skip GitHub CI-status lookup entirely
///
/// Phase 2 extends Phase 1 with:
/// - Per-worktree GitHub PR lookup via branch name.
/// - CI overall status fetched from the check-runs endpoint.
/// - Graceful degradation: missing token / no PR / network errors leave
///   `ci_status` as `None` without failing the command.
///
/// Returns `Ok(())` regardless of how many worktrees have issues so that the
/// exit code stays `0` (health is informational, not a CI gate).
pub async fn health(repo: &Path, mode: Mode, stale_days: u64, no_overlay: bool) -> Result<()> {
    let config = ParsecConfig::load()?;
    let manager = WorktreeManager::new(repo, &config)?;
    let workspaces = manager.list()?;

    if workspaces.is_empty() {
        if mode == Mode::Human {
            println!("No active worktrees.");
        } else if mode == Mode::Json {
            println!("[]");
        }
        return Ok(());
    }

    let stale_threshold = stale_days as i64;

    // Resolve GitHub client once for all worktrees (best-effort).
    // Failure to build a client is non-fatal; overlay is simply skipped.
    let gh_client: Option<GitHubClient> = if no_overlay {
        None
    } else {
        let remote_url = git::get_remote_url(repo).unwrap_or_default();
        GitHubClient::new(&remote_url, &config).unwrap_or(None)
    };

    let mut records: Vec<HealthRecord> = Vec::new();

    for ws in &workspaces {
        // --- lock file -------------------------------------------------
        let git_dir = ws.path.join(".git");
        let lock_path = if git_dir.is_file() {
            std::fs::read_to_string(&git_dir)
                .ok()
                .and_then(|s| {
                    s.strip_prefix("gitdir: ")
                        .map(|p| std::path::PathBuf::from(p.trim()))
                })
                .unwrap_or_else(|| git_dir.clone())
                .join("index.lock")
        } else {
            git_dir.join("index.lock")
        };
        let has_lock = lock_path.exists();

        // --- uncommitted -----------------------------------------------
        let uncommitted = git::get_uncommitted_files(&ws.path)
            .unwrap_or_default()
            .len();

        // --- stale (days since last commit) ----------------------------
        let stale_days_val = git::run_output(&ws.path, &["log", "-1", "--format=%ct"])
            .ok()
            .and_then(|s| s.trim().parse::<i64>().ok())
            .map(|ts| {
                let now = chrono::Utc::now().timestamp();
                (now - ts) / 86_400
            });

        // --- CI status overlay (Phase 2) --------------------------------
        let (ci_status, pr_number) = fetch_ci_overlay(&gh_client, &ws.branch).await;

        records.push(HealthRecord {
            ticket: ws.ticket.clone(),
            uncommitted,
            stale_days: stale_days_val,
            stale_threshold_days: stale_threshold,
            has_lock,
            ci_status,
            pr_number,
        });
    }

    output::print_health(&records, mode);
    Ok(())
}

/// Resolve CI status for a worktree branch via the GitHub client.
///
/// Returns `(ci_status, pr_number)`. Both are `None` when:
/// - `client` is `None` (no token or `--no-overlay`),
/// - no open PR exists for `branch`, or
/// - any network / API error occurs.
///
/// Errors are swallowed so the overall health command stays non-fatal.
async fn fetch_ci_overlay(
    client: &Option<GitHubClient>,
    branch: &str,
) -> (Option<String>, Option<u64>) {
    let client = match client {
        Some(c) => c,
        None => return (None, None),
    };

    let pr_num = match client.find_pr_by_branch(branch).await {
        Ok(Some(n)) => n,
        Ok(None) => return (None, None),
        Err(e) => {
            eprintln!("health: PR lookup failed for {}: {}", branch, e);
            return (None, None);
        }
    };

    match client.get_pr_status(pr_num).await {
        Ok(status) => (Some(status.ci_status), Some(pr_num)),
        Err(e) => {
            eprintln!("health: CI status fetch failed for PR #{}: {}", pr_num, e);
            (None, Some(pr_num))
        }
    }
}
