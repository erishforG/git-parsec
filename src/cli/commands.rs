use std::path::Path;

use anyhow::Result;

use crate::config::ParsecConfig;
use crate::conflict;
use crate::git;
use crate::github;
use crate::output::{self, Mode};
use crate::tracker;
use crate::worktree::WorktreeManager;

pub async fn start(repo: &Path, ticket: &str, base: Option<&str>, mode: Mode) -> Result<()> {
    let config = ParsecConfig::load()?;

    // Fetch ticket info from tracker (Jira/GitHub) if configured
    let ticket_info = match tracker::fetch_ticket(&config, ticket).await {
        Ok(info) => info,
        Err(e) => {
            eprintln!("warning: could not fetch ticket info: {e}");
            None
        }
    };

    let ticket_title = ticket_info.as_ref().map(|t| t.title.clone());

    let manager = WorktreeManager::new(repo, &config)?;
    let workspace = manager.create(ticket, base, ticket_title)?;

    output::print_start(&workspace, mode);
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

pub async fn ship(
    repo: &Path,
    ticket: &str,
    draft: bool,
    no_pr: bool,
    mode: Mode,
) -> Result<()> {
    let config = ParsecConfig::load()?;
    let manager = WorktreeManager::new(repo, &config)?;

    // Push + cleanup (sync git operations)
    let mut result = manager.ship(ticket)?;

    // Create GitHub PR (async, uses reqwest)
    if !no_pr && config.ship.auto_pr {
        let pr_title = result
            .ticket_title
            .as_ref()
            .map(|t| format!("{}: {}", result.ticket, t))
            .unwrap_or_else(|| result.ticket.clone());

        // Build PR body with Jira link if available
        let pr_body = build_pr_body(&config, &result.ticket, result.ticket_title.as_deref());

        let remote_url = git::get_remote_url(manager.repo_root());
        if let Ok(remote_url) = remote_url {
            match github::create_pr(
                &remote_url,
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
                    // No GitHub token — skip silently
                }
                Err(e) => {
                    eprintln!("warning: PR creation failed: {e}");
                }
            }
        }
    }

    output::print_ship(&result, mode);
    Ok(())
}

pub async fn clean(repo: &Path, all: bool, dry_run: bool, mode: Mode) -> Result<()> {
    let config = ParsecConfig::load()?;
    let manager = WorktreeManager::new(repo, &config)?;

    let removed = manager.clean(all, dry_run)?;

    output::print_clean(&removed, dry_run, mode);
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

pub async fn switch(repo: &Path, ticket: &str, mode: Mode) -> Result<()> {
    let config = ParsecConfig::load()?;
    let manager = WorktreeManager::new(repo, &config)?;

    let workspace = manager.get(ticket)?;

    output::print_switch(&workspace, mode);
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

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn build_pr_body(config: &ParsecConfig, ticket: &str, title: Option<&str>) -> String {
    let mut body = String::new();

    if let Some(title) = title {
        body.push_str(&format!("## {}\n\n", title));
    }

    // Add Jira link if Jira is configured
    if let Some(ref jira_config) = config.tracker.jira {
        body.push_str(&format!(
            "**Jira**: [{ticket}]({}/browse/{ticket})\n\n",
            jira_config.base_url
        ));
    }

    body.push_str(&format!("Shipped via `parsec ship {ticket}`\n"));

    body
}
