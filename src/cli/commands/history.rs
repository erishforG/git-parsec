use std::path::Path;

use anyhow::{Context, Result};

use crate::config::ParsecConfig;
use crate::errors::ErrorCode;
use crate::git;
use crate::output::{self, Mode};

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

pub async fn log_export(repo: &Path) -> Result<()> {
    let repo_root = git::get_main_repo_root(repo).or_else(|_| git::get_repo_root(repo))?;
    let raw = crate::execlog::read_raw(&repo_root)?;
    if raw.is_empty() {
        eprintln!("No execution log entries. Run some commands first.");
    } else {
        print!("{}", raw);
    }
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
            bail_code!(ErrorCode::E013, "cannot undo an undo operation");
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
