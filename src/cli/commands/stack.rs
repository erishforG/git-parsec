use std::path::Path;

use anyhow::Result;

use crate::config::ParsecConfig;
use crate::git;
use crate::output::{self, Mode};
use crate::worktree::WorktreeManager;

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
                let parent_ws = match workspaces.iter().find(|w| w.ticket == parent_ticket) {
                    Some(ws) => ws,
                    None => {
                        failed.push((
                            child.ticket.clone(),
                            format!("parent workspace '{}' not found", parent_ticket),
                        ));
                        continue;
                    }
                };
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
