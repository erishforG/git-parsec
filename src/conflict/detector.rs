use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

use crate::git;
use crate::worktree::Workspace;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileConflict {
    pub file: String,
    pub worktrees: Vec<String>, // ticket IDs that modify this file
}

/// Detect files that are modified in multiple active worktrees.
///
/// For each workspace, gets the list of changed files relative to the base branch,
/// then finds files that appear in 2+ worktrees.
pub fn detect(workspaces: &[Workspace]) -> Result<Vec<FileConflict>> {
    // Map: filename -> list of ticket IDs that touch it
    let mut file_map: HashMap<String, Vec<String>> = HashMap::new();
    let mut skipped: Vec<String> = Vec::new();

    for ws in workspaces {
        // Skip non-active workspaces
        if ws.status != crate::worktree::WorkspaceStatus::Active {
            continue;
        }

        // Try origin/base first (more reliable in worktrees), fall back to local base
        let origin_base = format!("origin/{}", ws.base_branch);
        let merge_base = git::get_merge_base(&ws.path, &origin_base, "HEAD")
            .or_else(|_| git::get_merge_base(&ws.path, &ws.base_branch, "HEAD"));

        let mut all_changed: HashSet<String> = HashSet::new();

        // Committed changes (merge_base..HEAD)
        if let Ok(ref base) = merge_base {
            if let Ok(files) = git::get_changed_files(&ws.path, base, "HEAD") {
                all_changed.extend(files);
            }
        }

        // Uncommitted changes (staged + unstaged)
        if let Ok(files) = git::get_uncommitted_files(&ws.path) {
            all_changed.extend(files);
        }

        if all_changed.is_empty() {
            skipped.push(ws.ticket.clone());
            continue;
        }

        for file in all_changed {
            file_map.entry(file).or_default().push(ws.ticket.clone());
        }
    }

    // Report skipped worktrees (no changes yet)
    if !skipped.is_empty() {
        eprintln!(
            "note: {} workspace(s) skipped (no changes yet): {}",
            skipped.len(),
            skipped.join(", ")
        );
    }

    // Filter to only files touched by 2+ worktrees
    let mut conflicts: Vec<FileConflict> = file_map
        .into_iter()
        .filter(|(_, tickets)| tickets.len() > 1)
        .map(|(file, worktrees)| FileConflict { file, worktrees })
        .collect();

    // Sort by number of conflicting worktrees (descending), then by filename
    conflicts.sort_by(|a, b| {
        b.worktrees
            .len()
            .cmp(&a.worktrees.len())
            .then_with(|| a.file.cmp(&b.file))
    });

    Ok(conflicts)
}
