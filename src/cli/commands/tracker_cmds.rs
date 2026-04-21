use std::collections::HashSet;
use std::path::Path;

use anyhow::Result;
use colored::Colorize;

use crate::config::{ParsecConfig, TrackerProvider};
use crate::git;
use crate::output::{self, BoardTicketDisplay, Mode};
use crate::tracker;
use crate::tracker::jira::JiraTracker;
use crate::worktree::WorktreeManager;

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
    let config_token = config
        .tracker
        .jira
        .as_ref()
        .and_then(|j| j.token.as_deref());
    let jira = JiraTracker::new(&base_url, email.as_deref(), config_token);

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
        return super::start(
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
    let config_token = config
        .tracker
        .jira
        .as_ref()
        .and_then(|j| j.token.as_deref());
    let jira = JiraTracker::new(&base_url, email.as_deref(), config_token);

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
