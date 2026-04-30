use std::path::Path;

use anyhow::{Context, Result};

use crate::config::{ParsecConfig, TrackerProvider};
use crate::errors::ErrorCode;
use crate::git;
use crate::github;
use crate::output::{self, Mode};
use crate::tracker;
use crate::worktree::WorktreeManager;

#[allow(clippy::too_many_arguments)]
pub async fn start(
    repo: &Path,
    ticket: &str,
    base: Option<&str>,
    title: Option<String>,
    on: Option<&str>,
    existing_branch: Option<&str>,
    hook: Option<String>,
    mode: Mode,
) -> Result<()> {
    crate::execlog::set_ticket(ticket);
    let mut config = ParsecConfig::load()?;
    let repo_root = git::get_repo_root(repo)?;
    config.resolve_for_repo(&repo_root);

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

    // Idempotency: if workspace already exists, switch to it instead of failing
    if let Ok(existing) = manager.get(ticket) {
        if existing.path.exists() {
            output::print_start(&existing, mode);
            return Ok(());
        }
        // Path gone but state exists — remove stale state and proceed
        let mut state = crate::worktree::ParsecState::load(&repo_root)?;
        state.remove_workspace(ticket);
        state.save(&repo_root)?;
    }

    let workspace = manager.create(ticket, base, ticket_title, on, existing_branch)?;

    output::print_start(&workspace, mode);

    // Auto-transition ticket status
    if let Some(ref auto) = config.tracker.auto_transition {
        if let Some(ref status) = auto.on_start {
            tracker::try_transition(&config, ticket, status).await;
        }
    }

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
            pr_number: None,
            pr_url: None,
        }),
    ) {
        eprintln!("warning: failed to write oplog: {e}");
    }

    // Run one-off hook if provided (runs after config-based hooks in manager.create)
    if let Some(hook_cmd) = hook {
        eprintln!("Running post-create hook: {}", hook_cmd);
        let status = std::process::Command::new("sh")
            .args(["-c", &hook_cmd])
            .current_dir(&workspace.path)
            .status();
        match status {
            Ok(s) if s.success() => {}
            Ok(s) => eprintln!("warning: hook '{}' exited with {}", hook_cmd, s),
            Err(e) => eprintln!("warning: failed to run hook '{}': {}", hook_cmd, e),
        }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn create(
    repo: &Path,
    title: &str,
    body: Option<&str>,
    labels: &[String],
    project: Option<&str>,
    issue_type: &str,
    start_worktree: bool,
    mode: Mode,
) -> Result<()> {
    crate::tracker::load_atlassian_env();

    let mut config = ParsecConfig::load()?;
    let repo_root = git::get_repo_root(repo)?;
    config.resolve_for_repo(&repo_root);

    let (ticket_id, ticket_url) = match config.tracker.provider {
        TrackerProvider::Github => {
            let remote_url = git::get_remote_url(&repo_root)?;
            let gh = github::GitHubClient::new(&remote_url, &config)?.ok_or_else(|| {
                anyhow::anyhow!(
                    "GitHub token not configured. Set GITHUB_TOKEN or configure \
                     [github.\"github.com\"] in parsec config."
                )
            })?;
            let result = gh.create_issue(title, body, labels).await?;
            (format!("#{}", result.number), result.url)
        }
        TrackerProvider::Jira => {
            let base_url = config
                .tracker
                .jira
                .as_ref()
                .map(|j| j.base_url.clone())
                .or_else(|| std::env::var(crate::env::JIRA_BASE_URL).ok())
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "Jira base URL not found. Set it in config or {} env var.",
                        crate::env::JIRA_BASE_URL,
                    )
                })?;

            let project_key = project
                .map(str::to_owned)
                .or_else(|| config.tracker.jira.as_ref().and_then(|j| j.project.clone()))
                .or_else(|| std::env::var(crate::env::PARSEC_JIRA_PROJECT).ok())
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "Jira project key required. Pass --project PROJ or set \
                         tracker.jira.project in config."
                    )
                })?;

            let email = config.tracker.jira.as_ref().and_then(|j| j.email.clone());
            let config_token = config
                .tracker
                .jira
                .as_ref()
                .and_then(|j| j.token.as_deref());
            let jira =
                crate::tracker::jira::JiraTracker::new(&base_url, email.as_deref(), config_token);
            let (key, url) = jira
                .create_issue(&project_key, title, body, issue_type)
                .await?;
            (key, url)
        }
        TrackerProvider::Gitlab | TrackerProvider::None => {
            // Auto-detect: try GitHub if remote points to github.com
            if let Ok(remote_url) = git::get_remote_url(&repo_root) {
                if github::parse_github_remote(&remote_url).is_some() {
                    let gh = github::GitHubClient::new(&remote_url, &config)?.ok_or_else(|| {
                        anyhow::anyhow!(
                            "GitHub token not configured. Set GITHUB_TOKEN or configure \
                             [github.\"github.com\"] in parsec config."
                        )
                    })?;
                    let result = gh.create_issue(title, body, labels).await?;
                    (format!("#{}", result.number), result.url)
                } else {
                    bail_code!(
                        ErrorCode::E011,
                        "Tracker not configured (or not yet supported). \
                         Set tracker.provider = \"github\" or \"jira\" in parsec config."
                    )
                }
            } else {
                anyhow::bail!(
                    "Tracker not configured (or not yet supported). \
                     Set tracker.provider = \"github\" or \"jira\" in parsec config."
                )
            }
        }
    };

    output::print_create(&ticket_id, title, &ticket_url, mode);

    if start_worktree {
        start(
            repo,
            &ticket_id,
            None,
            Some(title.to_string()),
            None,
            None,
            None,
            mode,
        )
        .await?;
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
    let mut config = ParsecConfig::load()?;
    let repo_root = git::get_repo_root(repo)?;
    config.resolve_for_repo(&repo_root);

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
            pr_number: None,
            pr_url: None,
        }),
    ) {
        eprintln!("warning: failed to write oplog: {e}");
    }

    // Auto-detect existing PR for the adopted branch
    if let Ok(remote_url) = git::run_output(manager.repo_root(), &["remote", "get-url", "origin"]) {
        if let Ok(Some(gh)) = github::GitHubClient::new(&remote_url, &config) {
            if let Ok(Some(pr_number)) = gh.find_pr_by_branch(&workspace.branch).await {
                // Record synthetic Ship entry so merge/pr-status can find the PR
                let remote = gh.remote();
                let pr_url = format!(
                    "https://{}/{}/{}/pull/{}",
                    remote.host, remote.owner, remote.repo, pr_number
                );
                if let Err(e) = crate::oplog::record(
                    manager.repo_root(),
                    crate::oplog::OpKind::Ship,
                    Some(ticket),
                    &format!("Adopted branch '{}' -> {}", workspace.branch, pr_url),
                    None,
                ) {
                    eprintln!("warning: failed to record PR in oplog: {e}");
                } else if mode != Mode::Quiet {
                    eprintln!("  Detected existing PR #{}", pr_number);
                }
            }
        }
    }

    Ok(())
}

pub async fn list(repo: &Path, no_pr: bool, full: bool, mode: Mode) -> Result<()> {
    let config = ParsecConfig::load()?;
    let manager = WorktreeManager::new(repo, &config)?;
    let workspaces = manager.list()?;

    // Build PR info map from oplog Ship entries
    let mut pr_map: std::collections::HashMap<String, (u64, String)> =
        std::collections::HashMap::new();
    if !no_pr {
        if let Ok(oplog) = crate::oplog::OpLog::load(manager.repo_root()) {
            let remote_url = git::get_remote_url(manager.repo_root()).ok();
            for entry in &oplog.entries {
                if matches!(entry.op, crate::oplog::OpKind::Ship) {
                    if let Some(ref ticket) = entry.ticket {
                        if let Some(pr_url) = extract_pr_url(&entry.detail) {
                            if let Some(pr_num) = extract_pr_number(&pr_url) {
                                pr_map
                                    .entry(ticket.clone())
                                    .or_insert((pr_num, "open".to_string()));
                            }
                        }
                    }
                }
            }

            // Fetch live PR status from GitHub
            if let Some(ref remote_url) = remote_url {
                if let Ok(Some(gh)) = github::GitHubClient::new(remote_url, &config) {
                    for (_ticket, (pr_num, state)) in pr_map.iter_mut() {
                        if let Ok(status) = gh.get_pr_status(*pr_num).await {
                            *state = status.state;
                        }
                    }
                }
            }
        }
    }

    if full {
        let infos = gather_full_info(workspaces);
        output::print_list_full(&infos, &pr_map, mode);
    } else {
        output::print_list(&workspaces, &pr_map, mode);
    }
    Ok(())
}

fn gather_full_info(workspaces: Vec<crate::worktree::Workspace>) -> Vec<output::WorkspaceFullInfo> {
    workspaces
        .into_iter()
        .map(|ws| {
            let path = &ws.path;

            // Unpushed commits: commits on HEAD not yet pushed to upstream
            let unpushed = git::run_output(path, &["rev-list", "@{u}..HEAD", "--count"])
                .ok()
                .and_then(|s| s.trim().parse::<u32>().ok());

            // Ahead/behind vs base branch
            let (ahead, behind) = {
                let range = format!("{}...HEAD", ws.base_branch);
                if let Ok(out) =
                    git::run_output(path, &["rev-list", "--left-right", "--count", &range])
                {
                    // output is "behind\tahead"
                    let parts: Vec<&str> = out.split_whitespace().collect();
                    if parts.len() == 2 {
                        let b = parts[0].parse::<u32>().ok();
                        let a = parts[1].parse::<u32>().ok();
                        (a, b)
                    } else {
                        (None, None)
                    }
                } else {
                    (None, None)
                }
            };

            // Last commit subject
            let last_commit_msg = git::run_output(path, &["log", "-1", "--format=%s"])
                .ok()
                .filter(|s| !s.is_empty());

            // Last commit relative age
            let last_commit_age = git::run_output(path, &["log", "-1", "--format=%ar"])
                .ok()
                .filter(|s| !s.is_empty());

            output::WorkspaceFullInfo {
                workspace: ws,
                unpushed,
                ahead,
                behind,
                last_commit_msg,
                last_commit_age,
            }
        })
        .collect()
}

fn extract_pr_url(detail: &str) -> Option<String> {
    // Oplog detail for Ship contains the PR URL after " -> "
    detail
        .split(" -> ")
        .nth(1)
        .map(|s| s.trim().to_string())
        .filter(|s| s.starts_with("http"))
}

fn extract_pr_number(url: &str) -> Option<u64> {
    // Extract PR number from URL like "https://github.com/owner/repo/pull/123"
    url.rsplit('/').next().and_then(|s| s.parse().ok())
}

pub async fn status(repo: &Path, ticket: Option<&str>, mode: Mode) -> Result<()> {
    let config = ParsecConfig::load()?;
    let repo_root = git::get_repo_root(repo).ok();
    let manager = WorktreeManager::new(repo, &config)?;

    let workspaces = match ticket {
        Some(t) => vec![manager.get(t)?],
        None => manager.list()?,
    };

    // Fetch live tracker info for each workspace (best-effort)
    let mut ticket_infos: Vec<Option<crate::tracker::Ticket>> = Vec::new();
    for ws in &workspaces {
        let info = crate::tracker::fetch_ticket(&config, &ws.ticket, repo_root.as_deref())
            .await
            .ok()
            .flatten();
        ticket_infos.push(info);
    }

    output::print_status(&workspaces, &ticket_infos, mode);
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
                bail_code!(
                    ErrorCode::E007,
                    "no active workspaces. Run `parsec start <ticket>` to create one."
                );
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
            if crate::env::is_agent() {
                anyhow::bail!("Interactive workspace picker is not available in agent mode. Specify the ticket ID explicitly: `parsec switch <TICKET>`");
            }
            let selection = dialoguer::Select::new()
                .with_prompt("Switch to workspace")
                .items(&items)
                .default(0)
                .interact()?;
            workspaces[selection].ticket.clone()
        }
    };

    // Handle pr:NUMBER syntax
    if let Some(pr_num_str) = ticket.strip_prefix("pr:") {
        let pr_number: u64 = pr_num_str
            .parse()
            .context("invalid PR number after 'pr:'")?;

        let repo_root = manager.repo_root().to_path_buf();
        let remote_url = git::get_remote_url(&repo_root)?;

        // Fetch PR info from GitHub to get head branch
        let gh = github::GitHubClient::new(&remote_url, &config)?
            .ok_or_else(|| anyhow::anyhow!("no GitHub token found. Set PARSEC_GITHUB_TOKEN."))?;
        let pr_info = gh
            .get_pr_info(pr_number)
            .await?
            .ok_or_else(|| anyhow::anyhow!("PR #{} not found", pr_number))?;

        let branch = pr_info.head_branch.clone();
        let ticket_id = format!("pr-{}", pr_number);

        // Fetch from remote to ensure the branch ref is available
        git::fetch(&repo_root)?;

        // Check if a worktree already exists for this branch
        let workspaces = manager.list()?;
        if let Some(existing) = workspaces.iter().find(|w| w.branch == branch) {
            output::print_switch(existing, mode);
            return Ok(());
        }

        // Ensure a local branch exists tracking the remote branch.
        // adopt() verifies refs/heads/<branch>, so we create it if absent.
        let local_exists = git::run_output(
            &repo_root,
            &["rev-parse", "--verify", &format!("refs/heads/{}", branch)],
        )
        .is_ok();

        if !local_exists {
            git::run(
                &repo_root,
                &["branch", "--track", &branch, &format!("origin/{}", branch)],
            )
            .with_context(|| {
                format!(
                    "failed to create local branch '{}' from origin/{}",
                    branch, branch
                )
            })?;
        }

        let workspace = manager.adopt(&ticket_id, Some(&branch), Some(pr_info.title.clone()))?;
        output::print_switch(&workspace, mode);
        return Ok(());
    }

    let workspace = manager.get(&ticket)?;
    output::print_switch(&workspace, mode);
    Ok(())
}

pub async fn clean(
    repo: &Path,
    ticket: Option<&str>,
    all: bool,
    dry_run: bool,
    orphans: bool,
    mode: Mode,
) -> Result<()> {
    let config = ParsecConfig::load()?;
    let manager = WorktreeManager::new(repo, &config)?;

    if orphans {
        // Orphan-only mode: just clean state entries without existing directories
        let orphan_list = manager.clean_orphans(dry_run)?;
        output::print_clean(&orphan_list, dry_run, mode);
        return Ok(());
    }

    // Single-ticket clean mode
    if let Some(ticket_id) = ticket {
        let ws = match manager.get(ticket_id) {
            Ok(ws) => ws,
            Err(_) => {
                if mode == Mode::Human {
                    eprintln!(
                        "Ticket {} already cleaned or not found. Nothing to do.",
                        ticket_id
                    );
                }
                return Ok(());
            }
        };

        if dry_run {
            output::print_clean(std::slice::from_ref(&ws), dry_run, mode);
            return Ok(());
        }

        // Remove the worktree
        let repo_root = manager.repo_root().to_path_buf();
        if ws.path.exists() {
            git::worktree_remove(&repo_root, &ws.path)?;
        }

        // Delete local branch
        if let Err(e) = git::delete_branch(&repo_root, &ws.branch) {
            eprintln!("warning: failed to delete branch '{}': {}", ws.branch, e);
        }

        // Remove from state
        let mut state = crate::worktree::ParsecState::load(&repo_root)?;
        state.remove_workspace(ticket_id);
        state.save(&repo_root)?;

        output::print_clean(std::slice::from_ref(&ws), dry_run, mode);

        // Record in oplog
        if let Err(e) = crate::oplog::record(
            &repo_root,
            crate::oplog::OpKind::Clean,
            Some(ticket_id),
            &format!("Cleaned workspace for branch '{}'", ws.branch),
            Some(crate::oplog::UndoInfo {
                branch: Some(ws.branch.clone()),
                base_branch: Some(ws.base_branch.clone()),
                path: Some(ws.path.clone()),
                ticket_title: ws.ticket_title.clone(),
                pr_number: None,
                pr_url: None,
            }),
        ) {
            eprintln!("warning: failed to write oplog: {e}");
        }

        return Ok(());
    }

    // Regular clean — also report orphans as a hint
    let orphan_list = manager.clean_orphans(true)?; // dry-run detection only
    if !orphan_list.is_empty() {
        eprintln!(
            "note: {} orphan state entry(ies) found (directory missing). Use `parsec clean --orphans` to remove.",
            orphan_list.len()
        );
    }

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
                    pr_number: None,
                    pr_url: None,
                }),
            ) {
                eprintln!("warning: failed to write oplog: {e}");
            }
        }
    }

    Ok(())
}

pub async fn rename(repo: &Path, old_ticket: &str, new_ticket: &str, mode: Mode) -> Result<()> {
    crate::worktree::validate_ticket_id(new_ticket)?;

    let config = ParsecConfig::load()?;
    let repo_root = git::get_main_repo_root(repo).or_else(|_| git::get_repo_root(repo))?;
    let manager = WorktreeManager::new(repo, &config)?;

    // Verify old workspace exists
    let old_ws = manager.get(old_ticket)?;

    // Verify new ticket doesn't already exist
    if manager.get(new_ticket).is_ok() {
        bail_code!(
            ErrorCode::E006,
            "ticket '{}' already exists. Choose a different ticket ID.",
            new_ticket
        );
    }

    // Compute new branch name
    let new_branch = format!("{}{}", config.workspace.branch_prefix, new_ticket);

    // Rename git branch
    git::run(&old_ws.path, &["branch", "-m", &old_ws.branch, &new_branch]).with_context(|| {
        format!(
            "failed to rename branch '{}' to '{}'",
            old_ws.branch, new_branch
        )
    })?;

    // Compute new worktree path
    let new_path = match config.workspace.layout {
        crate::config::WorktreeLayout::Sibling => {
            let repo_name = repo_root
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "repo".to_string());
            repo_root
                .parent()
                .unwrap_or(&repo_root)
                .join(format!("{}.{}", repo_name, new_ticket))
        }
        crate::config::WorktreeLayout::Internal => {
            repo_root.join(&config.workspace.base_dir).join(new_ticket)
        }
    };

    // Move worktree directory
    if old_ws.path != new_path {
        std::fs::rename(&old_ws.path, &new_path).with_context(|| {
            format!(
                "failed to move worktree from {:?} to {:?}",
                old_ws.path, new_path
            )
        })?;

        // Repair git worktree references after move
        let _ = git::run(&repo_root, &["worktree", "repair"]);
    }

    // Update state
    let mut state = crate::worktree::ParsecState::load(&repo_root)?;
    state.remove_workspace(old_ticket);

    let new_ws = crate::worktree::Workspace {
        ticket: new_ticket.to_owned(),
        path: new_path,
        branch: new_branch,
        base_branch: old_ws.base_branch,
        created_at: old_ws.created_at,
        ticket_title: old_ws.ticket_title,
        status: old_ws.status,
        parent_ticket: old_ws.parent_ticket,
    };
    state.add_workspace(new_ws.clone());
    state.save(&repo_root)?;

    // Record in oplog
    if let Err(e) = crate::oplog::record(
        &repo_root,
        crate::oplog::OpKind::Start,
        Some(new_ticket),
        &format!("Renamed {} -> {}", old_ticket, new_ticket),
        None,
    ) {
        eprintln!("warning: failed to write oplog: {e}");
    }

    output::print_rename(old_ticket, new_ticket, &new_ws, mode);

    Ok(())
}
