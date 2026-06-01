//! `parsec reviews` — unified PR review status across all active worktrees (#301).
//!
//! Phase 1 (this PR):
//! - Scan every active worktree via [`WorktreeManager`].
//! - For each worktree, resolve its open GitHub PR by branch name.
//! - Fetch the PR's review + CI status and collect into a [`ReviewEntry`].
//! - Render a table (human) or JSON array.
//!
//! Scope is intentionally the "author" view: PRs that the current user opened
//! and that have a pending review request from others. Both pending and
//! approved/changes-requested states are shown so that nothing falls through.
//!
//! Phase 2 hint:
//! - Add `--requested` flag: use GitHub Search API
//!   (`/search/issues?q=review-requested:{login}`) to show PRs *from others*
//!   where the current user is a requested reviewer.
//! - Add `--all` to include closed/merged PRs.

use std::path::Path;

use anyhow::Result;

use crate::config::ParsecConfig;
use crate::git;
use crate::github::GitHubClient;
use crate::output::{self, Mode, ReviewEntry};
use crate::worktree::WorktreeManager;

/// Entry point for the `parsec reviews` subcommand.
///
/// Iterates all active worktrees, resolves their associated open GitHub PRs,
/// and prints the aggregated review table.
///
/// # Errors
/// Returns an error if GitHub credentials are missing. Individual per-worktree
/// failures (e.g. no PR for the branch) are silently skipped so that the rest
/// of the table still renders.
pub async fn reviews(repo: &Path, mode: Mode) -> Result<()> {
    let config = ParsecConfig::load()?;
    let manager = WorktreeManager::new(repo, &config)?;
    let workspaces = manager.list()?;

    if workspaces.is_empty() {
        match mode {
            Mode::Human => println!("No active worktrees — nothing to review."),
            Mode::Json => println!("[]"),
            Mode::Quiet => {}
        }
        return Ok(());
    }

    let remote_url = git::get_remote_url(repo).unwrap_or_default();
    let gh = match GitHubClient::new(&remote_url, &config)? {
        Some(c) => c,
        None => {
            anyhow::bail!(
                "no GitHub token found\n\
                 caused by: GITHUB_TOKEN not set and no token in parsec config\n\
                 help: run `gh auth login` or set GITHUB_TOKEN=<pat> in your environment"
            );
        }
    };

    let mut entries: Vec<ReviewEntry> = Vec::new();

    for ws in &workspaces {
        // Resolve PR number from branch name — skip worktrees without an open PR.
        let pr_number = match gh.find_pr_by_branch(&ws.branch).await {
            Ok(Some(n)) => n,
            Ok(None) => continue,
            Err(_) => continue,
        };

        // Fetch PR status (title, state, ci_status, review_status, url).
        let status = match gh.get_pr_status(pr_number).await {
            Ok(s) => s,
            Err(_) => continue,
        };

        // Phase 1 shows open + draft PRs only; closed/merged are filtered out.
        if status.state == "closed" {
            continue;
        }

        entries.push(ReviewEntry {
            ticket: ws.ticket.clone(),
            pr_number: status.number,
            title: status.title.clone(),
            state: status.state.clone(),
            review_status: status.review_status.clone(),
            ci_status: status.ci_status.clone(),
            url: status.url.clone(),
        });
    }

    output::print_reviews(&entries, mode);
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {

    use crate::output::ReviewEntry;

    fn mk_entry(ticket: &str, pr: u64, review: &str, ci: &str) -> ReviewEntry {
        ReviewEntry {
            ticket: ticket.to_string(),
            pr_number: pr,
            title: format!("feat: {ticket} title"),
            state: "open".to_string(),
            review_status: review.to_string(),
            ci_status: ci.to_string(),
            url: format!("https://github.com/owner/repo/pull/{pr}"),
        }
    }

    #[test]
    fn review_entry_fields() {
        let e = mk_entry("CL-100", 42, "approved", "success");
        assert_eq!(e.ticket, "CL-100");
        assert_eq!(e.pr_number, 42);
        assert_eq!(e.review_status, "approved");
        assert_eq!(e.ci_status, "success");
    }

    #[test]
    fn review_entry_pending_state() {
        let e = mk_entry("CL-200", 99, "pending", "pending");
        assert_eq!(e.state, "open");
        assert_eq!(e.review_status, "pending");
    }

    #[test]
    fn review_entry_changes_requested() {
        let e = mk_entry("CL-300", 55, "changes_requested", "failure");
        assert_eq!(e.review_status, "changes_requested");
        assert_eq!(e.ci_status, "failure");
    }
}
