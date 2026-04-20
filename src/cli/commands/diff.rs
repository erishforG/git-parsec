use std::path::Path;

use anyhow::Result;

use crate::config::ParsecConfig;
use crate::conflict;
use crate::git;
use crate::output::{self, Mode};
use crate::worktree::WorktreeManager;

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
