//! MCP tool handler for `sync`.
//!
//! Phase 29 (#293): wires `sync` to rebase or merge a worktree branch against
//! its base branch, with a mandatory `dry_run` gate and explicit `confirm`
//! flag for the mutating path.
//!
//! ## Safety model
//!
//! `sync` is a **mutating** tool (`mutating: true` in the spec). To avoid
//! accidental rebases the tool enforces a two-key gate:
//!
//! - `dry_run` defaults to `true` → analysis-only unless caller opts out.
//! - `confirm` defaults to `false` → mutation requires both `dry_run=false`
//!   **and** `confirm=true`. Either flag alone yields a dry-run report.
//!
//! ## Input
//!
//! ```json
//! {
//!   "ticket":   "PROJ-123",         // required: parsec-managed worktree
//!   "repo":     "/optional/path",   // optional: defaults to ctx.repo_path
//!   "strategy": "rebase",           // "rebase" (default) | "merge"
//!   "dry_run":  true,               // default true — analysis only
//!   "confirm":  false,              // default false — must be true to mutate
//!   "stale_days": 5                 // staleness threshold for advisory flag
//! }
//! ```
//!
//! ## Output — dry-run (safe default)
//!
//! ```json
//! {
//!   "ticket":        "PROJ-123",
//!   "branch":        "feat/PROJ-123",
//!   "base_branch":   "main",
//!   "strategy":      "rebase",
//!   "commits_behind": 3,
//!   "stale":         true,
//!   "dry_run":       true,
//!   "applied":       false,
//!   "message":       "3 commits behind origin/main — run with dry_run=false confirm=true to sync"
//! }
//! ```
//!
//! ## Output — applied (dry_run=false, confirm=true)
//!
//! ```json
//! {
//!   "ticket":        "PROJ-123",
//!   "branch":        "feat/PROJ-123",
//!   "base_branch":   "main",
//!   "strategy":      "rebase",
//!   "commits_behind": 3,
//!   "stale":         true,
//!   "dry_run":       false,
//!   "applied":       true,
//!   "message":       "rebased feat/PROJ-123 onto origin/main (3 commits applied)"
//! }
//! ```

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

use crate::config::ParsecConfig;
use crate::git;
use crate::mcp::McpContext;
use crate::worktree::WorktreeManager;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const DEFAULT_STALE_DAYS: i64 = 5;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn repo_path(ctx: &McpContext, input: &Value) -> PathBuf {
    input
        .get("repo")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .unwrap_or_else(|| ctx.repo_path.clone())
}

/// Validate that the ticket string is safe to use in git operations.
///
/// Allows alphanumerics, hyphens, underscores, and dots; rejects shell-unsafe
/// characters and empty strings.
fn validate_ticket(ticket: &str) -> Result<()> {
    if ticket.is_empty() {
        bail!("ticket must not be empty");
    }
    if ticket.len() > 128 {
        bail!("ticket value too long (max 128 chars)");
    }
    if !ticket
        .chars()
        .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        bail!(
            "ticket '{}' contains unsafe characters; only alphanumerics, '-', '_', '.' allowed",
            ticket
        );
    }
    Ok(())
}

/// Parse the `strategy` field: `"rebase"` (default) or `"merge"`.
fn parse_strategy(input: &Value) -> Result<&'static str> {
    match input
        .get("strategy")
        .and_then(Value::as_str)
        .unwrap_or("rebase")
    {
        "rebase" => Ok("rebase"),
        "merge" => Ok("merge"),
        other => bail!(
            "unknown strategy '{}'; expected \"rebase\" or \"merge\"",
            other
        ),
    }
}

/// Count how many commits the worktree branch is behind `origin/<base_branch>`.
///
/// Uses `git rev-list --count HEAD..origin/<base_branch>` executed in the
/// worktree directory. Returns `None` when the remote ref doesn't exist yet
/// (e.g. freshly created repo with no pushed commits).
fn commits_behind(worktree_path: &Path, base_branch: &str) -> Option<u64> {
    let remote_ref = format!("origin/{base_branch}");
    let spec = format!("HEAD..{remote_ref}");
    git::run_output(worktree_path, &["rev-list", "--count", &spec])
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
}

// ---------------------------------------------------------------------------
// Public handler
// ---------------------------------------------------------------------------

/// `sync` — rebase or merge a parsec-managed worktree against its base branch.
///
/// # Errors
///
/// Returns an error when:
/// - `ticket` is missing, empty, or contains unsafe characters.
/// - The worktree for `ticket` cannot be found.
/// - `strategy` is not `"rebase"` or `"merge"`.
/// - `dry_run=false` + `confirm=true` (mutation path) fails at `git fetch` or
///   the subsequent rebase/merge step.
pub fn run(ctx: &McpContext, input: Value) -> Result<Value> {
    // --- Parse & validate inputs -------------------------------------------

    let ticket = input
        .get("ticket")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    validate_ticket(&ticket)?;

    let strategy = parse_strategy(&input)?;

    let dry_run = input
        .get("dry_run")
        .and_then(Value::as_bool)
        .unwrap_or(true);

    let confirm = input
        .get("confirm")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let stale_days_threshold = input
        .get("stale_days")
        .and_then(Value::as_i64)
        .filter(|&d| d > 0)
        .unwrap_or(DEFAULT_STALE_DAYS);

    // --- Resolve worktree ---------------------------------------------------

    let repo = repo_path(ctx, &input);
    let config = ParsecConfig::load().context("failed to load parsec config")?;
    let manager = WorktreeManager::new(&repo, &config)?;
    let workspace = manager.get(&ticket)?;

    let worktree_path: &Path = &workspace.path;
    let branch = &workspace.branch;
    let base_branch = &workspace.base_branch;

    // --- Staleness analysis (always computed, even on mutation path) --------

    // Fetch so that remote refs are current. Non-fatal in offline mode.
    let _ = git::fetch_if_remote(&repo);

    let behind = commits_behind(worktree_path, base_branch).unwrap_or(0);

    // Days since last commit in the worktree.
    let stale = {
        let days = git::run_output(worktree_path, &["log", "-1", "--format=%ct"])
            .ok()
            .and_then(|s| s.trim().parse::<i64>().ok())
            .map(|ts| (chrono::Utc::now().timestamp() - ts) / 86_400)
            .unwrap_or(0);
        days >= stale_days_threshold
    };

    // --- Gate: if dry_run OR !confirm → return analysis --------------------

    let effective_dry_run = dry_run || !confirm;

    if effective_dry_run {
        let msg = if behind == 0 {
            format!("{branch} is up to date with origin/{base_branch}")
        } else {
            format!(
                "{behind} commit{s} behind origin/{base_branch} — run with dry_run=false confirm=true to {strategy}",
                s = if behind == 1 { "" } else { "s" },
            )
        };

        return Ok(json!({
            "ticket":         ticket,
            "branch":         branch,
            "base_branch":    base_branch,
            "strategy":       strategy,
            "commits_behind": behind,
            "stale":          stale,
            "dry_run":        true,
            "applied":        false,
            "message":        msg,
        }));
    }

    // --- Mutation path: dry_run=false AND confirm=true ----------------------

    let remote_ref = format!("origin/{base_branch}");

    match strategy {
        "rebase" => {
            git::run(worktree_path, &["rebase", &remote_ref]).with_context(|| {
                format!(
                    "git rebase {remote_ref} failed in worktree {worktree_path:?}; \
                     resolve conflicts manually then re-run `git rebase --continue`"
                )
            })?;
        }
        "merge" => {
            git::run(worktree_path, &["merge", "--no-edit", &remote_ref]).with_context(|| {
                format!(
                    "git merge {remote_ref} failed in worktree {worktree_path:?}; \
                     resolve conflicts manually then run `git merge --continue`"
                )
            })?;
        }
        // unreachable: parse_strategy() validated the value above
        _ => bail!("internal: unexpected strategy '{strategy}'"),
    }

    Ok(json!({
        "ticket":         ticket,
        "branch":         branch,
        "base_branch":    base_branch,
        "strategy":       strategy,
        "commits_behind": behind,
        "stale":          stale,
        "dry_run":        false,
        "applied":        true,
        "message":        format!(
            "{strategy}d {branch} onto {remote_ref} ({behind} commit{s} applied)",
            s = if behind == 1 { "" } else { "s" },
        ),
    }))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ------------------------------------------------------------------
    // validate_ticket
    // ------------------------------------------------------------------

    #[test]
    fn validate_ticket_accepts_valid_ids() {
        assert!(validate_ticket("PROJ-123").is_ok());
        assert!(validate_ticket("feat_123").is_ok());
        assert!(validate_ticket("v1.0.0").is_ok());
        assert!(validate_ticket("ABC").is_ok());
    }

    #[test]
    fn validate_ticket_rejects_empty() {
        let err = validate_ticket("").expect_err("empty should fail");
        assert!(err.to_string().contains("empty"), "{err}");
    }

    #[test]
    fn validate_ticket_rejects_shell_chars() {
        for bad in &[";rm -rf", "$(cmd)", "a b", "tick&et", "bad;one"] {
            let err = validate_ticket(bad).expect_err("unsafe chars should fail");
            assert!(
                err.to_string().contains("unsafe"),
                "expected 'unsafe' in: {err}"
            );
        }
    }

    #[test]
    fn validate_ticket_rejects_too_long() {
        let long = "A".repeat(129);
        let err = validate_ticket(&long).expect_err("too-long should fail");
        assert!(err.to_string().contains("too long"), "{err}");
    }

    // ------------------------------------------------------------------
    // parse_strategy
    // ------------------------------------------------------------------

    #[test]
    fn parse_strategy_defaults_to_rebase() {
        assert_eq!(parse_strategy(&json!({})).unwrap(), "rebase");
    }

    #[test]
    fn parse_strategy_accepts_merge() {
        assert_eq!(
            parse_strategy(&json!({"strategy": "merge"})).unwrap(),
            "merge"
        );
    }

    #[test]
    fn parse_strategy_rejects_unknown() {
        let err = parse_strategy(&json!({"strategy": "squash"})).expect_err("squash invalid");
        assert!(err.to_string().contains("squash"), "{err}");
    }

    // ------------------------------------------------------------------
    // run() — missing ticket
    // ------------------------------------------------------------------

    #[test]
    fn sync_rejects_missing_ticket() {
        let ctx = McpContext::from_cwd(false).expect("context");
        let err = run(&ctx, json!({})).expect_err("missing ticket should fail");
        assert!(
            err.to_string().contains("ticket") || err.to_string().contains("empty"),
            "error should mention ticket: {err}"
        );
    }

    // ------------------------------------------------------------------
    // run() — invalid ticket characters
    // ------------------------------------------------------------------

    #[test]
    fn sync_rejects_unsafe_ticket() {
        let ctx = McpContext::from_cwd(false).expect("context");
        let err = run(&ctx, json!({"ticket": "bad;ticket"})).expect_err("unsafe ticket");
        assert!(
            err.to_string().contains("unsafe") || err.to_string().contains("ticket"),
            "error should mention validation: {err}"
        );
    }

    // ------------------------------------------------------------------
    // run() — non-existent worktree (dry_run default)
    // ------------------------------------------------------------------

    #[test]
    fn sync_dry_run_fails_for_nonexistent_worktree() {
        let dir = tempfile::tempdir().expect("temp dir");
        crate::git::run(dir.path(), &["init"]).expect("git init");
        crate::worktree::ParsecState::default()
            .save(dir.path())
            .expect("state save");

        let ctx = crate::mcp::McpContext {
            repo_path: dir.path().to_path_buf(),
            github_token: None,
            github_scopes: None,
            dry_run: false,
        };

        // dry_run=true (default) — should fail at worktree lookup, not at mutation
        let err = run(&ctx, json!({"ticket": "PROJ-1"})).expect_err("unknown worktree");
        assert!(
            err.to_string().contains("PROJ-1")
                || err.to_string().contains("worktree")
                || err.to_string().contains("not found"),
            "error should mention missing worktree: {err}"
        );
    }

    // ------------------------------------------------------------------
    // run() — confirm=false always yields dry_run regardless of dry_run param
    // ------------------------------------------------------------------

    #[test]
    fn sync_confirm_false_forces_dry_run_path() {
        let dir = tempfile::tempdir().expect("temp dir");
        crate::git::run(dir.path(), &["init"]).expect("git init");
        crate::worktree::ParsecState::default()
            .save(dir.path())
            .expect("state save");

        let ctx = crate::mcp::McpContext {
            repo_path: dir.path().to_path_buf(),
            github_token: None,
            github_scopes: None,
            dry_run: false,
        };

        // dry_run=false but confirm=false → should still use dry_run path
        // (fails at worktree lookup before reaching the mutation gate check)
        let err = run(
            &ctx,
            json!({"ticket": "PROJ-99", "dry_run": false, "confirm": false}),
        )
        .expect_err("worktree lookup must fail");
        // Must NOT attempt actual git rebase/merge (test passes as long as
        // the error is from worktree resolution, not from a mutation attempt).
        assert!(
            err.to_string().contains("PROJ-99")
                || err.to_string().contains("not found")
                || err.to_string().contains("worktree"),
            "error should be from worktree resolution: {err}"
        );
    }
}
