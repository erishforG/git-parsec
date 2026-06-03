//! `parsec __complete <kind>` — hidden helper for dynamic shell completion (#291).
//!
//! Emits newline-separated candidates to stdout. Shell completion scripts
//! (zsh/bash/fish) call this from inside the generated completion function
//! whenever the cursor is on a worktree- or branch-shaped argument.
//!
//! Failures are silent (empty output, exit 0) so that completion never
//! interrupts the user when, e.g., the cwd is not a git repo.

use std::path::Path;

use anyhow::Result;

use crate::cli::CompleteKind;
use crate::config::ParsecConfig;
use crate::git;
use crate::worktree::WorktreeManager;

pub async fn complete(repo_path: &Path, kind: CompleteKind) -> Result<()> {
    match kind {
        CompleteKind::Worktrees => emit_worktrees(repo_path),
        CompleteKind::Branches => emit_branches(repo_path),
    }
    Ok(())
}

fn emit_worktrees(repo_path: &Path) {
    let Ok(repo_root) = git::get_main_repo_root(repo_path) else {
        return;
    };
    let Ok(config) = ParsecConfig::load() else {
        return;
    };
    let Ok(manager) = WorktreeManager::new(&repo_root, &config) else {
        return;
    };
    let Ok(workspaces) = manager.list() else {
        return;
    };
    for ws in workspaces {
        println!("{}", ws.ticket);
    }
}

fn emit_branches(repo_path: &Path) {
    let Ok(repo_root) = git::get_main_repo_root(repo_path) else {
        return;
    };
    let Ok(branches) = git::list_local_branches(&repo_root) else {
        return;
    };
    for b in branches {
        println!("{}", b);
    }
}
