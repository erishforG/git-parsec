use std::path::Path;

use anyhow::Result;

use crate::config::ParsecConfig;
use crate::conflict;
use crate::git;
use crate::github;
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
        &format!("Adopted branch '{}' at {}", workspace.branch, workspace.path.display()),
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
                    eprintln!(
                        "note: PR creation skipped — no GitHub token found.\n      \
                         Set PARSEC_GITHUB_TOKEN, GITHUB_TOKEN, or GH_TOKEN to enable."
                    );
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

pub async fn log(repo: &Path, ticket: Option<&str>, last: usize, mode: Mode) -> Result<()> {
    let repo_root =
        git::get_main_repo_root(repo).or_else(|_| git::get_repo_root(repo))?;
    let oplog = crate::oplog::OpLog::load(&repo_root)?;
    let entries = oplog.get_entries(ticket);
    // Take last N entries
    let start = entries.len().saturating_sub(last);
    let entries: Vec<_> = entries[start..].to_vec();
    output::print_log(&entries, mode);
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
