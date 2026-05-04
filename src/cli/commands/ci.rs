use std::path::Path;

use anyhow::{Context, Result};

use crate::bitbucket;
use crate::config::ParsecConfig;
use crate::errors::ErrorCode;
use crate::git;
use crate::github;
use crate::output::{self, Mode};
use crate::worktree::WorktreeManager;

/// Forge backend selected for `parsec ci` based on the origin remote URL.
enum Forge {
    GitHub(github::GitHubClient),
    Bitbucket(bitbucket::BitbucketClient),
}

pub async fn ci(repo: &Path, tickets: &[&str], watch: bool, all: bool, mode: Mode) -> Result<()> {
    let config = ParsecConfig::load()?;
    let repo_root = git::get_main_repo_root(repo).or_else(|_| git::get_repo_root(repo))?;
    let remote_url = git::run_output(repo, &["remote", "get-url", "origin"])?;

    // Dispatch on remote type — GitHub takes priority when both tokens exist.
    let forge = if let Some(gh) = github::GitHubClient::new(&remote_url, &config)? {
        Forge::GitHub(gh)
    } else if let Some(bb) = bitbucket::BitbucketClient::new(&remote_url)? {
        Forge::Bitbucket(bb)
    } else {
        bail_code!(
            ErrorCode::E001,
            "no forge token found. Set PARSEC_GITHUB_TOKEN or PARSEC_BITBUCKET_TOKEN."
        );
    };

    let oplog = crate::oplog::OpLog::load(&repo_root)?;
    let manager = WorktreeManager::new(repo, &config)?;

    // Collect (ticket_id, pr_number) pairs to check. Bitbucket "PR id" and
    // GitHub "PR number" share the same numeric encoding in the oplog (last
    // path segment of the URL), so the resolution logic is forge-agnostic.
    let mut targets: Vec<(String, u64)> = Vec::new();

    if all {
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
        for t in tickets {
            let ticket_id = t.to_string();
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
                let ws = manager.get(&ticket_id).with_context(|| {
                    format!("ticket {ticket_id} not found in active workspaces or oplog")
                })?;
                let found = match &forge {
                    Forge::GitHub(gh) => gh.find_pr_by_branch(&ws.branch).await?,
                    Forge::Bitbucket(bb) => bb.find_pr_by_branch(&ws.branch).await?,
                };
                match found {
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
            let ws = manager.get(&ticket_id).with_context(|| {
                format!("ticket {ticket_id} not found in active workspaces or oplog")
            })?;
            let pr_lookup = match &forge {
                Forge::GitHub(gh) => gh.find_pr_by_branch(&ws.branch).await?,
                Forge::Bitbucket(bb) => bb.find_pr_by_branch(&ws.branch).await?,
            };
            match pr_lookup {
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
            let ci = match &forge {
                Forge::GitHub(gh) => gh.get_check_runs(*pr_number).await?,
                Forge::Bitbucket(bb) => fetch_bitbucket_ci(bb, *pr_number).await?,
            };
            statuses.push((ticket_id.clone(), ci));
        }

        if watch && mode == Mode::Human {
            print!("\x1B[2J\x1B[H");
        }

        output::print_ci_status(&statuses, mode);

        if !watch || mode != Mode::Human {
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

/// Fetch the latest pipeline for the PR's source branch and shape it into the
/// same `CiStatus` struct GitHub emits, so the renderer stays forge-agnostic.
async fn fetch_bitbucket_ci(
    bb: &bitbucket::BitbucketClient,
    pr_id: u64,
) -> Result<crate::github::CiStatus> {
    let branch = bb.get_pr_source_branch(pr_id).await?.unwrap_or_default();

    // No branch resolvable → return an empty CiStatus rather than erroring;
    // matches the behaviour of GitHub's "no checks" path.
    if branch.is_empty() {
        return Ok(crate::github::CiStatus {
            pr_number: pr_id,
            head_sha: String::new(),
            overall: "no checks".to_string(),
            checks: Vec::new(),
        });
    }

    let pipeline = bb.get_latest_pipeline_for_branch(&branch).await?;
    let overall = bitbucket::pipeline_status_to_ci_string(pipeline.as_ref());

    // Project a single CheckRun representing the pipeline so that --watch's
    // "all completed" check works the same way it does for GitHub. Pipelines
    // in pending/in_progress map to status "in_progress"; everything else to
    // "completed".
    let checks: Vec<crate::github::CheckRun> = match pipeline {
        Some(p) => {
            let upper = p.state.to_ascii_uppercase();
            let status = match upper.as_str() {
                "PENDING" | "IN_PROGRESS" | "HALTED" => "in_progress",
                _ => "completed",
            };
            let conclusion = match overall.as_str() {
                "passing" => Some("success".to_string()),
                "failing" => Some("failure".to_string()),
                _ => None,
            };
            vec![crate::github::CheckRun {
                name: p.name,
                status: status.to_string(),
                conclusion,
                started_at: None,
                completed_at: None,
                html_url: p.url,
            }]
        }
        None => Vec::new(),
    };

    Ok(crate::github::CiStatus {
        pr_number: pr_id,
        head_sha: String::new(),
        overall,
        checks,
    })
}
