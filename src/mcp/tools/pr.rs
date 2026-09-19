//! MCP tool handler for `pr_status`.
//!
//! Phase 26 (#293): wires `pr_status` to the real GitHub PR state via the
//! `gh` CLI. Read-only; requires `gh` on PATH with valid credentials (the
//! keyring or `GITHUB_TOKEN` / `GH_TOKEN` env vars).
//!
//! Auth tokens may also be injected via `McpContext.github_token` (delegated
//! PAT from the caller) — see `docs/mcp/auth.md`. When a delegated token is
//! present it is forwarded to `gh` via `GH_TOKEN` so the tool stays
//! credential-agnostic.
//!
//! ## Output shape
//!
//! ```json
//! {
//!   "ticket": "PROJ-123",
//!   "pr_number": 42,
//!   "pr_url": "https://github.com/org/repo/pull/42",
//!   "state": "open",
//!   "draft": false,
//!   "mergeable": true,
//!   "review_state": "approved",
//!   "approvals": 2,
//!   "change_requests": 0,
//!   "ci_overall": null
//! }
//! ```
//!
//! `ci_overall` is reserved for a future phase that bridges the async GitHub
//! check-runs client; it is always `null` in Phase 26.

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::ParsecConfig;
use crate::mcp::McpContext;
use crate::worktree::WorktreeManager;

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

/// Raw PR fields returned by `gh pr view --json`.
#[derive(Debug, Deserialize)]
struct GhPrView {
    number: u64,
    url: String,
    state: String,
    #[serde(rename = "isDraft")]
    is_draft: bool,
    mergeable: String,
    #[serde(rename = "reviewDecision")]
    review_decision: Option<String>,
    reviews: Vec<GhReview>,
}

/// A single GitHub PR review entry.
#[derive(Debug, Deserialize)]
struct GhReview {
    state: String,
}

/// Call `gh pr view` for the given branch.
///
/// When `token` is supplied it is forwarded as `GH_TOKEN` so the delegated
/// PAT takes precedence over any ambient credential.
fn fetch_pr(repo: &Path, branch: &str, token: Option<&str>) -> Result<GhPrView> {
    let mut cmd = Command::new("gh");
    cmd.args([
        "pr",
        "view",
        branch,
        "--json",
        "number,url,state,isDraft,mergeable,reviewDecision,reviews",
    ])
    .current_dir(repo);

    if let Some(t) = token {
        cmd.env("GH_TOKEN", t);
    }

    let output = cmd.output().context("failed to spawn 'gh pr view'")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "gh pr view failed for branch '{}': {}",
            branch,
            stderr.trim()
        );
    }

    let raw = String::from_utf8(output.stdout).context("gh pr view produced non-UTF-8 output")?;
    serde_json::from_str::<GhPrView>(&raw).context("failed to parse gh pr view JSON")
}

/// Map the `mergeable` string from the GitHub GraphQL enum to a bool.
///
/// `MERGEABLE` → `true`; `CONFLICTING` / `UNKNOWN` → `false`.
fn parse_mergeable(s: &str) -> bool {
    s.eq_ignore_ascii_case("MERGEABLE")
}

// ---------------------------------------------------------------------------
// Tool handler
// ---------------------------------------------------------------------------

/// `pr_status` — GitHub PR state, review approvals, and merge readiness.
///
/// # Errors
///
/// Returns an error when:
/// - `ticket` is missing or fails validation.
/// - No worktree is registered for the given ticket.
/// - `gh` is not on PATH or has no valid credentials.
/// - The branch has no open PR on GitHub.
pub fn status(ctx: &McpContext, input: Value) -> Result<Value> {
    let ticket = input
        .get("ticket")
        .and_then(Value::as_str)
        .context("pr_status requires 'ticket'")?;

    crate::worktree::validate_ticket_id(ticket)?;

    let repo = repo_path(ctx, &input);
    let config = ParsecConfig::load().context("failed to load parsec config")?;
    let manager = WorktreeManager::new(&repo, &config)?;
    let workspaces = manager.list()?;

    let ws = workspaces
        .iter()
        .find(|w| w.ticket.eq_ignore_ascii_case(ticket))
        .with_context(|| format!("no worktree registered for ticket '{ticket}'"))?;

    let token = ctx.github_token.as_ref().map(|t| t.expose_secret());
    let pr = fetch_pr(&repo, &ws.branch, token)?;

    let approvals = pr
        .reviews
        .iter()
        .filter(|r| r.state.eq_ignore_ascii_case("APPROVED"))
        .count() as u64;

    let change_requests = pr
        .reviews
        .iter()
        .filter(|r| r.state.eq_ignore_ascii_case("CHANGES_REQUESTED"))
        .count() as u64;

    // Normalise reviewDecision: APPROVED → "approved", CHANGES_REQUESTED →
    // "changes_requested", null/absent → "pending".
    let review_state = pr
        .review_decision
        .as_deref()
        .map(|s| s.to_lowercase().replace('-', "_"))
        .unwrap_or_else(|| "pending".to_string());

    Ok(json!({
        "ticket": ticket,
        "pr_number": pr.number,
        "pr_url": pr.url,
        "state": pr.state.to_lowercase(),
        "draft": pr.is_draft,
        "mergeable": parse_mergeable(&pr.mergeable),
        "review_state": review_state,
        "approvals": approvals,
        "change_requests": change_requests,
        "ci_overall": null,
    }))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_mergeable_handles_all_variants() {
        assert!(parse_mergeable("MERGEABLE"));
        assert!(parse_mergeable("mergeable")); // case-insensitive
        assert!(!parse_mergeable("CONFLICTING"));
        assert!(!parse_mergeable("UNKNOWN"));
        assert!(!parse_mergeable(""));
    }

    #[test]
    fn pr_status_rejects_missing_ticket() {
        let ctx = McpContext::from_cwd(false).expect("context");
        let err = status(&ctx, json!({})).expect_err("should fail without ticket");
        assert!(
            err.to_string().contains("ticket"),
            "error should mention 'ticket': {err}"
        );
    }

    #[test]
    fn pr_status_rejects_invalid_ticket() {
        let ctx = McpContext::from_cwd(false).expect("context");
        // Shell meta-character in ticket should be rejected by validate_ticket_id.
        let err = status(&ctx, json!({"ticket": "bad;ticket"}))
            .expect_err("shell meta in ticket should fail");
        assert!(
            err.to_string().contains("ticket") || err.to_string().contains("unsafe"),
            "error should mention validation failure: {err}"
        );
    }

    #[test]
    fn pr_status_fails_for_nonexistent_worktree() {
        // Use a temp git repo with no registered worktrees.
        let dir = tempfile::tempdir().expect("temp dir");
        crate::git::run(dir.path(), &["init"]).expect("git init");
        crate::worktree::ParsecState::default()
            .save(dir.path())
            .expect("state save");

        let ctx = McpContext {
            repo_path: dir.path().to_path_buf(),
            github_token: None,
            github_scopes: None,
            dry_run: false,
        };

        let err =
            status(&ctx, json!({"ticket": "PROJ-1"})).expect_err("non-existent ticket should fail");
        assert!(
            err.to_string().contains("PROJ-1") || err.to_string().contains("worktree"),
            "error should mention missing worktree: {err}"
        );
    }
}
