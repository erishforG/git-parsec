//! `parsec diff` / `parsec conflicts` / `parsec sync` — worktree-aware diff and sync.
//!
//! ## Commands
//! - **`parsec diff [ticket]`** — show changes in a worktree against its merge-base.
//!   Supports `--stat`, `--name-only`, and `--json` output modes.
//! - **`parsec conflicts`** — pre-flight check that scans all active worktrees for
//!   files that diverge from a common ancestor (speculative conflict detection).
//! - **`parsec sync [ticket]`** — fast-forward an active worktree against the latest
//!   upstream base branch via rebase (default) or merge. See issue #290 for the
//!   full roadmap.

use std::path::Path;

use anyhow::Result;

use crate::config::ParsecConfig;
use crate::conflict;
use crate::git;
use crate::output::{self, Mode};
use crate::worktree::WorktreeManager;

/// Show the diff between a worktree's current state and its merge-base with the
/// upstream base branch (`origin/<base_branch>`).
///
/// If `ticket` is `None`, the function auto-detects the worktree by comparing
/// `cwd` against known worktree paths; returns an error if the cwd is outside
/// any parsec-managed worktree.
///
/// Output modes:
/// - `--name-only` → list of changed file paths (human or JSON)
/// - `--stat`      → diffstat summary (human or JSON)
/// - default       → full unified diff piped to the terminal (human) or
///   name-status pairs (JSON)
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

/// Detect files that are modified in multiple active worktrees simultaneously.
///
/// Scans every workspace returned by [`WorktreeManager::list`] and compares
/// the set of changed files. Pairs of worktrees that touch the same path are
/// reported as potential conflicts so the developer can resolve them before
/// merging. Does **not** modify any worktree state.
pub async fn conflicts(repo: &Path, mode: Mode) -> Result<()> {
    let config = ParsecConfig::load()?;
    let manager = WorktreeManager::new(repo, &config)?;

    let workspaces = manager.list()?;
    let conflicts = conflict::detect(&workspaces)?;

    output::print_conflicts(&conflicts, mode);
    Ok(())
}

/// Sync one or more worktrees with the latest state of their upstream base branch.
///
/// Fetches `origin/<base_branch>` and applies either a **rebase** (default,
/// `strategy = "rebase"`) or a **merge** (`strategy = "merge"`). A failed
/// rebase/merge is automatically aborted so the worktree is left clean.
///
/// `min_behind`: skip worktrees with fewer than this many commits behind
/// `origin/<base_branch>` (default 1 — skip worktrees already up-to-date).
///
/// With `dry_run = true`, the function prints what would be synced and the
/// behind-count for each worktree, then returns without modifying anything.
///
/// Selection logic (in order):
/// 1. `--all`        → all active worktrees
/// 2. `ticket`       → the named worktree only
/// 3. auto-detect    → the worktree whose path contains `cwd`
///
/// Returns a summary of synced/skipped/failed tickets via [`output::print_sync`].
pub async fn sync(
    repo: &Path,
    ticket: Option<&str>,
    all: bool,
    strategy: &str,
    min_behind: u32,
    dry_run: bool,
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
    let mut skipped = Vec::new();
    let mut failed = Vec::new();

    for ws in &workspaces {
        let ws_path = std::path::Path::new(&ws.path);

        // Fetch the base branch from remote (skip in dry-run to stay offline)
        if !dry_run {
            if let Err(e) = git::run(ws_path, &["fetch", "origin", &ws.base_branch]) {
                failed.push((ws.ticket.clone(), format!("fetch failed: {e}")));
                continue;
            }
        }

        let remote_base = format!("origin/{}", ws.base_branch);

        // Count commits behind remote base
        let behind: u32 = git::run_output(
            ws_path,
            &["rev-list", "--count", &format!("HEAD..{remote_base}")],
        )
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);

        if behind < min_behind {
            skipped.push((ws.ticket.clone(), behind));
            continue;
        }

        if dry_run {
            eprintln!(
                "[dry-run] Would {} '{}' ({} commit(s) behind {})",
                strategy, ws.ticket, behind, remote_base
            );
            synced.push(ws.ticket.clone());
            continue;
        }

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
                let conflict_hint = if e.to_string().contains("CONFLICT")
                    || e.to_string().contains("conflict")
                {
                    " (conflict detected — resolve manually)"
                } else {
                    ""
                };
                failed.push((
                    ws.ticket.clone(),
                    format!("{strategy} failed: {e}{conflict_hint}"),
                ));
            }
        }
    }

    output::print_sync(&synced, &skipped, &failed, strategy, mode);
    Ok(())
}
