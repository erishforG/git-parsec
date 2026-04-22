use std::path::Path;

use anyhow::{Context, Result};

use crate::config::ParsecConfig;
use crate::errors::ErrorCode;
use crate::git;
use crate::github;
use crate::output::{self, Mode};
use crate::worktree::WorktreeManager;

pub async fn ci(repo: &Path, tickets: &[&str], watch: bool, all: bool, mode: Mode) -> Result<()> {
    let config = ParsecConfig::load()?;
    let repo_root = git::get_main_repo_root(repo).or_else(|_| git::get_repo_root(repo))?;
    let remote_url = git::run_output(repo, &["remote", "get-url", "origin"])?;
    let gh = github::GitHubClient::new(&remote_url, &config)?
        .ok_or_else(|| anyhow::anyhow!("no GitHub token found. Set PARSEC_GITHUB_TOKEN."))?;
    let oplog = crate::oplog::OpLog::load(&repo_root)?;
    let manager = WorktreeManager::new(repo, &config)?;

    // Collect (ticket_id, pr_number) pairs to check
    let mut targets: Vec<(String, u64)> = Vec::new();

    if all {
        // All shipped entries with PR numbers from oplog
        let entries: Vec<_> = oplog
            .get_entries(None)
            .into_iter()
            .filter(|e| matches!(e.op, crate::oplog::OpKind::Ship))
            .filter_map(|e| {
                let url = e.detail.split(" -> ").nth(1)?;
                let number = url.rsplit('/').next()?.parse::<u64>().ok()?;
                Some((e.ticket.clone().unwrap_or_default(), number))
            })
            .collect();
        if entries.is_empty() {
            bail_code!(
                ErrorCode::E010,
                "no shipped PRs found. Ship a ticket first with `parsec ship`."
            );
        }
        targets = entries;
    } else if !tickets.is_empty() {
        // Multiple tickets specified
        for t in tickets {
            let ticket_id = t.to_string();
            // First check if there's a shipped PR in the oplog
            let shipped_pr = oplog
                .get_entries(Some(&ticket_id))
                .into_iter()
                .rev()
                .filter(|e| matches!(e.op, crate::oplog::OpKind::Ship))
                .find_map(|e| {
                    let url = e.detail.split(" -> ").nth(1)?;
                    url.rsplit('/').next()?.parse::<u64>().ok()
                });

            if let Some(pr_number) = shipped_pr {
                targets.push((ticket_id, pr_number));
            } else {
                // Not shipped yet — try to find an open PR by branch name
                let ws = manager.get(&ticket_id).with_context(|| {
                    format!("ticket {ticket_id} not found in active workspaces or oplog")
                })?;
                match gh.find_pr_by_branch(&ws.branch).await? {
                    Some(pr_number) => targets.push((ticket_id, pr_number)),
                    None => {
                        bail_code!(
                            ErrorCode::E010,
                            "no PR found for {ticket_id}. Push and create a PR first, or ship with `parsec ship {ticket_id}`."
                        );
                    }
                }
            }
        }
    } else {
        // Auto-detect from current worktree
        let cwd = std::env::current_dir()?;
        let all_ws = manager.list()?;
        let found = all_ws
            .into_iter()
            .find(|w| cwd.starts_with(&w.path))
            .ok_or_else(|| {
                anyhow::anyhow!("not inside a parsec worktree. Specify a ticket or use --all.")
            })?;
        let ticket_id = found.ticket;

        // First check if there's a shipped PR in the oplog
        let shipped_pr = oplog
            .get_entries(Some(&ticket_id))
            .into_iter()
            .rev()
            .filter(|e| matches!(e.op, crate::oplog::OpKind::Ship))
            .find_map(|e| {
                let url = e.detail.split(" -> ").nth(1)?;
                url.rsplit('/').next()?.parse::<u64>().ok()
            });

        if let Some(pr_number) = shipped_pr {
            targets.push((ticket_id, pr_number));
        } else {
            // Not shipped yet — try to find an open PR by branch name
            let ws = manager.get(&ticket_id).with_context(|| {
                format!("ticket {ticket_id} not found in active workspaces or oplog")
            })?;
            match gh.find_pr_by_branch(&ws.branch).await? {
                Some(pr_number) => targets.push((ticket_id, pr_number)),
                None => {
                    anyhow::bail!(
                        "no PR found for {ticket_id}. Push and create a PR first, or ship with `parsec ship {ticket_id}`."
                    );
                }
            }
        }
    }

    loop {
        let mut statuses: Vec<(String, crate::github::CiStatus)> = Vec::new();

        for (ticket_id, pr_number) in &targets {
            let ci = gh.get_check_runs(*pr_number).await?;
            statuses.push((ticket_id.clone(), ci));
        }

        // In watch + human mode, clear screen before redraw
        if watch && mode == Mode::Human {
            print!("\x1B[2J\x1B[H");
        }

        output::print_ci_status(&statuses, mode);

        if !watch || mode != Mode::Human {
            // JSON/quiet mode prints once even with --watch
            // Determine exit code based on overall status
            let has_failure = statuses.iter().any(|(_t, ci)| ci.overall == "failing");
            if has_failure {
                bail_code!(
                    ErrorCode::E002,
                    "CI checks failing for {} ticket(s)",
                    statuses
                        .iter()
                        .filter(|(_t, ci)| ci.overall == "failing")
                        .count()
                );
            }
            return Ok(());
        }

        // Check if all checks are completed
        let all_completed = statuses
            .iter()
            .all(|(_t, ci)| ci.checks.iter().all(|c| c.status == "completed"));

        if all_completed {
            let has_failure = statuses.iter().any(|(_t, ci)| ci.overall == "failing");
            if has_failure {
                bail_code!(
                    ErrorCode::E002,
                    "CI checks failing for {} ticket(s)",
                    statuses
                        .iter()
                        .filter(|(_t, ci)| ci.overall == "failing")
                        .count()
                );
            }
            return Ok(());
        }

        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }
}
