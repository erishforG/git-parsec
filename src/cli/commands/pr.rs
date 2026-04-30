use std::path::Path;

use anyhow::{bail, Context, Result};
use colored::Colorize;

use crate::bitbucket;
use crate::config::ParsecConfig;
use crate::errors::ErrorCode;
use crate::git;
use crate::github;
use crate::output::{self, Mode};
use crate::tracker;
use crate::worktree::WorktreeManager;

pub async fn open(
    repo: &Path,
    ticket: &str,
    force_pr: bool,
    force_ticket: bool,
    mode: Mode,
) -> Result<()> {
    let mut config = ParsecConfig::load()?;

    // Try to find PR URL from oplog
    let repo_root = git::get_main_repo_root(repo).or_else(|_| git::get_repo_root(repo))?;
    config.resolve_for_repo(&repo_root);
    let oplog = crate::oplog::OpLog::load(&repo_root)?;
    let pr_url: Option<String> = oplog.get_entries(Some(ticket)).iter().rev().find_map(|e| {
        if matches!(e.op, crate::oplog::OpKind::Ship) {
            // PR URL is after " -> " in the detail string
            e.detail.split(" -> ").nth(1).map(|s| s.to_string())
        } else {
            None
        }
    });

    // Try to construct ticket tracker URL
    use crate::config::TrackerProvider;
    let ticket_url = match config.tracker.provider {
        TrackerProvider::Jira => config
            .tracker
            .jira
            .as_ref()
            .map(|j| format!("{}/browse/{}", j.base_url.trim_end_matches('/'), ticket)),
        TrackerProvider::Github => git::run_output(repo, &["remote", "get-url", "origin"])
            .ok()
            .and_then(|url| {
                let remote = github::parse_github_remote(url.trim())?;
                let base = format!("https://{}/{}/{}", remote.host, remote.owner, remote.repo);
                Some(format!(
                    "{}/issues/{}",
                    base,
                    ticket.trim_start_matches('#')
                ))
            }),
        TrackerProvider::Gitlab => config.tracker.gitlab.as_ref().map(|g| {
            let base = g.base_url.trim_end_matches('/');
            git::run_output(repo, &["remote", "get-url", "origin"])
                .ok()
                .and_then(|url| {
                    let path = url
                        .trim_end_matches(".git")
                        .rsplit_once("gitlab.com")
                        .map(|(_, p)| p.trim_start_matches([':', '/']))?;
                    Some(format!("{}/{}/-/issues/{}", base, path, ticket))
                })
                .unwrap_or_else(|| format!("{}/-/issues/{}", base, ticket))
        }),
        TrackerProvider::None => None,
    };

    // Decide which URL to open
    let url = if force_pr {
        pr_url.ok_or_else(|| anyhow::anyhow!("no PR found for ticket {ticket}. Ship it first."))?
    } else if force_ticket {
        ticket_url
            .ok_or_else(|| anyhow::anyhow!("no ticket URL for {ticket}. Configure a tracker."))?
    } else {
        // Default: PR if available, otherwise ticket
        pr_url.or(ticket_url).ok_or_else(|| {
            anyhow::anyhow!("no URL found for {ticket}. Ship it or configure a tracker.")
        })?
    };

    // Open in browser
    #[cfg(target_os = "macos")]
    let open_cmd = "open";
    #[cfg(not(target_os = "macos"))]
    let open_cmd = "xdg-open";

    std::process::Command::new(open_cmd)
        .arg(&url)
        .spawn()
        .with_context(|| format!("failed to open browser with {open_cmd}"))?;

    if mode == Mode::Json {
        let value = serde_json::json!({ "action": "open", "ticket": ticket, "url": url });
        println!("{}", value);
    } else if mode != Mode::Quiet {
        println!("Opening {}", url);
    }

    Ok(())
}

pub async fn pr_status(repo: &Path, ticket: Option<&str>, mode: Mode) -> Result<()> {
    let config = ParsecConfig::load()?;
    let repo_root = git::get_main_repo_root(repo).or_else(|_| git::get_repo_root(repo))?;
    let oplog = crate::oplog::OpLog::load(&repo_root)?;
    let remote_url = git::run_output(repo, &["remote", "get-url", "origin"])?;

    // Find shipped entries with PR URLs
    let entries: Vec<_> = oplog
        .get_entries(ticket)
        .into_iter()
        .filter(|e| matches!(e.op, crate::oplog::OpKind::Ship))
        .filter_map(|e| {
            let url = e.detail.split(" -> ").nth(1)?;
            // Extract PR number from URL (e.g. .../pull/42)
            let number = url.rsplit('/').next()?.parse::<u64>().ok()?;
            Some((
                e.ticket.clone().unwrap_or_default(),
                number,
                url.to_string(),
            ))
        })
        .collect();

    // Fallback: search active workspaces for PRs by branch name
    let mut all_entries = entries;
    if all_entries.is_empty() {
        let manager = WorktreeManager::new(repo, &config)?;
        let workspaces = match ticket {
            Some(t) => vec![manager.get(t)?],
            None => manager.list()?,
        };

        if let Some(gh) = github::GitHubClient::new(&remote_url, &config)? {
            for ws in &workspaces {
                if let Ok(Some(pr_number)) = gh.find_pr_by_branch(&ws.branch).await {
                    all_entries.push((ws.ticket.clone(), pr_number, String::new()));
                }
            }
        } else if let Some(bb) = bitbucket::BitbucketClient::new(&remote_url)? {
            for ws in &workspaces {
                if let Ok(Some(pr_id)) = bb.find_pr_by_branch(&ws.branch).await {
                    all_entries.push((ws.ticket.clone(), pr_id, String::new()));
                }
            }
        }

        if all_entries.is_empty() {
            if let Some(t) = ticket {
                bail_code!(ErrorCode::E010, "no PR found for {t}. Ship it first with `parsec ship {t}`, or check your forge token.");
            } else {
                bail_code!(ErrorCode::E010, "no PRs found. Ship a ticket first with `parsec ship`, or check your forge token.");
            }
        }
    }

    // Try GitHub first, then Bitbucket
    let mut statuses = Vec::new();
    if let Some(gh) = github::GitHubClient::new(&remote_url, &config)? {
        for (ticket_id, pr_number, _url) in &all_entries {
            let status = gh.get_pr_status(*pr_number).await?;
            statuses.push((ticket_id.clone(), status));
        }
    } else if let Some(bb) = bitbucket::BitbucketClient::new(&remote_url)? {
        for (ticket_id, pr_id, _url) in &all_entries {
            let bb_status = bb.get_pr_status(*pr_id).await?;
            // Map to github::PrStatus for output compatibility
            statuses.push((
                ticket_id.clone(),
                github::PrStatus {
                    number: bb_status.id,
                    title: bb_status.title,
                    state: bb_status.state.to_lowercase(),
                    mergeable: None,
                    ci_status: "unknown".to_string(),
                    review_status: "unknown".to_string(),
                    url: bb_status.url,
                },
            ));
        }
    } else {
        bail_code!(
            ErrorCode::E001,
            "no forge token found. Set PARSEC_GITHUB_TOKEN or PARSEC_BITBUCKET_TOKEN."
        );
    }

    output::print_pr_status(&statuses, mode);
    Ok(())
}

pub async fn merge(
    repo: &Path,
    ticket: Option<&str>,
    rebase: bool,
    no_wait: bool,
    no_delete_branch: bool,
    mode: Mode,
) -> Result<()> {
    let config = ParsecConfig::load()?;
    let repo_root = git::get_main_repo_root(repo).or_else(|_| git::get_repo_root(repo))?;
    let remote_url = git::run_output(repo, &["remote", "get-url", "origin"])?;
    let oplog = crate::oplog::OpLog::load(&repo_root)?;
    let manager = WorktreeManager::new(repo, &config)?;

    // Detect forge: GitHub or Bitbucket
    let has_github = github::GitHubClient::new(&remote_url, &config)?.is_some();
    let has_bitbucket = !has_github && bitbucket::BitbucketClient::new(&remote_url)?.is_some();

    if !has_github && !has_bitbucket {
        bail_code!(
            ErrorCode::E001,
            "no forge token found. Set PARSEC_GITHUB_TOKEN or PARSEC_BITBUCKET_TOKEN."
        );
    }

    // Resolve ticket
    let ticket_id = if let Some(t) = ticket {
        t.to_string()
    } else {
        let cwd = std::env::current_dir()?;
        let all_ws = manager.list()?;
        let found = all_ws
            .into_iter()
            .find(|w| cwd.starts_with(&w.path))
            .ok_or_else(|| anyhow::anyhow!("not inside a parsec worktree. Specify a ticket."))?;
        found.ticket
    };

    // Find PR number: check oplog first, then try open PR by branch
    let pr_number = {
        let shipped_pr = oplog
            .get_entries(Some(&ticket_id))
            .into_iter()
            .rev()
            .filter(|e| matches!(e.op, crate::oplog::OpKind::Ship))
            .find_map(|e| {
                let url = e.detail.split(" -> ").nth(1)?;
                url.rsplit('/').next()?.parse::<u64>().ok()
            });

        if let Some(pr) = shipped_pr {
            pr
        } else {
            let ws = manager.get(&ticket_id).with_context(|| {
                format!("ticket {ticket_id} not found in active workspaces or oplog")
            })?;
            if has_github {
                let gh = github::GitHubClient::new(&remote_url, &config)?.unwrap();
                gh.find_pr_by_branch(&ws.branch).await?.ok_or_else(|| {
                    anyhow::anyhow!(
                        "no open PR found for {ticket_id} (branch '{}'). Ship it first.",
                        ws.branch
                    )
                })?
            } else {
                let bb = bitbucket::BitbucketClient::new(&remote_url)?.unwrap();
                bb.find_pr_by_branch(&ws.branch).await?.ok_or_else(|| {
                    anyhow::anyhow!(
                        "no open PR found for {ticket_id} (branch '{}'). Ship it first.",
                        ws.branch
                    )
                })?
            }
        }
    };

    // Bitbucket merge path
    if has_bitbucket {
        let bb = bitbucket::BitbucketClient::new(&remote_url)?.unwrap();
        let method = if rebase { "rebase" } else { "squash" };
        match bb.merge_pr(pr_number, method).await {
            Ok(mr) => {
                if mode == Mode::Human {
                    println!("Merged PR #{} ({})", pr_number, mr.message);
                } else if mode == Mode::Json {
                    println!(
                        "{}",
                        serde_json::json!({
                            "ticket": ticket_id,
                            "pr_number": pr_number,
                            "merged": mr.merged,
                            "method": method,
                        })
                    );
                }
            }
            Err(e) => {
                bail!("Bitbucket merge failed: {e}");
            }
        }

        // Auto-transition ticket status
        if let Some(ref auto) = config.tracker.auto_transition {
            if let Some(ref status) = auto.on_merge {
                tracker::try_transition(&config, &ticket_id, status).await;
            }
        }

        if let Err(e) = crate::oplog::record(
            &repo_root,
            crate::oplog::OpKind::Clean,
            Some(&ticket_id),
            &format!("Merged PR #{} ({})", pr_number, method),
            None,
        ) {
            eprintln!("warning: failed to write oplog: {e}");
        }

        return Ok(());
    }

    // GitHub merge path
    let gh = github::GitHubClient::new(&remote_url, &config)?.unwrap();

    // Idempotency: check if PR is already merged/closed
    if let Ok(status) = gh.get_pr_status(pr_number).await {
        if status.state == "closed" {
            if mode == Mode::Human {
                eprintln!(
                    "PR #{} is already closed/merged. Skipping merge.",
                    pr_number
                );
            }
            // Still do cleanup if needed
            if config.ship.auto_cleanup {
                if let Ok(ws) = manager.get(&ticket_id) {
                    if ws.path.exists() {
                        if let Err(e) = git::worktree_remove(&repo_root, &ws.path) {
                            eprintln!("warning: failed to remove worktree: {e}");
                        }
                    }
                    let mut state = crate::worktree::ParsecState::load(&repo_root)?;
                    state.remove_workspace(&ticket_id);
                    state.save(&repo_root)?;
                }
            }
            return Ok(());
        }
    }

    // Wait for CI to pass (unless --no-wait)
    if !no_wait {
        if mode == Mode::Human {
            eprint!("Waiting for CI to pass...");
        }
        loop {
            let ci = gh.get_check_runs(pr_number).await?;
            if ci.overall == "passing" {
                if mode == Mode::Human {
                    eprintln!(" {}", "✓".green());
                }
                break;
            } else if ci.overall == "failing" {
                if mode == Mode::Human {
                    eprintln!(" {}", "✗".red());
                }
                bail_code!(
                    ErrorCode::E002,
                    "CI is failing for PR #{}. Fix CI or use --no-wait to merge anyway.",
                    pr_number
                );
            }
            // Still pending — wait and retry
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    }

    // Determine merge method
    let method = if rebase { "rebase" } else { "squash" };
    let delete_branch = !no_delete_branch;

    // Try merging, auto-update branch if not mergeable
    let result = match gh.merge_pr(pr_number, method, delete_branch).await {
        Ok(result) => result,
        Err(e) if e.to_string().starts_with("not mergeable") => {
            // PR is behind base branch — try updating
            if mode == Mode::Human {
                eprintln!("PR #{} is not mergeable. Updating branch...", pr_number);
            }
            match gh.update_pr_branch(pr_number).await? {
                true => {
                    if mode == Mode::Human {
                        eprintln!("Branch updated. Waiting for CI...");
                    }
                    // Wait for CI to pass again after update
                    loop {
                        let ci = gh.get_check_runs(pr_number).await?;
                        if ci.overall == "passing" {
                            break;
                        } else if ci.overall == "failing" {
                            bail_code!(
                                ErrorCode::E002,
                                "CI is failing after branch update for PR #{}.",
                                pr_number
                            );
                        }
                        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    }
                    // Retry merge
                    gh.merge_pr(pr_number, method, delete_branch).await?
                }
                false => {
                    bail_code!(
                        ErrorCode::E003,
                        "PR #{} has conflicts with the base branch. Resolve conflicts manually and retry.",
                        pr_number
                    );
                }
            }
        }
        Err(e) => return Err(e),
    };

    output::print_merge(&ticket_id, pr_number, &result, method, mode);

    // Prune stale remote-tracking references after remote branch deletion
    if delete_branch {
        if let Err(e) = git::fetch(&repo_root) {
            eprintln!("warning: failed to prune remote-tracking references: {e}");
        }
    }

    // Auto-transition ticket status
    if let Some(ref auto) = config.tracker.auto_transition {
        if let Some(ref status) = auto.on_merge {
            tracker::try_transition(&config, &ticket_id, status).await;
        }
    }

    // Auto-close GitHub issue if ticket looks like a number
    let issue_number = ticket_id
        .strip_prefix('#')
        .unwrap_or(&ticket_id)
        .parse::<u64>()
        .ok();

    if let Some(issue_num) = issue_number {
        match gh.close_issue(issue_num).await {
            Ok(true) => {
                if mode == Mode::Human {
                    println!("  Closed issue #{}", issue_num);
                }
            }
            Ok(false) => {} // no token or already closed, skip silently
            Err(e) => {
                eprintln!("warning: failed to close issue #{}: {}", issue_num, e);
            }
        }
    }

    // Clean up local worktree if it exists and auto_cleanup is enabled
    if config.ship.auto_cleanup {
        if let Ok(ws) = manager.get(&ticket_id) {
            if ws.path.exists() {
                if let Err(e) = git::worktree_remove(&repo_root, &ws.path) {
                    eprintln!("warning: failed to remove worktree: {e}");
                }
            }
            // Update state
            let mut state = crate::worktree::ParsecState::load(&repo_root)?;
            state.remove_workspace(&ticket_id);
            state.save(&repo_root)?;

            // Delete local branch
            if let Some(branch) = &oplog
                .get_entries(Some(&ticket_id))
                .last()
                .and_then(|e| e.undo_info.as_ref())
                .and_then(|u| u.branch.clone())
            {
                if let Err(e) = git::delete_branch(&repo_root, branch) {
                    eprintln!("warning: failed to delete local branch '{}': {}", branch, e);
                }
            }

            if mode == Mode::Human {
                println!("  {}", "Local worktree cleaned up.".dimmed());
            }
        }
    }

    // Record in oplog
    if let Err(e) = crate::oplog::record(
        &repo_root,
        crate::oplog::OpKind::Clean,
        Some(&ticket_id),
        &format!("Merged PR #{} ({})", pr_number, method),
        None,
    ) {
        eprintln!("warning: failed to write oplog: {e}");
    }

    Ok(())
}

/// Batch merge multiple PRs sequentially.
pub async fn merge_batch(
    repo: &Path,
    tickets: &[&str],
    rebase: bool,
    no_wait: bool,
    no_delete_branch: bool,
    mode: Mode,
) -> Result<()> {
    if mode == Mode::Human {
        println!("Batch merging {} tickets...\n", tickets.len());
    }

    let mut failed: Vec<(String, String)> = Vec::new();
    let mut succeeded: Vec<String> = Vec::new();

    for (i, ticket) in tickets.iter().enumerate() {
        if mode == Mode::Human {
            println!("[{}/{}] Merging {}...", i + 1, tickets.len(), ticket);
        }

        match merge(repo, Some(ticket), rebase, no_wait, no_delete_branch, mode).await {
            Ok(()) => {
                succeeded.push(ticket.to_string());
            }
            Err(e) => {
                let err_msg = format!("{e}");
                if mode == Mode::Human {
                    eprintln!("  Failed: {}", err_msg);
                }
                failed.push((ticket.to_string(), err_msg));
            }
        }
    }

    if mode == Mode::Human {
        println!();
        println!(
            "Batch merge complete: {} succeeded, {} failed",
            succeeded.len(),
            failed.len()
        );
        for (ticket, err) in &failed {
            println!("  x {}: {}", ticket, err);
        }
    } else if mode == Mode::Json {
        println!(
            "{}",
            serde_json::json!({
                "succeeded": succeeded,
                "failed": failed.iter().map(|(t, e)| serde_json::json!({"ticket": t, "error": e})).collect::<Vec<_>>(),
            })
        );
    }

    if !failed.is_empty() {
        bail_code!(ErrorCode::E004, "{} merge(s) failed", failed.len());
    }

    Ok(())
}
