//! `parsec reviews` — unified PR review status across all active worktrees (#301).
//!
//! Phase 1 (PR #331):
//! - Scan every active worktree via [`WorktreeManager`].
//! - For each worktree, resolve its open GitHub PR by branch name.
//! - Fetch the PR's review + CI status and collect into a [`ReviewEntry`].
//! - Render a table (human) or JSON array.
//!
//! Phase 2 (this PR):
//! - Add `--requested` flag: uses GitHub Search API
//!   (`/search/issues?q=review-requested:{login}`) to show PRs *from others*
//!   where the current user is a requested reviewer.
//! - Both views (author + requested) share the same `ReviewEntry` table output.

use std::path::Path;

use anyhow::Result;

use crate::config::ParsecConfig;
use crate::git;
use crate::github::GitHubClient;
use crate::output::{self, Mode, ReviewEntry};
use crate::worktree::WorktreeManager;

/// Entry point for the `parsec reviews` subcommand.
///
/// When `requested` is `false` (default): iterates all active worktrees,
/// resolves their associated open GitHub PRs, and prints the aggregated
/// review table (author view).
///
/// When `requested` is `true`: uses the GitHub Search API to find open PRs
/// *in this repo* where the authenticated user is a requested reviewer.
///
/// # Errors
/// Returns an error if GitHub credentials are missing. Individual per-worktree
/// failures (e.g. no PR for the branch) are silently skipped.
pub async fn reviews(repo: &Path, requested: bool, mode: Mode) -> Result<()> {
    let config = ParsecConfig::load()?;

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

    let entries = if requested {
        collect_requested_reviews(&gh).await?
    } else {
        collect_authored_reviews(repo, &config, &gh).await?
    };

    if entries.is_empty() {
        match mode {
            Mode::Human => {
                if requested {
                    println!("No open PRs where you are a requested reviewer.");
                } else {
                    println!("No open PRs found in active worktrees.");
                }
            }
            Mode::Json => println!("[]"),
            Mode::Quiet => {}
        }
        return Ok(());
    }

    output::print_reviews(&entries, mode);
    Ok(())
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Collect PRs authored by the user — one per active worktree (Phase 1 logic).
async fn collect_authored_reviews(
    repo: &Path,
    config: &ParsecConfig,
    gh: &GitHubClient,
) -> Result<Vec<ReviewEntry>> {
    let manager = WorktreeManager::new(repo, config)?;
    let workspaces = manager.list()?;
    let mut entries = Vec::new();

    for ws in &workspaces {
        let pr_number = match gh.find_pr_by_branch(&ws.branch).await {
            Ok(Some(n)) => n,
            Ok(None) | Err(_) => continue,
        };
        let status = match gh.get_pr_status(pr_number).await {
            Ok(s) => s,
            Err(_) => continue,
        };
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
    Ok(entries)
}

/// Collect PRs from *others* where the current user is a requested reviewer
/// (Phase 2 — uses GitHub Search API).
async fn collect_requested_reviews(gh: &GitHubClient) -> Result<Vec<ReviewEntry>> {
    let login = gh.get_authenticated_user().await?;
    let found = gh.search_review_requested_prs(&login).await?;

    let mut entries = Vec::new();
    for (pr_number, title, url, state) in found {
        // Fetch full PR status to get CI + review data.
        // Fall back to "–" on individual fetch failure rather than aborting.
        let (review_status, ci_status) = match gh.get_pr_status(pr_number).await {
            Ok(s) => (s.review_status, s.ci_status),
            Err(_) => ("–".to_string(), "–".to_string()),
        };

        // No worktree is associated with reviewer-mode PRs.
        entries.push(ReviewEntry {
            ticket: "–".to_string(),
            pr_number,
            title,
            state,
            review_status,
            ci_status,
            url,
        });
    }
    Ok(entries)
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

    #[test]
    fn review_entry_requested_mode_ticket_placeholder() {
        // In --requested mode, ticket is set to "–" because no worktree is associated.
        let e = mk_entry("–", 77, "pending", "success");
        assert_eq!(e.ticket, "–");
        assert_eq!(e.pr_number, 77);
    }
}
