use std::path::Path;

use anyhow::{Context, Result};
use colored::Colorize;

use crate::config::ParsecConfig;
use crate::conflict;
use crate::git;
use crate::github;
use crate::gitlab;
use crate::output::{self, Mode};
use crate::tracker;
use crate::worktree::WorktreeManager;

pub async fn start(
    repo: &Path,
    ticket: &str,
    base: Option<&str>,
    title: Option<String>,
    mode: Mode,
) -> Result<()> {
    let config = ParsecConfig::load()?;
    let repo_root = git::get_repo_root(repo)?;

    let ticket_title = if let Some(t) = title {
        // Manual title provided — skip tracker lookup
        Some(t)
    } else {
        // Fetch from tracker
        match tracker::fetch_ticket(&config, ticket, Some(&repo_root)).await {
            Ok(info) => info.map(|t| t.title),
            Err(e) => {
                eprintln!("warning: could not fetch ticket info: {e}");
                None
            }
        }
    };

    let manager = WorktreeManager::new(repo, &config)?;
    let workspace = manager.create(ticket, base, ticket_title)?;

    output::print_start(&workspace, mode);

    if let Err(e) = crate::oplog::record(
        manager.repo_root(),
        crate::oplog::OpKind::Start,
        Some(ticket),
        &format!("Created workspace at {}", workspace.path.display()),
        Some(crate::oplog::UndoInfo {
            branch: Some(workspace.branch.clone()),
            base_branch: Some(workspace.base_branch.clone()),
            path: Some(workspace.path.clone()),
            ticket_title: workspace.ticket_title.clone(),
        }),
    ) {
        eprintln!("warning: failed to write oplog: {e}");
    }

    Ok(())
}

pub async fn adopt(
    repo: &Path,
    ticket: &str,
    branch: Option<&str>,
    title: Option<String>,
    mode: Mode,
) -> Result<()> {
    let config = ParsecConfig::load()?;
    let repo_root = git::get_repo_root(repo)?;

    let ticket_title = if let Some(t) = title {
        Some(t)
    } else {
        match tracker::fetch_ticket(&config, ticket, Some(&repo_root)).await {
            Ok(info) => info.map(|t| t.title),
            Err(_) => None,
        }
    };

    let manager = WorktreeManager::new(repo, &config)?;
    let workspace = manager.adopt(ticket, branch, ticket_title)?;

    output::print_adopt(&workspace, mode);

    if let Err(e) = crate::oplog::record(
        manager.repo_root(),
        crate::oplog::OpKind::Adopt,
        Some(ticket),
        &format!(
            "Adopted branch '{}' at {}",
            workspace.branch,
            workspace.path.display()
        ),
        Some(crate::oplog::UndoInfo {
            branch: Some(workspace.branch.clone()),
            base_branch: Some(workspace.base_branch.clone()),
            path: Some(workspace.path.clone()),
            ticket_title: workspace.ticket_title.clone(),
        }),
    ) {
        eprintln!("warning: failed to write oplog: {e}");
    }

    Ok(())
}

pub async fn list(repo: &Path, mode: Mode) -> Result<()> {
    let config = ParsecConfig::load()?;
    let manager = WorktreeManager::new(repo, &config)?;

    let workspaces = manager.list()?;

    output::print_list(&workspaces, mode);
    Ok(())
}

pub async fn status(repo: &Path, ticket: Option<&str>, mode: Mode) -> Result<()> {
    let config = ParsecConfig::load()?;
    let manager = WorktreeManager::new(repo, &config)?;

    let workspaces = match ticket {
        Some(t) => vec![manager.get(t)?],
        None => manager.list()?,
    };

    output::print_status(&workspaces, mode);
    Ok(())
}

pub async fn ship(repo: &Path, ticket: &str, draft: bool, no_pr: bool, mode: Mode) -> Result<()> {
    let config = ParsecConfig::load()?;
    let manager = WorktreeManager::new(repo, &config)?;

    // Push + cleanup (sync git operations)
    let mut result = manager.ship(ticket)?;

    // Create GitHub PR (async, uses reqwest)
    if !no_pr && config.ship.auto_pr {
        // Fetch ticket info for URL (works for any tracker)
        let ticket_url =
            match tracker::fetch_ticket(&config, ticket, Some(manager.repo_root())).await {
                Ok(Some(t)) => t.url,
                _ => None,
            };

        let pr_title = result
            .ticket_title
            .as_ref()
            .map(|t| format!("{}: {}", result.ticket, t))
            .unwrap_or_else(|| result.ticket.clone());

        let pr_body = build_pr_body(
            &result.ticket,
            result.ticket_title.as_deref(),
            ticket_url.as_deref(),
        );

        let remote_url = git::get_remote_url(manager.repo_root());
        if let Ok(ref remote_url) = remote_url {
            // Try GitHub first
            match github::create_pr(
                remote_url,
                &result.branch,
                &result.base_branch,
                &pr_title,
                &pr_body,
                draft || config.ship.draft,
            )
            .await
            {
                Ok(Some(pr)) => {
                    result.pr_url = Some(pr.url);
                }
                Ok(None) => {
                    // GitHub had no token — try GitLab
                    match gitlab::create_mr(
                        remote_url,
                        &result.branch,
                        &result.base_branch,
                        &pr_title,
                        &pr_body,
                        draft || config.ship.draft,
                    )
                    .await
                    {
                        Ok(Some(mr)) => {
                            result.pr_url = Some(mr.url);
                        }
                        Ok(None) => {
                            eprintln!(
                                "note: PR/MR creation skipped — no token found.\n      \
                                 Set PARSEC_GITHUB_TOKEN or PARSEC_GITLAB_TOKEN to enable."
                            );
                        }
                        Err(e) => {
                            eprintln!("warning: GitLab MR creation failed: {e}");
                        }
                    }
                }
                Err(e) => {
                    eprintln!("warning: PR creation failed: {e}");
                }
            }
        }
    }

    output::print_ship(&result, mode);

    if let Err(e) = crate::oplog::record(
        manager.repo_root(),
        crate::oplog::OpKind::Ship,
        Some(ticket),
        &format!(
            "Shipped branch '{}'{}",
            result.branch,
            result
                .pr_url
                .as_ref()
                .map(|u| format!(" -> {}", u))
                .unwrap_or_default()
        ),
        Some(crate::oplog::UndoInfo {
            branch: Some(result.branch.clone()),
            base_branch: Some(result.base_branch.clone()),
            path: None,
            ticket_title: result.ticket_title.clone(),
        }),
    ) {
        eprintln!("warning: failed to write oplog: {e}");
    }

    Ok(())
}

pub async fn clean(repo: &Path, all: bool, dry_run: bool, mode: Mode) -> Result<()> {
    let config = ParsecConfig::load()?;
    let manager = WorktreeManager::new(repo, &config)?;

    let removed = manager.clean(all, dry_run)?;

    output::print_clean(&removed, dry_run, mode);

    if !dry_run && !removed.is_empty() {
        for ws in &removed {
            if let Err(e) = crate::oplog::record(
                manager.repo_root(),
                crate::oplog::OpKind::Clean,
                Some(&ws.ticket),
                &format!("Cleaned workspace for branch '{}'", ws.branch),
                Some(crate::oplog::UndoInfo {
                    branch: Some(ws.branch.clone()),
                    base_branch: Some(ws.base_branch.clone()),
                    path: Some(ws.path.clone()),
                    ticket_title: ws.ticket_title.clone(),
                }),
            ) {
                eprintln!("warning: failed to write oplog: {e}");
            }
        }
    }

    Ok(())
}

pub async fn open(
    repo: &Path,
    ticket: &str,
    force_pr: bool,
    force_ticket: bool,
    mode: Mode,
) -> Result<()> {
    let config = ParsecConfig::load()?;

    // Try to find PR URL from oplog
    let repo_root = git::get_main_repo_root(repo).or_else(|_| git::get_repo_root(repo))?;
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
            .map(|url| {
                let url = url
                    .trim_end_matches(".git")
                    .replace("git@github.com:", "https://github.com/");
                format!("{}/issues/{}", url, ticket.trim_start_matches('#'))
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

    if entries.is_empty() {
        if let Some(t) = ticket {
            anyhow::bail!("no shipped PR found for {t}. Ship it first with `parsec ship {t}`.");
        } else {
            anyhow::bail!("no shipped PRs found. Ship a ticket first with `parsec ship`.");
        }
    }

    let mut statuses = Vec::new();
    for (ticket_id, pr_number, _url) in &entries {
        match crate::github::get_pr_status(&remote_url, *pr_number).await? {
            Some(status) => statuses.push((ticket_id.clone(), status)),
            None => {
                anyhow::bail!("no GitHub token found. Set PARSEC_GITHUB_TOKEN.");
            }
        }
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
            github::find_pr_by_branch(&remote_url, &ws.branch)
                .await?
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "no PR found for {ticket_id}. Ship it first with `parsec ship {ticket_id}`."
                    )
                })?
        }
    };

    // Wait for CI to pass (unless --no-wait)
    if !no_wait {
        if mode == Mode::Human {
            eprint!("Waiting for CI to pass...");
        }
        loop {
            match github::get_check_runs(&remote_url, pr_number).await? {
                Some(ci) => {
                    if ci.overall == "passing" {
                        if mode == Mode::Human {
                            eprintln!(" {}", "✓".green());
                        }
                        break;
                    } else if ci.overall == "failing" {
                        if mode == Mode::Human {
                            eprintln!(" {}", "✗".red());
                        }
                        anyhow::bail!(
                            "CI is failing for PR #{}. Fix CI or use --no-wait to merge anyway.",
                            pr_number
                        );
                    }
                    // Still pending — wait and retry
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                }
                None => {
                    anyhow::bail!("no GitHub token found. Set PARSEC_GITHUB_TOKEN.");
                }
            }
        }
    }

    // Determine merge method
    let method = if rebase { "rebase" } else { "squash" };
    let delete_branch = !no_delete_branch;

    // Merge the PR
    match github::merge_pr(&remote_url, pr_number, method, delete_branch).await? {
        Some(result) => {
            output::print_merge(&ticket_id, pr_number, &result, method, mode);

            // Clean up local worktree if it exists
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
                    let _ = git::delete_branch(&repo_root, branch);
                }

                if mode == Mode::Human {
                    println!("  {}", "Local worktree cleaned up.".dimmed());
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
        }
        None => {
            anyhow::bail!("no GitHub token found. Set PARSEC_GITHUB_TOKEN.");
        }
    }

    Ok(())
}

pub async fn ci(
    repo: &Path,
    ticket: Option<&str>,
    watch: bool,
    all: bool,
    mode: Mode,
) -> Result<()> {
    let config = ParsecConfig::load()?;
    let repo_root = git::get_main_repo_root(repo).or_else(|_| git::get_repo_root(repo))?;
    let remote_url = git::run_output(repo, &["remote", "get-url", "origin"])?;
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
            anyhow::bail!("no shipped PRs found. Ship a ticket first with `parsec ship`.");
        }
        targets = entries;
    } else {
        // Resolve which ticket to look up
        let ticket_id = if let Some(t) = ticket {
            t.to_string()
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
            found.ticket
        };

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
            match github::find_pr_by_branch(&remote_url, &ws.branch).await? {
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
            match github::get_check_runs(&remote_url, *pr_number).await? {
                Some(ci) => statuses.push((ticket_id.clone(), ci)),
                None => {
                    anyhow::bail!("no GitHub token found. Set PARSEC_GITHUB_TOKEN.");
                }
            }
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
                std::process::exit(1);
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
                std::process::exit(1);
            }
            return Ok(());
        }

        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }
}

pub async fn diff(
    repo: &Path,
    ticket: Option<&str>,
    stat: bool,
    name_only: bool,
    mode: Mode,
) -> Result<()> {
    let config = ParsecConfig::load()?;
    let manager = WorktreeManager::new(repo, &config)?;

    // Resolve workspace
    let ws = if let Some(t) = ticket {
        manager.get(t)?
    } else {
        let cwd = std::env::current_dir()?;
        let all_ws = manager.list()?;
        all_ws
            .into_iter()
            .find(|w| cwd.starts_with(&w.path))
            .ok_or_else(|| anyhow::anyhow!("not inside a parsec worktree. Specify a ticket."))?
    };

    // Find merge base
    let merge_base = git::run_output(
        &ws.path,
        &["merge-base", &format!("origin/{}", ws.base_branch), "HEAD"],
    )?;
    let merge_base = merge_base.trim();

    if name_only {
        let output = git::run_output(&ws.path, &["diff", "--name-only", merge_base])?;
        let files: Vec<String> = output.lines().map(|l| l.to_string()).collect();
        output::print_diff_names(&files, &ws.ticket, mode);
    } else if stat {
        let output = git::run_output(&ws.path, &["diff", "--stat", merge_base])?;
        output::print_diff_stat(&output, &ws.ticket, mode);
    } else {
        // Full diff — just pass through to terminal in human mode
        if mode == Mode::Json {
            let output = git::run_output(&ws.path, &["diff", "--name-status", merge_base])?;
            let files: Vec<(String, String)> = output
                .lines()
                .filter_map(|l| {
                    let mut parts = l.splitn(2, '\t');
                    let status = parts.next()?.to_string();
                    let file = parts.next()?.to_string();
                    Some((status, file))
                })
                .collect();
            output::print_diff_full_json(&files, &ws.ticket);
        } else if mode == Mode::Human {
            // Pass through with color
            let _ = std::process::Command::new("git")
                .args(["diff", "--color=always", merge_base])
                .current_dir(&ws.path)
                .status();
        }
    }
    Ok(())
}

pub async fn conflicts(repo: &Path, mode: Mode) -> Result<()> {
    let config = ParsecConfig::load()?;
    let manager = WorktreeManager::new(repo, &config)?;

    let workspaces = manager.list()?;
    let conflicts = conflict::detect(&workspaces)?;

    output::print_conflicts(&conflicts, mode);
    Ok(())
}

pub async fn sync(
    repo: &Path,
    ticket: Option<&str>,
    all: bool,
    strategy: &str,
    mode: Mode,
) -> Result<()> {
    let config = ParsecConfig::load()?;
    let manager = WorktreeManager::new(repo, &config)?;

    let workspaces = if all {
        let ws = manager.list()?;
        if ws.is_empty() {
            anyhow::bail!("no active workspaces to sync");
        }
        ws
    } else if let Some(t) = ticket {
        vec![manager.get(t)?]
    } else {
        // Try to detect which worktree we're in
        let cwd = std::env::current_dir()?;
        let all_ws = manager.list()?;
        let found = all_ws
            .into_iter()
            .find(|w| cwd.starts_with(&w.path))
            .ok_or_else(|| {
                anyhow::anyhow!("not inside a parsec worktree. Specify a ticket or use --all.")
            })?;
        vec![found]
    };

    let mut synced = Vec::new();
    let mut failed = Vec::new();

    for ws in &workspaces {
        let ws_path = std::path::Path::new(&ws.path);
        // Fetch the base branch from remote
        if let Err(e) = git::run(ws_path, &["fetch", "origin", &ws.base_branch]) {
            failed.push((ws.ticket.clone(), format!("fetch failed: {e}")));
            continue;
        }
        let remote_base = format!("origin/{}", ws.base_branch);
        let result = match strategy {
            "merge" => git::run(ws_path, &["merge", &remote_base]),
            _ => git::run(ws_path, &["rebase", &remote_base]),
        };
        match result {
            Ok(()) => synced.push(ws.ticket.clone()),
            Err(e) => {
                // Abort failed rebase/merge to leave worktree clean
                if strategy != "merge" {
                    let _ = git::run(ws_path, &["rebase", "--abort"]);
                } else {
                    let _ = git::run(ws_path, &["merge", "--abort"]);
                }
                failed.push((ws.ticket.clone(), format!("{strategy} failed: {e}")));
            }
        }
    }

    output::print_sync(&synced, &failed, strategy, mode);
    Ok(())
}

pub async fn switch(repo: &Path, ticket: Option<&str>, mode: Mode) -> Result<()> {
    let config = ParsecConfig::load()?;
    let manager = WorktreeManager::new(repo, &config)?;

    let ticket = match ticket {
        Some(t) => t.to_string(),
        None => {
            let workspaces = manager.list()?;
            if workspaces.is_empty() {
                anyhow::bail!("no active workspaces. Run `parsec start <ticket>` to create one.");
            }
            let items: Vec<String> = workspaces
                .iter()
                .map(|w| {
                    let title = w
                        .ticket_title
                        .as_deref()
                        .map(|t| format!(" — {t}"))
                        .unwrap_or_default();
                    format!("{}{title}", w.ticket)
                })
                .collect();
            let selection = dialoguer::Select::new()
                .with_prompt("Switch to workspace")
                .items(&items)
                .default(0)
                .interact()?;
            workspaces[selection].ticket.clone()
        }
    };

    let workspace = manager.get(&ticket)?;
    output::print_switch(&workspace, mode);
    Ok(())
}

pub async fn log(repo: &Path, ticket: Option<&str>, last: usize, mode: Mode) -> Result<()> {
    let repo_root = git::get_main_repo_root(repo).or_else(|_| git::get_repo_root(repo))?;
    let oplog = crate::oplog::OpLog::load(&repo_root)?;
    let entries = oplog.get_entries(ticket);
    // Take last N entries
    let start = entries.len().saturating_sub(last);
    let entries: Vec<_> = entries[start..].to_vec();
    output::print_log(&entries, mode);
    Ok(())
}

pub async fn undo(repo: &Path, dry_run: bool, mode: Mode) -> Result<()> {
    let config = ParsecConfig::load()?;
    let repo_root = git::get_main_repo_root(repo).or_else(|_| git::get_repo_root(repo))?;

    let mut oplog = crate::oplog::OpLog::load(&repo_root)?;

    let last = oplog.last_entry().cloned().ok_or_else(|| {
        anyhow::anyhow!("nothing to undo. Run `parsec log` to see operation history.")
    })?;

    let undo_info = last.undo_info.as_ref().ok_or_else(|| {
        anyhow::anyhow!(
            "last operation ({}) cannot be undone — no undo info recorded.",
            last.op
        )
    })?;

    if dry_run {
        output::print_undo_preview(&last, mode);
        return Ok(());
    }

    match last.op {
        crate::oplog::OpKind::Start | crate::oplog::OpKind::Adopt => {
            // Undo start/adopt = remove the worktree + branch + state
            let ticket = last
                .ticket
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("no ticket in oplog entry"))?;

            if let Some(path) = &undo_info.path {
                if path.exists() {
                    git::worktree_remove(&repo_root, path)
                        .with_context(|| format!("failed to remove worktree at {:?}", path))?;
                }
            }
            if let Some(branch) = &undo_info.branch {
                let _ = git::delete_branch(&repo_root, branch);
            }

            // Remove from state
            let mut state = crate::worktree::ParsecState::load(&repo_root)?;
            state.remove_workspace(ticket);
            state.save(&repo_root)?;
        }
        crate::oplog::OpKind::Ship | crate::oplog::OpKind::Clean => {
            // Undo ship/clean = re-create the worktree
            let ticket = last
                .ticket
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("no ticket in oplog entry"))?;
            let branch = undo_info
                .branch
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("no branch info to restore"))?;
            let base_branch = undo_info.base_branch.as_deref().unwrap_or("main");

            // Check if branch exists locally
            let branch_exists_locally = git::run_output(
                &repo_root,
                &["rev-parse", "--verify", &format!("refs/heads/{}", branch)],
            )
            .is_ok();

            // If not local, try to restore from remote
            if !branch_exists_locally {
                let remote_ref = format!("origin/{}", branch);
                if git::run_output(&repo_root, &["rev-parse", "--verify", &remote_ref]).is_ok() {
                    git::run(&repo_root, &["branch", branch, &remote_ref])?;
                } else {
                    anyhow::bail!(
                        "branch '{}' not found locally or on remote. Cannot restore workspace.",
                        branch
                    );
                }
            }

            // Compute worktree path based on layout
            let worktree_path = match config.workspace.layout {
                crate::config::WorktreeLayout::Sibling => {
                    let repo_name = repo_root
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| "repo".to_string());
                    repo_root
                        .parent()
                        .unwrap_or(&repo_root)
                        .join(format!("{}.{}", repo_name, ticket))
                }
                crate::config::WorktreeLayout::Internal => {
                    repo_root.join(&config.workspace.base_dir).join(ticket)
                }
            };

            git::run(
                &repo_root,
                &[
                    "worktree",
                    "add",
                    worktree_path.to_str().unwrap_or(""),
                    branch,
                ],
            )?;

            // Restore state
            let workspace = crate::worktree::Workspace {
                ticket: ticket.to_owned(),
                path: worktree_path,
                branch: branch.to_owned(),
                base_branch: base_branch.to_owned(),
                created_at: chrono::Utc::now(),
                ticket_title: undo_info.ticket_title.clone(),
                status: crate::worktree::WorkspaceStatus::Active,
            };

            let mut state = crate::worktree::ParsecState::load(&repo_root)?;
            state.add_workspace(workspace);
            state.save(&repo_root)?;
        }
        crate::oplog::OpKind::Undo => {
            anyhow::bail!("cannot undo an undo operation");
        }
    }

    // Record undo in oplog
    oplog.append(
        crate::oplog::OpKind::Undo,
        last.ticket.clone(),
        format!("Undid {} operation", last.op),
        None,
    );
    oplog.save(&repo_root)?;

    output::print_undo(&last, mode);
    Ok(())
}

pub async fn config_init(mode: Mode) -> Result<()> {
    let config = ParsecConfig::init_interactive()?;
    config.save()?;

    output::print_config_init(mode);
    Ok(())
}

pub async fn config_show(mode: Mode) -> Result<()> {
    let config = ParsecConfig::load()?;

    output::print_config_show(&config, mode);
    Ok(())
}

pub async fn config_shell(shell: &str, _mode: Mode) -> Result<()> {
    let script = match shell {
        "bash" => SHELL_INTEGRATION_BASH,
        _ => SHELL_INTEGRATION_ZSH,
    };
    print!("{}", script);
    Ok(())
}

const SHELL_INTEGRATION_ZSH: &str = r#"
# parsec shell integration - add to ~/.zshrc
# eval "$(parsec config shell zsh)"
function parsec() {
    if [[ "$1" == "switch" && -n "$2" ]]; then
        local dir
        dir=$(command parsec switch "${@:2}" 2>&1)
        if [[ $? -eq 0 && -d "$dir" ]]; then
            cd "$dir"
        else
            echo "$dir" >&2
            return 1
        fi
    else
        command parsec "$@"
    fi
}
"#;

const SHELL_INTEGRATION_BASH: &str = r#"
# parsec shell integration - add to ~/.bashrc
# eval "$(parsec config shell bash)"
function parsec() {
    if [[ "$1" == "switch" && -n "$2" ]]; then
        local dir
        dir=$(command parsec switch "${@:2}" 2>&1)
        if [[ $? -eq 0 && -d "$dir" ]]; then
            cd "$dir"
        else
            echo "$dir" >&2
            return 1
        fi
    else
        command parsec "$@"
    fi
}
"#;

pub async fn config_man(dir: &Path) -> Result<()> {
    use clap::CommandFactory;
    let cmd = super::Cli::command();
    let man = clap_mangen::Man::new(cmd);
    let mut buf = Vec::new();
    man.render(&mut buf)?;

    let man1_dir = dir.join("man1");
    std::fs::create_dir_all(&man1_dir)
        .with_context(|| format!("Failed to create directory {}", man1_dir.display()))?;

    let path = man1_dir.join("parsec.1");
    std::fs::write(&path, buf)
        .with_context(|| format!("Failed to write man page to {}", path.display()))?;

    println!("Man page installed to {}", path.display());
    println!("Try: man parsec");
    Ok(())
}

pub async fn config_completions(shell: clap_complete::Shell) -> Result<()> {
    use clap::CommandFactory;
    let mut cmd = super::Cli::command();
    clap_complete::generate(shell, &mut cmd, "parsec", &mut std::io::stdout());
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn build_pr_body(ticket: &str, title: Option<&str>, ticket_url: Option<&str>) -> String {
    let mut body = String::new();

    if let Some(title) = title {
        body.push_str(&format!("## {}\n\n", title));
    }

    // Add ticket link if URL is available (works for any tracker)
    if let Some(url) = ticket_url {
        body.push_str(&format!("**Ticket**: [{ticket}]({url})\n\n"));
    }

    body.push_str(&format!("Shipped via `parsec ship {ticket}`\n"));

    body
}
