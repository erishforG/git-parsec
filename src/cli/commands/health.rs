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
//! Phase 3 additions:
//! - Detect in-progress git operations: rebase, merge, cherry-pick.
//!   Checked via git-dir state files (`rebase-merge/`, `MERGE_HEAD`, etc.).
//!   Works for both main and linked worktrees (resolves `gitdir:` pointer).
//!   Failures are soft: detection errors return `false` rather than aborting.
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
        // --- resolve effective git directory ---------------------------
        // For linked worktrees, `.git` is a text file: `gitdir: <path>`.
        // All per-worktree state files live under that resolved path.
        let effective_git_dir = resolve_git_dir(&ws.path);

        // --- lock file -------------------------------------------------
        let has_lock = effective_git_dir.join("index.lock").exists();

        // --- in-progress git operations (Phase 3) ----------------------
        let rebase_in_progress = effective_git_dir.join("rebase-merge").is_dir()
            || effective_git_dir.join("rebase-apply").is_dir();
        let merge_in_progress = effective_git_dir.join("MERGE_HEAD").exists();
        let cherry_pick_in_progress = effective_git_dir.join("CHERRY_PICK_HEAD").exists();

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
            rebase_in_progress,
            merge_in_progress,
            cherry_pick_in_progress,
        });
    }

    output::print_health(&records, mode);
    Ok(())
}

/// Resolve the effective git directory for a worktree path.
///
/// For a linked worktree, `.git` is a text file containing
/// `gitdir: <absolute-path>`. For the main worktree, `.git` is a directory.
/// Returns the resolved path, or `<ws_path>/.git` as a fallback.
fn resolve_git_dir(ws_path: &Path) -> std::path::PathBuf {
    let dot_git = ws_path.join(".git");
    if dot_git.is_file() {
        std::fs::read_to_string(&dot_git)
            .ok()
            .and_then(|s| {
                s.strip_prefix("gitdir: ")
                    .map(|p| std::path::PathBuf::from(p.trim()))
            })
            .unwrap_or(dot_git)
    } else {
        dot_git
    }
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Create a fake main-worktree layout: `<dir>/.git/` is a directory.
    fn make_main_git_dir(root: &TempDir) -> std::path::PathBuf {
        let git_dir = root.path().join(".git");
        fs::create_dir_all(&git_dir).unwrap();
        git_dir
    }

    /// Create a fake linked-worktree layout:
    /// `<root>/.git` is a file pointing to `<target>`.
    fn make_linked_git_dir(root: &TempDir, target: &std::path::Path) {
        let dot_git = root.path().join(".git");
        fs::write(&dot_git, format!("gitdir: {}\n", target.display())).unwrap();
    }

    #[test]
    fn resolve_git_dir_main_worktree() {
        let tmp = TempDir::new().unwrap();
        let git_dir = make_main_git_dir(&tmp);
        let resolved = resolve_git_dir(tmp.path());
        assert_eq!(resolved, git_dir, "main worktree: resolved path = .git dir");
    }

    #[test]
    fn resolve_git_dir_linked_worktree() {
        let main_tmp = TempDir::new().unwrap();
        let linked_tmp = TempDir::new().unwrap();
        let target = main_tmp.path().join(".git").join("worktrees").join("feat");
        fs::create_dir_all(&target).unwrap();
        make_linked_git_dir(&linked_tmp, &target);

        let resolved = resolve_git_dir(linked_tmp.path());
        assert_eq!(
            resolved, target,
            "linked worktree: resolved path = gitdir target"
        );
    }

    #[test]
    fn detects_rebase_merge_dir() {
        let tmp = TempDir::new().unwrap();
        let git_dir = make_main_git_dir(&tmp);
        fs::create_dir_all(git_dir.join("rebase-merge")).unwrap();

        let effective = resolve_git_dir(tmp.path());
        assert!(effective.join("rebase-merge").is_dir());
        assert!(effective.join("rebase-merge").is_dir() || effective.join("rebase-apply").is_dir());
    }

    #[test]
    fn detects_rebase_apply_dir() {
        let tmp = TempDir::new().unwrap();
        let git_dir = make_main_git_dir(&tmp);
        fs::create_dir_all(git_dir.join("rebase-apply")).unwrap();

        let effective = resolve_git_dir(tmp.path());
        let rebase_in_progress =
            effective.join("rebase-merge").is_dir() || effective.join("rebase-apply").is_dir();
        assert!(
            rebase_in_progress,
            "rebase-apply dir should trigger rebase_in_progress"
        );
    }

    #[test]
    fn detects_merge_head() {
        let tmp = TempDir::new().unwrap();
        let git_dir = make_main_git_dir(&tmp);
        fs::write(git_dir.join("MERGE_HEAD"), "abc123\n").unwrap();

        let effective = resolve_git_dir(tmp.path());
        assert!(effective.join("MERGE_HEAD").exists());
    }

    #[test]
    fn detects_cherry_pick_head() {
        let tmp = TempDir::new().unwrap();
        let git_dir = make_main_git_dir(&tmp);
        fs::write(git_dir.join("CHERRY_PICK_HEAD"), "abc123\n").unwrap();

        let effective = resolve_git_dir(tmp.path());
        assert!(effective.join("CHERRY_PICK_HEAD").exists());
    }

    #[test]
    fn no_false_positives_when_clean() {
        let tmp = TempDir::new().unwrap();
        make_main_git_dir(&tmp);

        let effective = resolve_git_dir(tmp.path());
        assert!(!effective.join("rebase-merge").is_dir());
        assert!(!effective.join("rebase-apply").is_dir());
        assert!(!effective.join("MERGE_HEAD").exists());
        assert!(!effective.join("CHERRY_PICK_HEAD").exists());
        assert!(!effective.join("index.lock").exists());
    }
}
