use std::path::Path;

use anyhow::{Context, Result};

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

pub async fn conflicts(repo: &Path, mode: Mode) -> Result<()> {
    let config = ParsecConfig::load()?;
    let manager = WorktreeManager::new(repo, &config)?;

    let workspaces = manager.list()?;
    let conflicts = conflict::detect(&workspaces)?;

    output::print_conflicts(&conflicts, mode);
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
