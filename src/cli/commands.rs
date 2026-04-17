use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result};
use colored::Colorize;

use crate::config::{ParsecConfig, TrackerProvider};
use crate::conflict;
use crate::git;
use crate::github;
use crate::gitlab;
use crate::output::{self, BoardTicketDisplay, Mode};
use crate::tracker;
use crate::tracker::jira::JiraTracker;
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
    body: Option<String>,
    label: Option<String>,
    project: Option<String>,
    start_worktree: bool,
    mode: Mode,
) -> Result<()> {
    let mut config = ParsecConfig::load()?;
    let repo_root = git::get_repo_root(repo)?;
    config.resolve_for_repo(&repo_root);

    let labels: Vec<String> = label
        .as_deref()
        .map(|l| l.split(',').map(|s| s.trim().to_string()).collect())
        .unwrap_or_default();

    let (ticket_id, ticket_url) = match config.tracker.provider {
        crate::config::TrackerProvider::Github => {
            let remote_url = git::get_remote_url(&repo_root)?;
            match github::create_issue(&remote_url, title, body.as_deref(), labels, &config).await?
            {
                Some((number, url)) => (format!("#{}", number), url),
                None => anyhow::bail!(
                    "GitHub token not configured. Set GITHUB_TOKEN or configure \
                     [github.\"github.com\"] in parsec config."
                ),
            }
        }
        crate::config::TrackerProvider::Jira => {
            crate::tracker::load_atlassian_env();

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
                .or_else(|| config.tracker.jira.as_ref().and_then(|j| j.project.clone()))
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "Jira project key required. Pass --project PROJ or set \
                         tracker.jira.project in config."
                    )
                })?;

            let email = config.tracker.jira.as_ref().and_then(|j| j.email.clone());
            let jira = crate::tracker::jira::JiraTracker::new(&base_url, email.as_deref());
            let (key, url) = jira
                .create_issue(&project_key, title, body.as_deref())
                .await?;
            (key, url)
        }
        crate::config::TrackerProvider::Gitlab | crate::config::TrackerProvider::None => {
            // Auto-detect: try GitHub if remote points to github.com
            if let Ok(remote_url) = git::get_remote_url(&repo_root) {
                if github::parse_github_remote(&remote_url).is_some() {
                    match github::create_issue(&remote_url, title, body.as_deref(), labels, &config)
                        .await?
                    {
                        Some((number, url)) => (format!("#{}", number), url),
                        None => anyhow::bail!(
                            "GitHub token not configured. Set GITHUB_TOKEN or configure \
                             [github.\"github.com\"] in parsec config."
                        ),
                    }
                } else {
                    anyhow::bail!(
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
        }),
    ) {
        eprintln!("warning: failed to write oplog: {e}");
    }

    // Auto-detect existing PR for the adopted branch
    if let Ok(remote_url) = git::run_output(manager.repo_root(), &["remote", "get-url", "origin"]) {
        if let Ok(Some(pr_number)) =
            github::find_pr_by_branch(&remote_url, &workspace.branch, &config).await
        {
            // Record synthetic Ship entry so merge/pr-status can find the PR
            let pr_url = if let Some(remote) = github::parse_github_remote(&remote_url) {
                format!(
                    "https://{}/{}/{}/pull/{}",
                    remote.host, remote.owner, remote.repo, pr_number
                )
            } else {
                format!("pull/{}", pr_number)
            };
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
                for (_ticket, (pr_num, state)) in pr_map.iter_mut() {
                    if let Ok(Some(status)) =
                        github::get_pr_status(remote_url, *pr_num, &config).await
                    {
                        *state = status.state;
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
    let manager = WorktreeManager::new(repo, &config)?;

    let workspaces = match ticket {
        Some(t) => vec![manager.get(t)?],
        None => manager.list()?,
    };

    output::print_status(&workspaces, mode);
    Ok(())
}

pub async fn ship(
    repo: &Path,
    ticket: &str,
    draft: bool,
    no_pr: bool,
    base_override: Option<String>,
    skip_hooks: bool,
    mode: Mode,
) -> Result<()> {
    let mut config = ParsecConfig::load()?;
    let manager = WorktreeManager::new(repo, &config)?;
    config.resolve_for_repo(manager.repo_root());

    // Run pre-ship hooks before pushing
    if !skip_hooks && !config.hooks.pre_ship.is_empty() {
        let workspace = manager.get(ticket)?;
        for hook_cmd in &config.hooks.pre_ship {
            if mode == Mode::Human {
                eprintln!("Running pre-ship hook: {}", hook_cmd);
            }
            let output = std::process::Command::new("sh")
                .args(["-c", hook_cmd])
                .current_dir(&workspace.path)
                .output()
                .with_context(|| format!("Failed to spawn hook: {}", hook_cmd))?;
            if !output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                anyhow::bail!(
                    "pre-ship hook failed: {}\n{}{}",
                    hook_cmd,
                    if stdout.is_empty() {
                        String::new()
                    } else {
                        format!("stdout:\n{}", stdout)
                    },
                    if stderr.is_empty() {
                        String::new()
                    } else {
                        format!("stderr:\n{}", stderr)
                    },
                );
            }
        }
    }

    // Phase 1: Push only (don't clean up yet)
    let mut result = manager.ship_push(ticket)?;

    // Resolve base branch: --base CLI > config default_base > worktree's base_branch
    if let Some(base) = base_override {
        result.base_branch = base;
    } else if let Some(ref default_base) = config.ship.default_base {
        result.base_branch = default_base.clone();
    }
    // else: keep the worktree's original base_branch

    // Phase 2: Create PR/MR (async)
    let mut pr_failed = false;
    if !no_pr && config.ship.auto_pr {
        let (ticket_title, ticket_url) =
            match tracker::fetch_ticket(&config, ticket, Some(manager.repo_root())).await {
                Ok(Some(t)) => (Some(t.title), t.url),
                _ => (None, None),
            };

        // Prefer freshly fetched title over stored one
        let effective_title = ticket_title.as_deref().or(result.ticket_title.as_deref());

        let pr_title = effective_title
            .map(|t| format!("{}: {}", result.ticket, t))
            .unwrap_or_else(|| result.ticket.clone());

        let pr_body = build_pr_body(&result.ticket, effective_title, ticket_url.as_deref());

        let remote_url = git::get_remote_url(manager.repo_root());
        if let Ok(ref remote_url) = remote_url {
            // Check if a PR already exists for this branch (#98)
            if let Ok(Some(existing_pr)) =
                github::find_pr_by_branch(remote_url, &result.branch, &config).await
            {
                let remote = github::parse_github_remote(remote_url);
                let pr_url = if let Some(r) = remote {
                    format!(
                        "https://{}/{}/{}/pull/{}",
                        r.host, r.owner, r.repo, existing_pr
                    )
                } else {
                    format!("PR #{}", existing_pr)
                };
                result.pr_url = Some(pr_url);
            } else {
                match github::create_pr(
                    remote_url,
                    &result.branch,
                    &result.base_branch,
                    &pr_title,
                    &pr_body,
                    draft || config.ship.draft,
                    &config,
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
                                pr_failed = true;
                            }
                            Err(e) => {
                                eprintln!("error: GitLab MR creation failed: {e}");
                                pr_failed = true;
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("error: PR creation failed: {e}");
                        pr_failed = true;
                    }
                }
            }
        }
    }

    // Auto-comment PR link on the ticket if configured
    if config.tracker.comment_on_ship {
        if let Some(ref pr_url) = result.pr_url {
            let comment_body = format!("PR opened: {}", pr_url);
            if let Err(e) =
                tracker::post_comment(&config, ticket, &comment_body, Some(manager.repo_root()))
                    .await
            {
                eprintln!("warning: failed to post comment on ticket: {e}");
            }
        }
    }

    if pr_failed {
        eprintln!(
            "note: worktree preserved at {} — fix the issue and retry `parsec ship {}`",
            manager
                .get(ticket)
                .map(|ws| ws.path.display().to_string())
                .unwrap_or_default(),
            ticket
        );
    }

    output::print_ship(&result, mode);

    // Auto-transition ticket status
    if let Some(ref auto) = config.tracker.auto_transition {
        if let Some(ref status) = auto.on_ship {
            tracker::try_transition(&config, ticket, status).await;
        }
    }

    if let Err(e) = crate::oplog::record(
        manager.repo_root(),
        crate::oplog::OpKind::Ship,
        Some(ticket),
        &format!(
            "Shipped branch '{}'{}{}",
            result.branch,
            result
                .pr_url
                .as_ref()
                .map(|u| format!(" -> {}", u))
                .unwrap_or_default(),
            if pr_failed {
                " (partial: PR failed)"
            } else {
                ""
            },
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

    if pr_failed {
        anyhow::bail!("Ship partial: branch pushed but PR/MR creation failed. Worktree preserved.");
    }

    Ok(())
}

pub async fn clean(repo: &Path, all: bool, dry_run: bool, orphans: bool, mode: Mode) -> Result<()> {
    let config = ParsecConfig::load()?;
    let manager = WorktreeManager::new(repo, &config)?;

    if orphans {
        // Orphan-only mode: just clean state entries without existing directories
        let orphan_list = manager.clean_orphans(dry_run)?;
        output::print_clean(&orphan_list, dry_run, mode);
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

        for ws in &workspaces {
            if let Ok(Some(pr_number)) =
                github::find_pr_by_branch(&remote_url, &ws.branch, &config).await
            {
                all_entries.push((ws.ticket.clone(), pr_number, String::new()));
            }
        }

        if all_entries.is_empty() {
            if let Some(t) = ticket {
                anyhow::bail!("no PR found for {t}. Ship it first with `parsec ship {t}`, or check your GitHub token.");
            } else {
                anyhow::bail!("no PRs found. Ship a ticket first with `parsec ship`, or check your GitHub token.");
            }
        }
    }

    let mut statuses = Vec::new();
    for (ticket_id, pr_number, _url) in &all_entries {
        match crate::github::get_pr_status(&remote_url, *pr_number, &config).await? {
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
            github::find_pr_by_branch(&remote_url, &ws.branch, &config)
                .await?
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "no open PR found for {ticket_id} (branch '{}'). Either ship it with `parsec ship {ticket_id}`, or check that PARSEC_GITHUB_TOKEN is set.",
                        ws.branch
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
            match github::get_check_runs(&remote_url, pr_number, &config).await? {
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
    match github::merge_pr(&remote_url, pr_number, method, delete_branch, &config).await? {
        Some(result) => {
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
            match github::find_pr_by_branch(&remote_url, &ws.branch, &config).await? {
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
            match github::get_check_runs(&remote_url, *pr_number, &config).await? {
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

    // Handle pr:NUMBER syntax
    if let Some(pr_num_str) = ticket.strip_prefix("pr:") {
        let pr_number: u64 = pr_num_str
            .parse()
            .context("invalid PR number after 'pr:'")?;

        let repo_root = manager.repo_root().to_path_buf();
        let remote_url = git::get_remote_url(&repo_root)?;

        // Fetch PR info from GitHub to get head branch
        let pr_info = github::get_pr_info(&remote_url, pr_number, &config)
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
                if let Err(e) = git::delete_branch(&repo_root, branch) {
                    eprintln!("warning: failed to delete branch '{}': {e}", branch);
                }
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

            // Restore state (parent_ticket is lost on undo — acceptable)
            let workspace = crate::worktree::Workspace {
                ticket: ticket.to_owned(),
                path: worktree_path,
                branch: branch.to_owned(),
                base_branch: base_branch.to_owned(),
                created_at: chrono::Utc::now(),
                ticket_title: undo_info.ticket_title.clone(),
                status: crate::worktree::WorkspaceStatus::Active,
                parent_ticket: None,
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

pub async fn stack(repo: &Path, mode: Mode) -> Result<()> {
    let config = ParsecConfig::load()?;
    let manager = WorktreeManager::new(repo, &config)?;
    let workspaces = manager.list()?;

    // Build the set of workspaces that are part of any stack:
    // either they have a parent, or they ARE a parent of something.
    let stacked: Vec<_> = workspaces
        .iter()
        .filter(|w| {
            w.parent_ticket.is_some()
                || workspaces
                    .iter()
                    .any(|other| other.parent_ticket.as_deref() == Some(&w.ticket))
        })
        .cloned()
        .collect();

    if stacked.is_empty() {
        if mode == Mode::Human {
            println!(
                "No stacked worktrees. Use `parsec start <ticket> --on <parent>` to create a stack."
            );
        }
        return Ok(());
    }

    output::print_stack(&stacked, mode);
    Ok(())
}

pub async fn stack_sync(repo: &Path, mode: Mode) -> Result<()> {
    let config = ParsecConfig::load()?;
    let manager = WorktreeManager::new(repo, &config)?;
    let workspaces = manager.list()?;

    let mut synced = Vec::new();
    let mut failed = Vec::new();

    // Find roots: workspaces that have children but no parent themselves
    let roots: Vec<_> = workspaces
        .iter()
        .filter(|w| {
            w.parent_ticket.is_none()
                && workspaces
                    .iter()
                    .any(|other| other.parent_ticket.as_deref() == Some(&w.ticket))
        })
        .collect();

    if roots.is_empty() {
        if mode == Mode::Human {
            println!(
                "No stacked worktrees to sync. Use `parsec start <ticket> --on <parent>` to create a stack."
            );
        }
        return Ok(());
    }

    for root in &roots {
        // First sync root with its base branch
        if let Err(e) = git::run(&root.path, &["fetch", "origin", &root.base_branch]) {
            failed.push((root.ticket.clone(), format!("fetch failed: {e}")));
            continue;
        }
        let remote_base = format!("origin/{}", root.base_branch);
        if let Err(e) = git::run(&root.path, &["rebase", &remote_base]) {
            let _ = git::run(&root.path, &["rebase", "--abort"]);
            failed.push((root.ticket.clone(), format!("rebase failed: {e}")));
            continue;
        }
        synced.push(root.ticket.clone());

        // Then rebase children onto their parent, in topological order
        let mut queue: Vec<&str> = vec![&root.ticket];
        while let Some(parent_ticket) = queue.first().copied() {
            queue.remove(0);
            let children: Vec<_> = workspaces
                .iter()
                .filter(|w| w.parent_ticket.as_deref() == Some(parent_ticket))
                .collect();
            for child in &children {
                let parent_ws = workspaces
                    .iter()
                    .find(|w| w.ticket == parent_ticket)
                    .unwrap();
                if let Err(e) = git::run(&child.path, &["rebase", &parent_ws.branch]) {
                    let _ = git::run(&child.path, &["rebase", "--abort"]);
                    failed.push((
                        child.ticket.clone(),
                        format!("rebase onto {} failed: {e}", parent_ticket),
                    ));
                } else {
                    synced.push(child.ticket.clone());
                    queue.push(&child.ticket);
                }
            }
        }
    }

    output::print_sync(&synced, &failed, "rebase (stack)", mode);
    Ok(())
}

pub async fn ticket(
    repo: &Path,
    ticket_override: Option<&str>,
    comment: Option<String>,
    mode: Mode,
) -> Result<()> {
    let mut config = ParsecConfig::load()?;
    let repo_root = git::get_repo_root(repo)?;
    config.resolve_for_repo(&repo_root);

    // Resolve ticket: explicit arg > auto-detect from current worktree
    let ticket_id = if let Some(t) = ticket_override {
        t.to_string()
    } else {
        // Try to detect from current worktree
        let manager = WorktreeManager::new(repo, &config)?;
        let workspaces = manager.list()?;
        let current_dir = std::env::current_dir()?;

        workspaces
            .iter()
            .find(|ws| current_dir.starts_with(&ws.path))
            .map(|ws| ws.ticket.clone())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Not inside a parsec worktree. Specify a ticket: `parsec ticket <TICKET>`"
                )
            })?
    };

    // If --comment is provided, post the comment and return
    if let Some(comment_text) = comment {
        tracker::post_comment(&config, &ticket_id, &comment_text, Some(&repo_root)).await?;
        output::print_comment(&ticket_id, mode);
        return Ok(());
    }

    // Fetch ticket from tracker
    let ticket = tracker::fetch_ticket(&config, &ticket_id, Some(repo))
        .await?
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Could not fetch ticket '{}'. Check your tracker configuration.",
                ticket_id
            )
        })?;

    output::print_ticket(&ticket, mode);
    Ok(())
}

pub async fn inbox(repo: &Path, pick: bool, mode: Mode) -> Result<()> {
    let mut config = ParsecConfig::load()?;
    if let Ok(repo_root) = git::get_repo_root(repo) {
        config.resolve_for_repo(&repo_root);
    }

    // Inbox currently supports Jira only
    if !matches!(
        config.tracker.provider,
        TrackerProvider::Jira | TrackerProvider::None
    ) {
        anyhow::bail!("Inbox currently supports Jira only.");
    }

    // Load atlassian env for auto-detection
    tracker::load_atlassian_env();

    // Resolve Jira base URL and email
    let base_url = config
        .tracker
        .jira
        .as_ref()
        .map(|j| j.base_url.clone())
        .or_else(|| std::env::var(crate::env::JIRA_BASE_URL).ok())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Jira not configured. Run `parsec config init` or set {}.",
                crate::env::JIRA_BASE_URL,
            )
        })?;
    let email = config.tracker.jira.as_ref().and_then(|j| j.email.clone());
    let jira = JiraTracker::new(&base_url, email.as_deref());

    // JQL: assigned to current user, open statuses, ordered by priority
    let jql =
        "assignee = currentUser() AND status in (\"To Do\", \"In Progress\") ORDER BY priority DESC";

    let tickets = jira.search_assigned_issues(jql).await?;

    // Filter out tickets that already have an active parsec worktree
    let manager = WorktreeManager::new(repo, &config)?;
    let active_tickets: HashSet<String> =
        manager.list()?.iter().map(|ws| ws.ticket.clone()).collect();

    let inbox_tickets: Vec<_> = tickets
        .into_iter()
        .filter(|t| !active_tickets.contains(&t.key))
        .collect();

    if pick {
        if inbox_tickets.is_empty() {
            anyhow::bail!("No assigned tickets without active worktrees.");
        }
        let items: Vec<String> = inbox_tickets
            .iter()
            .map(|t| format!("{} — {} [{}]", t.key, t.summary, t.priority))
            .collect();
        let selection = dialoguer::Select::new()
            .with_prompt("Pick a ticket to start")
            .items(&items)
            .default(0)
            .interact()?;
        let chosen = &inbox_tickets[selection];
        eprintln!("Starting workspace for {} ...", chosen.key.bold());
        // Delegate to `start` command
        return start(
            repo,
            &chosen.key,
            None,
            Some(chosen.summary.clone()),
            None,
            None,
            None,
            mode,
        )
        .await;
    }

    output::print_inbox(&inbox_tickets, mode);
    Ok(())
}

pub async fn board(
    repo: &Path,
    board_id_override: Option<u64>,
    project_override: Option<String>,
    assignee_override: Option<String>,
    show_all: bool,
    mode: Mode,
) -> Result<()> {
    let mut config = ParsecConfig::load()?;
    if let Ok(repo_root) = git::get_repo_root(repo) {
        config.resolve_for_repo(&repo_root);
    }

    // Board view currently supports Jira only
    if !matches!(
        config.tracker.provider,
        TrackerProvider::Jira | TrackerProvider::None
    ) {
        anyhow::bail!("Board view currently supports Jira only.");
    }

    // Load atlassian env for auto-detection
    tracker::load_atlassian_env();

    // Resolve Jira base URL and email
    let base_url = config
        .tracker
        .jira
        .as_ref()
        .map(|j| j.base_url.clone())
        .or_else(|| std::env::var(crate::env::JIRA_BASE_URL).ok())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Jira not configured. Run `parsec config init` or set {}.",
                crate::env::JIRA_BASE_URL,
            )
        })?;
    let email = config.tracker.jira.as_ref().and_then(|j| j.email.clone());
    let jira = JiraTracker::new(&base_url, email.as_deref());

    // Resolve project key: --project CLI > PARSEC_JIRA_PROJECT env > config > worktree inference
    let project = if let Some(p) = project_override {
        p
    } else if let Ok(p) = std::env::var(crate::env::PARSEC_JIRA_PROJECT) {
        p
    } else if let Some(p) = config.tracker.jira.as_ref().and_then(|j| j.project.clone()) {
        p
    } else {
        let config2 = ParsecConfig::load()?;
        let manager = WorktreeManager::new(repo, &config2)?;
        let workspaces = manager.list()?;
        workspaces
            .iter()
            .find_map(|ws| ws.ticket.split('-').next().map(String::from))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Could not infer project key. Use --project <KEY>, set {}, or start a worktree first.",
                    crate::env::PARSEC_JIRA_PROJECT,
                )
            })?
    };

    // Resolve board ID: --board-id CLI > PARSEC_JIRA_BOARD_ID env > config > API fetch
    let board_id = if let Some(id) = board_id_override {
        id
    } else if let Ok(id_str) = std::env::var(crate::env::PARSEC_JIRA_BOARD_ID) {
        id_str.parse::<u64>().map_err(|_| {
            anyhow::anyhow!(
                "{} must be a valid number, got: {}",
                crate::env::PARSEC_JIRA_BOARD_ID,
                id_str,
            )
        })?
    } else if let Some(id) = config.tracker.jira.as_ref().and_then(|j| j.board_id) {
        id
    } else {
        jira.fetch_board_id(&project).await?
    };

    // Resolve assignee filter: --assignee CLI > PARSEC_JIRA_ASSIGNEE env > config > none
    let assignee_filter = if show_all {
        None
    } else if let Some(a) = assignee_override {
        Some(a)
    } else if let Ok(a) = std::env::var(crate::env::PARSEC_JIRA_ASSIGNEE) {
        Some(a)
    } else {
        config
            .tracker
            .jira
            .as_ref()
            .and_then(|j| j.assignee.clone())
    };

    // Fetch active sprint
    let sprint = jira.fetch_active_sprint(board_id).await?;

    // Fetch sprint issues
    let tickets = jira.fetch_sprint_issues(sprint.id).await?;

    // Collect active worktree ticket set
    let config3 = ParsecConfig::load()?;
    let manager = WorktreeManager::new(repo, &config3)?;
    let active_worktree_tickets: HashSet<String> =
        manager.list()?.iter().map(|ws| ws.ticket.clone()).collect();

    // Collect shipped PR ticket set from oplog
    let repo_root = git::get_main_repo_root(repo).or_else(|_| git::get_repo_root(repo))?;
    let oplog = crate::oplog::OpLog::load(&repo_root)?;
    let shipped_tickets: HashSet<String> = oplog
        .entries
        .iter()
        .filter(|e| matches!(e.op, crate::oplog::OpKind::Ship))
        .filter_map(|e| e.ticket.clone())
        .collect();

    // Annotate tickets and group by status
    let mut column_map: Vec<(String, Vec<BoardTicketDisplay>)> = Vec::new();

    // Preserve unique status order as encountered
    let mut seen_statuses: Vec<String> = Vec::new();
    for ticket in &tickets {
        if !seen_statuses.contains(&ticket.status) {
            seen_statuses.push(ticket.status.clone());
        }
    }

    for status in &seen_statuses {
        let col_tickets: Vec<BoardTicketDisplay> = tickets
            .iter()
            .filter(|t| &t.status == status)
            .filter(|t| {
                // Apply assignee filter if set
                if let Some(ref filter) = assignee_filter {
                    t.assignee.as_deref() == Some(filter.as_str())
                } else {
                    true
                }
            })
            .map(|t| BoardTicketDisplay {
                key: t.key.clone(),
                summary: t.summary.clone(),
                assignee: t.assignee.clone(),
                has_worktree: active_worktree_tickets.contains(&t.key),
                has_pr: shipped_tickets.contains(&t.key),
                url: Some(format!(
                    "{}/browse/{}",
                    base_url.trim_end_matches('/'),
                    t.key
                )),
            })
            .collect();
        if !col_tickets.is_empty() {
            column_map.push((status.clone(), col_tickets));
        }
    }

    output::print_board(Some(&sprint), &column_map, mode);
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

pub async fn root(repo_path: &Path) -> Result<()> {
    let repo_root = git::get_main_repo_root(repo_path)?;
    print!("{}", repo_root.display());
    Ok(())
}

pub async fn init_shell(shell: &str) -> Result<()> {
    let script = match shell {
        "bash" => INIT_SHELL_BASH,
        _ => INIT_SHELL_ZSH,
    };
    print!("{}", script);
    Ok(())
}

pub async fn init_install(shell: &str, yes: bool) -> Result<()> {
    let home = dirs::home_dir().context("Could not determine home directory")?;
    let (config_path, eval_line) = match shell {
        "bash" => (
            home.join(".bashrc"),
            "eval \"$(parsec init bash)\"".to_string(),
        ),
        _ => (
            home.join(".zshrc"),
            "eval \"$(parsec init zsh)\"".to_string(),
        ),
    };

    // Check if already installed
    if config_path.exists() {
        let existing = std::fs::read_to_string(&config_path)
            .with_context(|| format!("Failed to read {}", config_path.display()))?;
        if existing.contains("parsec init") {
            println!(
                "{}",
                format!(
                    "Shell integration already present in {}. Nothing to do.",
                    config_path.display()
                )
                .yellow()
            );
            return Ok(());
        }
    }

    if !yes {
        let confirmed = dialoguer::Confirm::new()
            .with_prompt(format!(
                "Add shell integration to {}?",
                config_path.display()
            ))
            .default(true)
            .interact()
            .context("Failed to read confirmation")?;

        if !confirmed {
            println!("{}", "Skipped.".dimmed());
            return Ok(());
        }
    }

    // Append the eval line with a comment
    let append = format!("\n# parsec shell integration\n{}\n", eval_line);
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&config_path)
        .with_context(|| format!("Failed to open {} for writing", config_path.display()))?;
    file.write_all(append.as_bytes())
        .with_context(|| format!("Failed to write to {}", config_path.display()))?;

    println!(
        "{}",
        format!(
            "Shell integration added to {}. Run `source {}` or restart your shell.",
            config_path.display(),
            config_path.display()
        )
        .green()
    );
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

const INIT_SHELL_ZSH: &str = r#"
# parsec shell integration - add to ~/.zshrc
# eval "$(parsec init zsh)"
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
        # Save repo root before merge (CWD may be deleted after)
        local saved_root=""
        if [[ "$1" == "merge" ]]; then
            saved_root=$(command parsec root 2>/dev/null)
        fi
        command parsec "$@"
        local exit_code=$?
        # After merge, if CWD was deleted (worktree cleaned up), cd to main repo
        if [[ "$1" == "merge" && $exit_code -eq 0 ]] && [[ ! -d "$(pwd)" ]]; then
            if [[ -n "$saved_root" && -d "$saved_root" ]]; then
                cd "$saved_root"
                echo "  cd $saved_root"
            fi
        fi
        return $exit_code
    fi
}
"#;

const INIT_SHELL_BASH: &str = r#"
# parsec shell integration - add to ~/.bashrc
# eval "$(parsec init bash)"
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
        # Save repo root before merge (CWD may be deleted after)
        local saved_root=""
        if [[ "$1" == "merge" ]]; then
            saved_root=$(command parsec root 2>/dev/null)
        fi
        command parsec "$@"
        local exit_code=$?
        # After merge, if CWD was deleted (worktree cleaned up), cd to main repo
        if [[ "$1" == "merge" && $exit_code -eq 0 ]] && [[ ! -d "$(pwd)" ]]; then
            if [[ -n "$saved_root" && -d "$saved_root" ]]; then
                cd "$saved_root"
                echo "  cd $saved_root"
            fi
        fi
        return $exit_code
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
// Doctor
// ---------------------------------------------------------------------------

pub async fn doctor(repo: &Path, mode: Mode) -> Result<()> {
    use output::DoctorCheck;
    use std::process::Command as StdCommand;

    let mut checks: Vec<DoctorCheck> = Vec::new();

    // ------------------------------------------------------------------
    // 1. git version and worktree support (requires >= 2.15)
    // ------------------------------------------------------------------
    {
        let git_out = StdCommand::new("git").arg("--version").output();
        match git_out {
            Err(_) => {
                checks.push(DoctorCheck {
                    name: "git_version".to_string(),
                    ok: false,
                    detail: "git not found".to_string(),
                    fix: Some("Install git: https://git-scm.com/downloads".to_string()),
                });
            }
            Ok(out) => {
                let version_str = String::from_utf8_lossy(&out.stdout).trim().to_string();
                // Parse "git version X.Y.Z"
                let ver_nums: Option<(u32, u32)> = version_str
                    .split_whitespace()
                    .find(|s| s.contains('.'))
                    .and_then(|v| {
                        let parts: Vec<&str> = v.split('.').collect();
                        let major = parts.first().and_then(|s| s.parse::<u32>().ok())?;
                        let minor = parts.get(1).and_then(|s| s.parse::<u32>().ok())?;
                        Some((major, minor))
                    });
                let ok = match ver_nums {
                    Some((major, minor)) => major > 2 || (major == 2 && minor >= 15),
                    None => false,
                };
                checks.push(DoctorCheck {
                    name: "git_version".to_string(),
                    ok,
                    detail: if ok {
                        format!("{} (worktree support ok)", version_str)
                    } else {
                        format!("{} (need >= 2.15 for worktree support)", version_str)
                    },
                    fix: if ok {
                        None
                    } else {
                        Some("Upgrade git to 2.15 or later".to_string())
                    },
                });
            }
        }
    }

    // ------------------------------------------------------------------
    // 2. Config file
    // ------------------------------------------------------------------
    {
        let config_path = crate::config::ParsecConfig::config_path();
        let exists = config_path.exists();
        checks.push(DoctorCheck {
            name: "config_file".to_string(),
            ok: exists,
            detail: if exists {
                format!("config file found at {}", config_path.display())
            } else {
                format!("config file not found at {}", config_path.display())
            },
            fix: if exists {
                None
            } else {
                Some("Run `parsec config init` to create the config file".to_string())
            },
        });
    }

    // ------------------------------------------------------------------
    // 3. Token configuration
    // ------------------------------------------------------------------
    {
        let config_result = crate::config::ParsecConfig::load();
        let github_token_found = match &config_result {
            Ok(cfg) => {
                let from_config = cfg.github.values().any(|h| h.token.is_some());
                let from_env = std::env::var("GITHUB_TOKEN").is_ok();
                let from_gh = StdCommand::new("gh")
                    .args(["auth", "token"])
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false);
                if from_config {
                    Some("config file")
                } else if from_env {
                    Some("GITHUB_TOKEN env var")
                } else if from_gh {
                    Some("gh auth token")
                } else {
                    None
                }
            }
            Err(_) => {
                let from_env = std::env::var("GITHUB_TOKEN").is_ok();
                let from_gh = StdCommand::new("gh")
                    .args(["auth", "token"])
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false);
                if from_env {
                    Some("GITHUB_TOKEN env var")
                } else if from_gh {
                    Some("gh auth token")
                } else {
                    None
                }
            }
        };

        match github_token_found {
            Some(source) => {
                let host = config_result
                    .as_ref()
                    .ok()
                    .and_then(|cfg| cfg.github.keys().next().cloned())
                    .unwrap_or_else(|| "github.com".to_string());
                checks.push(DoctorCheck {
                    name: "github_token".to_string(),
                    ok: true,
                    detail: format!("GitHub token configured ({host}) via {source}"),
                    fix: None,
                });
            }
            None => {
                checks.push(DoctorCheck {
                    name: "github_token".to_string(),
                    ok: false,
                    detail: "GitHub token not found".to_string(),
                    fix: Some(
                        "Set GITHUB_TOKEN, run `gh auth login`, or add token to config via `parsec config init`".to_string(),
                    ),
                });
            }
        }

        // Jira token check (only if Jira is configured)
        let jira_configured = config_result
            .as_ref()
            .map(|cfg| cfg.tracker.provider == crate::config::TrackerProvider::Jira)
            .unwrap_or(false);
        if jira_configured {
            let jira_token = std::env::var("JIRA_TOKEN").is_ok();
            checks.push(DoctorCheck {
                name: "jira_token".to_string(),
                ok: jira_token,
                detail: if jira_token {
                    "Jira token configured (JIRA_TOKEN env var)".to_string()
                } else {
                    "Jira token not found — set JIRA_TOKEN or run `parsec config init`".to_string()
                },
                fix: if jira_token {
                    None
                } else {
                    Some("Set JIRA_TOKEN environment variable".to_string())
                },
            });
        }
    }

    // ------------------------------------------------------------------
    // 4. Tracker connectivity
    // ------------------------------------------------------------------
    {
        let config_result = crate::config::ParsecConfig::load();
        if let Ok(cfg) = &config_result {
            match cfg.tracker.provider {
                crate::config::TrackerProvider::Jira => {
                    if let Some(jira) = &cfg.tracker.jira {
                        let url =
                            format!("{}/rest/api/2/myself", jira.base_url.trim_end_matches('/'));
                        let token = std::env::var("JIRA_TOKEN").unwrap_or_default();
                        let email = jira.email.clone().unwrap_or_default();
                        let reachable = if token.is_empty() || email.is_empty() {
                            // Try a simple HEAD without auth
                            StdCommand::new("curl")
                                .args([
                                    "-s",
                                    "-o",
                                    "/dev/null",
                                    "-w",
                                    "%{http_code}",
                                    "--max-time",
                                    "5",
                                    &url,
                                ])
                                .output()
                                .map(|o| {
                                    let code =
                                        String::from_utf8_lossy(&o.stdout).trim().to_string();
                                    code == "200" || code == "401"
                                })
                                .unwrap_or(false)
                        } else {
                            let creds = format!("{}:{}", email, token);
                            StdCommand::new("curl")
                                .args([
                                    "-s",
                                    "-o",
                                    "/dev/null",
                                    "-w",
                                    "%{http_code}",
                                    "--max-time",
                                    "5",
                                    "-u",
                                    &creds,
                                    &url,
                                ])
                                .output()
                                .map(|o| {
                                    let code =
                                        String::from_utf8_lossy(&o.stdout).trim().to_string();
                                    code == "200"
                                })
                                .unwrap_or(false)
                        };
                        checks.push(DoctorCheck {
                            name: "tracker_connectivity".to_string(),
                            ok: reachable,
                            detail: if reachable {
                                format!("Jira API reachable ({})", jira.base_url)
                            } else {
                                format!("Jira API unreachable ({})", jira.base_url)
                            },
                            fix: if reachable {
                                None
                            } else {
                                Some(format!("Check network and Jira URL: {}", jira.base_url))
                            },
                        });
                    }
                }
                crate::config::TrackerProvider::Github => {
                    let reachable = StdCommand::new("curl")
                        .args([
                            "-s",
                            "-o",
                            "/dev/null",
                            "-w",
                            "%{http_code}",
                            "--max-time",
                            "5",
                            "https://api.github.com",
                        ])
                        .output()
                        .map(|o| {
                            let code = String::from_utf8_lossy(&o.stdout).trim().to_string();
                            !code.is_empty() && code != "000"
                        })
                        .unwrap_or(false);
                    checks.push(DoctorCheck {
                        name: "tracker_connectivity".to_string(),
                        ok: reachable,
                        detail: if reachable {
                            "GitHub API reachable (api.github.com)".to_string()
                        } else {
                            "GitHub API unreachable".to_string()
                        },
                        fix: if reachable {
                            None
                        } else {
                            Some("Check network connectivity to api.github.com".to_string())
                        },
                    });
                }
                _ => {}
            }
        }
    }

    // ------------------------------------------------------------------
    // 5. Shell integration installed
    // ------------------------------------------------------------------
    {
        let home = std::env::var("HOME").unwrap_or_default();
        let shell = std::env::var("SHELL").unwrap_or_default();

        let shell_files: Vec<std::path::PathBuf> = if shell.contains("zsh") {
            vec![std::path::PathBuf::from(format!("{}/.zshrc", home))]
        } else if shell.contains("bash") {
            vec![
                std::path::PathBuf::from(format!("{}/.bashrc", home)),
                std::path::PathBuf::from(format!("{}/.bash_profile", home)),
            ]
        } else {
            vec![
                std::path::PathBuf::from(format!("{}/.zshrc", home)),
                std::path::PathBuf::from(format!("{}/.bashrc", home)),
            ]
        };

        let shell_name = if shell.contains("zsh") { "zsh" } else { "bash" };
        let init_pattern = "parsec init";
        let found = shell_files.iter().any(|f| {
            std::fs::read_to_string(f)
                .map(|contents| contents.contains(init_pattern))
                .unwrap_or(false)
        });

        checks.push(DoctorCheck {
            name: "shell_integration".to_string(),
            ok: found,
            detail: if found {
                format!("shell integration installed ({})", shell_name)
            } else {
                "shell integration not found in shell config".to_string()
            },
            fix: if found {
                None
            } else {
                Some(format!(
                    r#"Add to ~/.{shell_name}rc:  eval "$(parsec init {shell_name})""#
                ))
            },
        });
    }

    // ------------------------------------------------------------------
    // 6. Tab completions configured
    // ------------------------------------------------------------------
    {
        let home = std::env::var("HOME").unwrap_or_default();
        let shell = std::env::var("SHELL").unwrap_or_default();
        let shell_name = if shell.contains("zsh") { "zsh" } else { "bash" };

        let shell_files: Vec<std::path::PathBuf> = if shell.contains("zsh") {
            vec![std::path::PathBuf::from(format!("{}/.zshrc", home))]
        } else {
            vec![
                std::path::PathBuf::from(format!("{}/.bashrc", home)),
                std::path::PathBuf::from(format!("{}/.bash_profile", home)),
            ]
        };

        let completions_pattern = "parsec config completions";
        let found = shell_files.iter().any(|f| {
            std::fs::read_to_string(f)
                .map(|contents| contents.contains(completions_pattern))
                .unwrap_or(false)
        });

        checks.push(DoctorCheck {
            name: "tab_completions".to_string(),
            ok: found,
            detail: if found {
                "tab completions configured".to_string()
            } else {
                "tab completions not configured".to_string()
            },
            fix: if found {
                None
            } else {
                Some(format!(
                    r#"Add to ~/.{shell_name}rc:  eval "$(parsec config completions {shell_name})""#
                ))
            },
        });
    }

    // ------------------------------------------------------------------
    // 7. Remote access
    // ------------------------------------------------------------------
    {
        let remote_url = git::run_output(repo, &["remote", "get-url", "origin"]);
        match remote_url {
            Err(_) => {
                checks.push(DoctorCheck {
                    name: "remote_access".to_string(),
                    ok: false,
                    detail: "no remote 'origin' configured".to_string(),
                    fix: Some("Run `git remote add origin <url>` to add a remote".to_string()),
                });
            }
            Ok(url) => {
                let ls_remote = git::run_output(repo, &["ls-remote", "--heads", "origin"]);
                let ok = ls_remote.is_ok();
                checks.push(DoctorCheck {
                    name: "remote_access".to_string(),
                    ok,
                    detail: if ok {
                        format!("remote origin accessible ({})", url)
                    } else {
                        format!("remote origin not accessible ({})", url)
                    },
                    fix: if ok {
                        None
                    } else {
                        Some("Check network access and credentials for the remote".to_string())
                    },
                });
            }
        }
    }

    output::print_doctor(&checks, mode);
    Ok(())
}

pub async fn release(
    repo: &Path,
    version: &str,
    from: Option<&str>,
    no_github_release: bool,
    dry_run: bool,
    mode: Mode,
) -> Result<()> {
    let config = ParsecConfig::load()?;
    let repo_root = git::get_repo_root(repo)?;

    // Resolve source branch: --from > "develop" > git default branch
    let source_branch = if let Some(f) = from {
        f.to_string()
    } else {
        // Try "develop" first, fall back to default branch
        let has_develop =
            git::run_output(&repo_root, &["rev-parse", "--verify", "refs/heads/develop"]).is_ok();
        if has_develop {
            "develop".to_string()
        } else {
            git::get_default_branch(&repo_root)?
        }
    };

    // Resolve target branch from config (default: "main")
    let target_branch = config.release.branch.clone();
    let tag_prefix = config.release.tag_prefix.clone();
    let tag = format!("{}{}", tag_prefix, version);

    let step = |msg: &str| {
        if mode != Mode::Quiet {
            println!("  {}", msg);
        }
    };

    if dry_run && mode != Mode::Quiet {
        println!("Dry run — no changes will be made.\n");
    }

    // Step a: git fetch origin
    step("Fetching from origin...");
    if !dry_run {
        git::run(&repo_root, &["fetch", "origin"])?;
    }

    // Step b: Verify source branch is up to date with origin
    step(&format!(
        "Verifying '{}' is up to date with origin...",
        source_branch
    ));
    if !dry_run {
        // Get local and remote SHAs
        let local_sha = git::run_output(&repo_root, &["rev-parse", &source_branch])?;
        let remote_ref = format!("origin/{}", source_branch);
        let remote_sha =
            git::run_output(&repo_root, &["rev-parse", &remote_ref]).unwrap_or_default();
        if !remote_sha.is_empty() && local_sha != remote_sha {
            // Check if local is behind remote
            let behind = git::run_output(
                &repo_root,
                &[
                    "rev-list",
                    "--count",
                    &format!("{}..{}", source_branch, remote_ref),
                ],
            )
            .unwrap_or_default();
            let behind_n: u32 = behind.trim().parse().unwrap_or(0);
            if behind_n > 0 {
                anyhow::bail!(
                    "branch '{}' is {} commit(s) behind origin/{}. Pull first.",
                    source_branch,
                    behind_n,
                    source_branch
                );
            }
        }
    }

    // Step c: checkout target branch and pull
    step(&format!("Checking out '{}' and pulling...", target_branch));
    if !dry_run {
        git::run(&repo_root, &["checkout", &target_branch])?;
        git::run(&repo_root, &["pull", "origin", &target_branch])?;
    }

    // Step d: merge source branch with --no-ff
    let merge_msg = format!("Release {}", version);
    step(&format!(
        "Merging '{}' into '{}' (no-ff)...",
        source_branch, target_branch
    ));
    if !dry_run {
        git::run(
            &repo_root,
            &["merge", &source_branch, "--no-ff", "-m", &merge_msg],
        )?;
    }

    // Step e: create annotated tag
    step(&format!("Creating tag '{}'...", tag));
    if !dry_run {
        git::run(&repo_root, &["tag", "-a", &tag, "-m", &merge_msg])?;
    }

    // Step f: push with tags
    step(&format!("Pushing '{}' with tags...", target_branch));
    if !dry_run {
        git::run(
            &repo_root,
            &["push", "origin", &target_branch, "--follow-tags"],
        )?;
    }

    // Step g: GitHub Release
    let release_url = if !no_github_release {
        // Check for GitHub remote
        let remote_url = git::run_output(&repo_root, &["remote", "get-url", "origin"]).ok();

        if let Some(ref remote) = remote_url {
            if github::parse_github_remote(remote).is_some() {
                // Generate changelog from git log between previous tag and new tag
                let changelog = if config.release.changelog {
                    // Find the previous tag
                    let prev_tag = git::run_output(
                        &repo_root,
                        &["describe", "--tags", "--abbrev=0", &format!("{}^", tag)],
                    )
                    .ok();

                    let range = if let Some(ref pt) = prev_tag {
                        format!("{}..{}", pt, tag)
                    } else {
                        tag.clone()
                    };

                    let log = if dry_run {
                        // In dry run, use HEAD instead of the new tag (not yet created)
                        let dry_range = if let Some(ref pt) = prev_tag {
                            format!("{}..HEAD", pt)
                        } else {
                            "HEAD".to_string()
                        };
                        git::run_output(
                            &repo_root,
                            &["log", &dry_range, "--pretty=format:- %s (%h)"],
                        )
                        .unwrap_or_default()
                    } else {
                        git::run_output(&repo_root, &["log", &range, "--pretty=format:- %s (%h)"])
                            .unwrap_or_default()
                    };
                    log
                } else {
                    String::new()
                };

                let release_name = format!("{}{}", tag_prefix, version);
                step(&format!("Creating GitHub Release '{}'...", release_name));

                if !dry_run {
                    match github::create_release(remote, &tag, &release_name, &changelog, &config)
                        .await?
                    {
                        Some(url) => Some(url),
                        None => {
                            eprintln!(
                                "warning: no GitHub token found, skipping GitHub Release creation."
                            );
                            None
                        }
                    }
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    // Step h: Print summary
    if mode == Mode::Json {
        let summary = serde_json::json!({
            "version": version,
            "tag": tag,
            "source_branch": source_branch,
            "target_branch": target_branch,
            "dry_run": dry_run,
            "github_release_url": release_url,
        });
        println!("{}", serde_json::to_string_pretty(&summary)?);
    } else if mode == Mode::Human {
        println!();
        println!("{}", "Release complete!".green().bold());
        println!("  Version : {}", version.bold());
        println!("  Tag     : {}", tag.bold());
        println!("  Branch  : {} -> {}", source_branch, target_branch);
        if dry_run {
            println!("  (dry run — no changes were made)");
        }
        if let Some(url) = &release_url {
            println!("  Release : {}", url);
        }
    }

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
