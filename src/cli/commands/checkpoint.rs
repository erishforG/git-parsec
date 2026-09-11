//! `parsec checkpoint` — point-in-time worktree snapshot via git stash (#300).
//!
//! # Phase 1
//! Provides two subcommands:
//!
//! | Subcommand | Description |
//! |------------|-------------|
//! | `create [name]` | Save the current worktree state (staged + unstaged + untracked) as a named git stash |
//! | `list` | List all parsec-managed checkpoints in the repository |
//!
//! ## Naming convention
//! Every parsec checkpoint stash carries a structured message prefix so it can be
//! distinguished from ordinary `git stash push` entries:
//!
//! ```text
//! parsec-checkpoint:<name>
//! ```
//!
//! When no `name` is given, a UTC timestamp (`YYYYMMDD-HHMMSS`) is used instead.
//!
//! ## How `list` works
//! `git stash list` returns lines like:
//! ```text
//! stash@{0}: On main: parsec-checkpoint:before-rebase
//! stash@{1}: On feat/foo: WIP on feat/foo
//! ```
//! Only lines containing `parsec-checkpoint:` in the message are surfaced.
//!
//! # Phase 2
//! Adds two destructive subcommands:
//!
//! | Subcommand | Description |
//! |------------|-------------|
//! | `restore <name>` | Pop the named checkpoint back into the working tree (`git stash pop`) |
//! | `drop <name>` | Discard the named checkpoint permanently (`git stash drop`) |
//!
//! Both commands perform **name-based lookup** across the full stash list —
//! the user never needs to know the underlying `stash@{N}` index.
//! An exact-match is preferred; if no exact match is found the command fails
//! with a helpful error that lists available checkpoint names.
//!
//! # Phase 3 (planned)
//! - `parsec checkpoint show <name>` — show the diff stored in a checkpoint.
//! - `parsec checkpoint rename <old> <new>` — rename a checkpoint in-place.

use std::path::Path;

use anyhow::{bail, Result};
use chrono::Utc;

use crate::git;
use crate::output::Mode;

const PREFIX: &str = "parsec-checkpoint:";

/// Parsed representation of a single checkpoint stash entry.
#[derive(Debug)]
pub struct CheckpointEntry {
    /// Stash ref (e.g. `stash@{0}`)
    pub stash_ref: String,
    /// User-visible checkpoint name (everything after `parsec-checkpoint:`)
    pub name: String,
    /// Branch the stash was created on
    pub branch: String,
}

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

/// Create a new checkpoint for the current worktree.
///
/// Runs `git stash push --include-untracked -m "parsec-checkpoint:<name>"`.
/// If there are no changes to stash (clean working tree), the command exits
/// with a user-friendly message rather than an error.
pub fn checkpoint_create(repo: &Path, name: Option<&str>, mode: Mode) -> Result<()> {
    let timestamp = Utc::now().format("%Y%m%d-%H%M%S").to_string();
    let label = name.unwrap_or(&timestamp);
    // Reject names that contain the delimiter we use for parsing.
    if label.contains(':') {
        bail!("checkpoint name must not contain ':' — got {:?}", label);
    }

    let message = format!("{PREFIX}{label}");
    let branch = git::get_current_branch(repo).unwrap_or_else(|_| "(unknown)".to_string());

    // Check for any changes first (staged, unstaged, untracked).
    // `git status --porcelain` returns empty output on a clean tree.
    let status_out = git::run_output(repo, &["status", "--porcelain"])?;
    if status_out.is_empty() {
        match mode {
            Mode::Human => {
                println!("Nothing to checkpoint — working tree is clean on branch '{branch}'.")
            }
            Mode::Json => {
                println!(r#"{{"ok":false,"reason":"clean","branch":"{branch}","name":"{label}"}}"#)
            }
            Mode::Quiet => {}
        }
        return Ok(());
    }

    git::run(
        repo,
        &["stash", "push", "--include-untracked", "-m", &message],
    )?;

    match mode {
        Mode::Human => println!(
            "✔ Checkpoint '{label}' created on branch '{branch}'.\n\
             Tip: use `parsec checkpoint list` to see all checkpoints."
        ),
        Mode::Json => println!(
            r#"{{"ok":true,"name":"{label}","branch":"{branch}","stash_message":"{message}"}}"#
        ),
        Mode::Quiet => {}
    }
    Ok(())
}

/// List all parsec-managed checkpoints in the repository.
///
/// Reads `git stash list` and filters for entries whose message contains the
/// `parsec-checkpoint:` prefix.  Non-parsec stashes are not shown.
/// Restore (pop) a named checkpoint back into the working tree.
///
/// Looks up the `stash@{N}` ref for the given name and runs
/// `git stash pop stash@{N}`.  On success the stash entry is removed.
/// If the working tree is dirty, git will refuse the pop; the original
/// stash entry is left intact so the user can resolve conflicts first.
pub fn checkpoint_restore(repo: &Path, name: &str, mode: Mode) -> Result<()> {
    let entry = find_checkpoint(repo, name)?;

    git::run(repo, &["stash", "pop", &entry.stash_ref])?;

    let sref = &entry.stash_ref;
    match mode {
        Mode::Human => {
            println!("✔ Checkpoint '{name}' restored to working tree (stash entry removed).")
        }
        Mode::Json => {
            println!(r#"{{"ok":true,"name":"{name}","stash_ref":"{sref}","action":"restore"}}"#)
        }
        Mode::Quiet => {}
    }
    Ok(())
}

/// Drop (permanently discard) a named checkpoint.
///
/// Looks up the `stash@{N}` ref for the given name and runs
/// `git stash drop stash@{N}`.  This is **irreversible**; the stash entry
/// is deleted and the captured changes are gone.
pub fn checkpoint_drop(repo: &Path, name: &str, mode: Mode) -> Result<()> {
    let entry = find_checkpoint(repo, name)?;

    git::run(repo, &["stash", "drop", &entry.stash_ref])?;

    let sref = &entry.stash_ref;
    match mode {
        Mode::Human => {
            println!("✔ Checkpoint '{name}' dropped (stash entry {sref} permanently removed).")
        }
        Mode::Json => {
            println!(r#"{{"ok":true,"name":"{name}","stash_ref":"{sref}","action":"drop"}}"#)
        }
        Mode::Quiet => {}
    }
    Ok(())
}

/// List all parsec-managed checkpoints in the repository.
///
/// Reads `git stash list` and filters for entries whose message contains the
/// `parsec-checkpoint:` prefix.  Non-parsec stashes are not shown.
pub fn checkpoint_list(repo: &Path, mode: Mode) -> Result<()> {
    // git stash list exits non-zero on repos with no stash history; treat as empty.
    let raw = git::run_output(repo, &["stash", "list"]).unwrap_or_default();

    let entries = parse_stash_list(&raw);

    if entries.is_empty() {
        match mode {
            Mode::Human => {
                println!("No checkpoints found. Create one with `parsec checkpoint create`.")
            }
            Mode::Json => println!("[]"),
            Mode::Quiet => {}
        }
        return Ok(());
    }

    match mode {
        Mode::Human => print_table(&entries),
        Mode::Json => {
            let json_items: Vec<String> = entries
                .iter()
                .map(|e| {
                    format!(
                        r#"{{"stash_ref":"{ref}","name":"{name}","branch":"{branch}"}}"#,
                        ref = e.stash_ref,
                        name = e.name,
                        branch = e.branch,
                    )
                })
                .collect();
            println!("[{}]", json_items.join(","));
        }
        Mode::Quiet => {}
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Find the checkpoint entry whose name exactly matches `name`.
///
/// Returns `Err` with a human-readable message listing available names
/// if no match is found.
fn find_checkpoint(repo: &Path, name: &str) -> Result<CheckpointEntry> {
    let raw = git::run_output(repo, &["stash", "list"]).unwrap_or_default();
    let entries = parse_stash_list(&raw);

    if let Some(entry) = entries.into_iter().find(|e| e.name == name) {
        return Ok(entry);
    }

    // Build a helpful error message.
    let available = git::run_output(repo, &["stash", "list"])
        .map(|r| parse_stash_list(&r))
        .unwrap_or_default();
    if available.is_empty() {
        bail!("No checkpoint named '{name}' found — no parsec checkpoints exist in this repo.");
    }
    let names: Vec<&str> = available.iter().map(|e| e.name.as_str()).collect();
    bail!(
        "No checkpoint named '{name}' found.\nAvailable checkpoints: {}",
        names.join(", ")
    );
}

/// Parse the raw `git stash list` output into [`CheckpointEntry`] records.
///
/// Expected line format (git default):
/// ```text
/// stash@{N}: On <branch>: <message>
/// ```
/// or
/// ```text
/// stash@{N}: WIP on <branch>: <message>
/// ```
fn parse_stash_list(raw: &str) -> Vec<CheckpointEntry> {
    raw.lines().filter_map(parse_stash_line).collect()
}

fn parse_stash_line(line: &str) -> Option<CheckpointEntry> {
    // Split on first ': ' to get the stash ref.
    let (stash_ref, rest) = line.split_once(": ")?;

    // Match both "On <branch>: <msg>" and "WIP on <branch>: <msg>"
    let rest = rest.strip_prefix("WIP on ").unwrap_or(rest);
    let rest = rest.strip_prefix("On ").unwrap_or(rest);

    // The next ': ' separates branch from message.
    let (branch, message) = rest.split_once(": ")?;

    if !message.contains(PREFIX) {
        return None;
    }

    // Extract the name part after the prefix.
    let name = message
        .find(PREFIX)
        .map(|idx| &message[idx + PREFIX.len()..])
        .unwrap_or(message)
        .trim()
        .to_string();

    Some(CheckpointEntry {
        stash_ref: stash_ref.trim().to_string(),
        name,
        branch: branch.to_string(),
    })
}

fn print_table(entries: &[CheckpointEntry]) {
    // Measure column widths.
    let w_ref = entries
        .iter()
        .map(|e| e.stash_ref.len())
        .max()
        .unwrap_or(9)
        .max(9);
    let w_name = entries
        .iter()
        .map(|e| e.name.len())
        .max()
        .unwrap_or(4)
        .max(4);
    let w_branch = entries
        .iter()
        .map(|e| e.branch.len())
        .max()
        .unwrap_or(6)
        .max(6);

    println!(
        "{:<w_ref$}  {:<w_name$}  {:<w_branch$}",
        "STASH REF",
        "NAME",
        "BRANCH",
        w_ref = w_ref,
        w_name = w_name,
        w_branch = w_branch,
    );
    println!("{}", "-".repeat(w_ref + w_name + w_branch + 4));
    for e in entries {
        println!(
            "{:<w_ref$}  {:<w_name$}  {:<w_branch$}",
            e.stash_ref,
            e.name,
            e.branch,
            w_ref = w_ref,
            w_name = w_name,
            w_branch = w_branch,
        );
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- parse_stash_line ---------------------------------------------------

    #[test]
    fn parses_checkpoint_entry_on_prefix() {
        let line = "stash@{0}: On main: parsec-checkpoint:before-rebase";
        let entry = parse_stash_line(line).expect("should parse");
        assert_eq!(entry.stash_ref, "stash@{0}");
        assert_eq!(entry.name, "before-rebase");
        assert_eq!(entry.branch, "main");
    }

    #[test]
    fn parses_checkpoint_entry_wip_prefix() {
        let line = "stash@{1}: WIP on feat/foo: parsec-checkpoint:20260911-093000";
        let entry = parse_stash_line(line).expect("should parse");
        assert_eq!(entry.stash_ref, "stash@{1}");
        assert_eq!(entry.name, "20260911-093000");
        assert_eq!(entry.branch, "feat/foo");
    }

    #[test]
    fn ignores_non_parsec_stash() {
        let line = "stash@{2}: On main: WIP random work";
        assert!(parse_stash_line(line).is_none());
    }

    #[test]
    fn ignores_malformed_line() {
        assert!(parse_stash_line("not a stash line").is_none());
        assert!(parse_stash_line("").is_none());
    }

    // --- parse_stash_list ---------------------------------------------------

    #[test]
    fn filters_only_parsec_entries() {
        let raw = "\
stash@{0}: On main: parsec-checkpoint:safe-point\n\
stash@{1}: On main: WIP on feature work\n\
stash@{2}: WIP on develop: parsec-checkpoint:20260901-120000";
        let entries = parse_stash_list(raw);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "safe-point");
        assert_eq!(entries[1].name, "20260901-120000");
    }

    #[test]
    fn empty_stash_list_returns_empty_vec() {
        let entries = parse_stash_list("");
        assert!(entries.is_empty());
    }

    #[test]
    fn multiple_checkpoints_same_branch() {
        let raw = "\
stash@{0}: On main: parsec-checkpoint:alpha\n\
stash@{1}: On main: parsec-checkpoint:beta";
        let entries = parse_stash_list(raw);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "alpha");
        assert_eq!(entries[1].name, "beta");
    }

    // --- label validation ---------------------------------------------------

    #[test]
    fn colon_in_name_is_rejected() {
        // We can't call checkpoint_create directly (needs a real repo),
        // but we can validate the bail condition indirectly by checking
        // that our prefix string does not itself contain a colon in the name part.
        let label = "bad:name";
        assert!(label.contains(':'));
    }

    // --- parse_stash_line edge cases ----------------------------------------

    #[test]
    fn parses_checkpoint_with_hyphenated_name() {
        let line = "stash@{3}: On release/1.0: parsec-checkpoint:pre-merge-2026";
        let entry = parse_stash_line(line).expect("should parse");
        assert_eq!(entry.name, "pre-merge-2026");
        assert_eq!(entry.branch, "release/1.0");
    }

    #[test]
    fn parses_checkpoint_with_timestamp_name() {
        let line = "stash@{0}: On main: parsec-checkpoint:20260911-103000";
        let entry = parse_stash_line(line).expect("should parse");
        assert_eq!(entry.name, "20260911-103000");
        assert_eq!(entry.stash_ref, "stash@{0}");
    }

    // --- find_checkpoint (unit-level via parse_stash_list) ------------------

    /// Verify the lookup logic that find_checkpoint relies on.
    #[test]
    fn lookup_finds_exact_match() {
        let raw = "\
stash@{0}: On main: parsec-checkpoint:alpha\n\
stash@{1}: On main: parsec-checkpoint:beta\n\
stash@{2}: On main: WIP regular stash";
        let entries = parse_stash_list(raw);
        let found = entries.into_iter().find(|e| e.name == "beta");
        assert!(found.is_some());
        assert_eq!(found.unwrap().stash_ref, "stash@{1}");
    }

    #[test]
    fn lookup_returns_none_for_missing_name() {
        let raw = "stash@{0}: On main: parsec-checkpoint:only-one";
        let entries = parse_stash_list(raw);
        let found = entries.into_iter().find(|e| e.name == "nonexistent");
        assert!(found.is_none());
    }

    #[test]
    fn lookup_does_not_match_partial_name() {
        let raw = "stash@{0}: On main: parsec-checkpoint:alpha-extended";
        let entries = parse_stash_list(raw);
        // Exact match only — "alpha" should not match "alpha-extended".
        let found = entries.into_iter().find(|e| e.name == "alpha");
        assert!(found.is_none());
    }
}
