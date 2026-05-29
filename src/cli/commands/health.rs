//! `parsec health` — quick sanity-check for all active worktrees (#299, Phase 1).
//!
//! Iterates every active worktree and reports three lightweight indicators:
//!
//! | Indicator     | Signal                                              |
//! |---------------|-----------------------------------------------------|
//! | **lock**      | `.git/index.lock` exists → hung git process         |
//! | **uncommitted** | unstaged or staged files not yet committed        |
//! | **stale**     | last commit older than [`STALE_THRESHOLD_DAYS`] days |
//!
//! CI-status overlay is deferred to Phase 2 (depends on #309 / #310).
//! All checks are read-only; no worktree state is modified.

use std::path::Path;

use anyhow::Result;

use crate::config::ParsecConfig;
use crate::git;
use crate::output::{self, HealthRecord, Mode};
use crate::worktree::WorktreeManager;

/// Worktrees with no commit activity within this many days are flagged stale.
const STALE_THRESHOLD_DAYS: i64 = 7;

/// Run health checks for all active worktrees and print a summary.
///
/// Checks performed per worktree:
/// - `index.lock` presence (indicates an interrupted git operation).
/// - Uncommitted file count via `git diff --name-only` (staged + unstaged).
/// - Days since last commit via `git log -1 --format=%ct`.
///
/// Failures are non-fatal: a worktree whose git commands error out is still
/// included in the output with `None` for the affected field.
///
/// Returns `Ok(())` regardless of how many worktrees have issues so that the
/// exit code stays `0` (health is informational, not a CI gate in Phase 1).
pub async fn health(repo: &Path, mode: Mode) -> Result<()> {
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

    let mut records: Vec<HealthRecord> = Vec::new();

    for ws in &workspaces {
        // --- lock file -------------------------------------------------
        // Bare worktrees expose the git dir as a file that contains
        // `gitdir: <path>`.  Resolve the real git dir before checking.
        let git_dir = ws.path.join(".git");
        let lock_path = if git_dir.is_file() {
            // Linked worktree: read the `gitdir:` pointer
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
        let stale_days = git::run_output(&ws.path, &["log", "-1", "--format=%ct"])
            .ok()
            .and_then(|s| s.trim().parse::<i64>().ok())
            .map(|ts| {
                let now = chrono::Utc::now().timestamp();
                (now - ts) / 86_400
            });

        records.push(HealthRecord {
            ticket: ws.ticket.clone(),
            uncommitted,
            stale_days,
            stale_threshold_days: STALE_THRESHOLD_DAYS,
            has_lock,
        });
    }

    output::print_health(&records, mode);
    Ok(())
}
