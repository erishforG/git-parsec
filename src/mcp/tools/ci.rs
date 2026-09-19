//! MCP tool handler for `ci_status`.
//!
//! Phase 28 (#293): wires `ci_status` to real GitHub Actions check-run data
//! via `gh pr view --json statusCheckRollup`. Read-only; requires `gh` on
//! PATH with valid credentials (keyring or `GITHUB_TOKEN` / `GH_TOKEN`).
//!
//! Auth tokens injected via `McpContext.github_token` are forwarded as
//! `GH_TOKEN` — see `docs/mcp/auth.md`.
//!
//! ## Input
//!
//! ```json
//! { "ticket": "PROJ-123", "repo": "/optional/path", "limit": 10 }
//! ```
//!
//! ## Output shape
//!
//! ```json
//! {
//!   "ticket": "PROJ-123",
//!   "pr_number": 42,
//!   "branch": "feat/PROJ-123",
//!   "overall": "passing",
//!   "check_count": 3,
//!   "checks": [
//!     {
//!       "name": "CI / build",
//!       "status": "completed",
//!       "conclusion": "success",
//!       "started_at": "2026-08-21T00:35:00Z",
//!       "completed_at": "2026-08-21T00:37:00Z",
//!       "details_url": "https://github.com/..."
//!     }
//!   ]
//! }
//! ```
//!
//! `overall` is one of `"passing"`, `"failing"`, `"pending"`, `"no_checks"`.
//!
//! Both GraphQL `CheckRun` (has `status`/`conclusion`) and `StatusContext`
//! (has `state`) objects are handled uniformly.

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::ParsecConfig;
use crate::mcp::McpContext;
use crate::worktree::WorktreeManager;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const DEFAULT_LIMIT: usize = 10;

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

// ---------------------------------------------------------------------------
// gh CLI types
// ---------------------------------------------------------------------------

/// Raw response from `gh pr view --json number,statusCheckRollup`.
#[derive(Debug, Deserialize)]
struct GhPrCiView {
    number: u64,
    /// Mixed `CheckRun` + `StatusContext` entries from the GraphQL rollup.
    #[serde(rename = "statusCheckRollup", default)]
    status_check_rollup: Vec<GhCheckEntry>,
}

/// Unified deserialization for both GraphQL `CheckRun` and `StatusContext`
/// objects that appear in `statusCheckRollup`.
///
/// - `CheckRun` carries `name`, `status`, `conclusion`, `startedAt`,
///   `completedAt`, `detailsUrl`.
/// - `StatusContext` carries `context` (display name), `state`
///   (terminal result), and `targetUrl`.
#[derive(Debug, Deserialize)]
struct GhCheckEntry {
    /// Display name (CheckRun). Empty for StatusContext.
    #[serde(default)]
    name: String,
    /// Context key used as display name for StatusContext. Empty for CheckRun.
    #[serde(default)]
    context: String,
    /// CheckRun lifecycle: "COMPLETED", "IN_PROGRESS", "QUEUED", …
    #[serde(default)]
    status: String,
    /// CheckRun outcome: "SUCCESS", "FAILURE", "CANCELLED", "SKIPPED", …
    conclusion: Option<String>,
    /// StatusContext terminal state: "SUCCESS", "FAILURE", "PENDING", "ERROR".
    state: Option<String>,
    #[serde(rename = "startedAt")]
    started_at: Option<String>,
    #[serde(rename = "completedAt")]
    completed_at: Option<String>,
    #[serde(rename = "detailsUrl")]
    details_url: Option<String>,
    #[serde(rename = "targetUrl")]
    target_url: Option<String>,
}

impl GhCheckEntry {
    fn display_name(&self) -> &str {
        if !self.name.is_empty() {
            &self.name
        } else {
            &self.context
        }
    }

    /// Normalised lifecycle status (lowercase).
    fn normalised_status(&self) -> &'static str {
        if !self.status.is_empty() {
            normalise_check_status(&self.status)
        } else {
            // StatusContext: derive lifecycle from terminal state.
            match self.state.as_deref() {
                Some("SUCCESS") | Some("FAILURE") | Some("ERROR") => "completed",
                Some("PENDING") => "pending",
                _ => "unknown",
            }
        }
    }

    /// Normalised conclusion string (lowercase), if available.
    fn normalised_conclusion(&self) -> Option<String> {
        if let Some(c) = &self.conclusion {
            Some(c.to_ascii_lowercase())
        } else {
            // StatusContext: map terminal state to a conclusion-like value.
            match self.state.as_deref() {
                Some("SUCCESS") => Some("success".to_owned()),
                Some("FAILURE") | Some("ERROR") => Some("failure".to_owned()),
                _ => None,
            }
        }
    }

    fn url(&self) -> Option<&str> {
        self.details_url.as_deref().or(self.target_url.as_deref())
    }
}

/// Map a CheckRun `status` string to a normalised lowercase value.
fn normalise_check_status(s: &str) -> &'static str {
    let upper = s.to_ascii_uppercase();
    match upper.as_str() {
        "COMPLETED" => "completed",
        "IN_PROGRESS" => "in_progress",
        "QUEUED" | "PENDING" | "REQUESTED" | "WAITING" => "pending",
        _ => "unknown",
    }
}

/// Derive the overall CI status from all rollup entries.
///
/// - Any failure/timeout → `"failing"`
/// - All concluded successfully (success/neutral/skipped) → `"passing"`
/// - Some still running or queued → `"pending"`
/// - No entries → `"no_checks"`
fn derive_overall(checks: &[GhCheckEntry]) -> &'static str {
    if checks.is_empty() {
        return "no_checks";
    }
    if checks.iter().any(|c| {
        matches!(
            c.normalised_conclusion().as_deref(),
            Some("failure") | Some("timed_out") | Some("action_required") | Some("error")
        )
    }) {
        return "failing";
    }
    if checks.iter().all(|c| {
        matches!(
            c.normalised_conclusion().as_deref(),
            Some("success") | Some("neutral") | Some("skipped")
        )
    }) {
        return "passing";
    }
    "pending"
}

// ---------------------------------------------------------------------------
// gh CLI call
// ---------------------------------------------------------------------------

fn fetch_ci(repo: &Path, branch: &str, token: Option<&str>) -> Result<GhPrCiView> {
    let mut cmd = Command::new("gh");
    cmd.args(["pr", "view", branch, "--json", "number,statusCheckRollup"])
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
    serde_json::from_str::<GhPrCiView>(&raw).context("failed to parse gh pr view JSON")
}

// ---------------------------------------------------------------------------
// Tool handler
// ---------------------------------------------------------------------------

/// `ci_status` — GitHub Actions check-run status for a worktree branch.
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
        .context("ci_status requires 'ticket'")?;

    crate::worktree::validate_ticket_id(ticket)?;

    let limit = input
        .get("limit")
        .and_then(Value::as_u64)
        .map(|v| v as usize)
        .unwrap_or(DEFAULT_LIMIT)
        .clamp(1, 100);

    let repo = repo_path(ctx, &input);
    let config = ParsecConfig::load().context("failed to load parsec config")?;
    let manager = WorktreeManager::new(&repo, &config)?;
    let workspaces = manager.list()?;

    let ws = workspaces
        .iter()
        .find(|w| w.ticket.eq_ignore_ascii_case(ticket))
        .with_context(|| format!("no worktree registered for ticket '{ticket}'"))?;

    let token = ctx.github_token.as_ref().map(|t| t.expose_secret());
    let mut view = fetch_ci(&repo, &ws.branch, token)?;

    // Apply caller-supplied limit before serialisation.
    view.status_check_rollup.truncate(limit);

    let overall = derive_overall(&view.status_check_rollup);
    let checks: Vec<Value> = view
        .status_check_rollup
        .iter()
        .map(|c| {
            json!({
                "name": c.display_name(),
                "status": c.normalised_status(),
                "conclusion": c.normalised_conclusion(),
                "started_at": c.started_at,
                "completed_at": c.completed_at,
                "details_url": c.url(),
            })
        })
        .collect();

    Ok(json!({
        "ticket": ticket,
        "pr_number": view.number,
        "branch": ws.branch,
        "overall": overall,
        "check_count": checks.len(),
        "checks": checks,
    }))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::McpContext;

    fn make_check(name: &str, status: &str, conclusion: Option<&str>) -> GhCheckEntry {
        GhCheckEntry {
            name: name.to_owned(),
            context: String::new(),
            status: status.to_owned(),
            conclusion: conclusion.map(str::to_owned),
            state: None,
            started_at: None,
            completed_at: None,
            details_url: None,
            target_url: None,
        }
    }

    fn make_status_context(context: &str, state: &str) -> GhCheckEntry {
        GhCheckEntry {
            name: String::new(),
            context: context.to_owned(),
            status: String::new(),
            conclusion: None,
            state: Some(state.to_owned()),
            started_at: None,
            completed_at: None,
            details_url: None,
            target_url: None,
        }
    }

    #[test]
    fn derive_overall_no_checks() {
        assert_eq!(derive_overall(&[]), "no_checks");
    }

    #[test]
    fn derive_overall_all_passing() {
        let checks = vec![
            make_check("build", "COMPLETED", Some("SUCCESS")),
            make_check("test", "COMPLETED", Some("SUCCESS")),
        ];
        assert_eq!(derive_overall(&checks), "passing");
    }

    #[test]
    fn derive_overall_any_failure() {
        let checks = vec![
            make_check("build", "COMPLETED", Some("SUCCESS")),
            make_check("test", "COMPLETED", Some("FAILURE")),
        ];
        assert_eq!(derive_overall(&checks), "failing");
    }

    #[test]
    fn derive_overall_in_progress_is_pending() {
        let checks = vec![
            make_check("build", "COMPLETED", Some("SUCCESS")),
            make_check("test", "IN_PROGRESS", None),
        ];
        assert_eq!(derive_overall(&checks), "pending");
    }

    #[test]
    fn derive_overall_skipped_counts_as_success() {
        let checks = vec![
            make_check("build", "COMPLETED", Some("SUCCESS")),
            make_check("lint", "COMPLETED", Some("SKIPPED")),
        ];
        assert_eq!(derive_overall(&checks), "passing");
    }

    #[test]
    fn status_context_failure_maps_to_failing() {
        let checks = vec![make_status_context("travis-ci", "FAILURE")];
        assert_eq!(derive_overall(&checks), "failing");
    }

    #[test]
    fn status_context_success_maps_to_passing() {
        let checks = vec![make_status_context("travis-ci", "SUCCESS")];
        assert_eq!(derive_overall(&checks), "passing");
    }

    #[test]
    fn normalise_check_status_variants() {
        assert_eq!(normalise_check_status("COMPLETED"), "completed");
        assert_eq!(normalise_check_status("IN_PROGRESS"), "in_progress");
        assert_eq!(normalise_check_status("QUEUED"), "pending");
        assert_eq!(normalise_check_status("WAITING"), "pending");
        assert_eq!(normalise_check_status("REQUESTED"), "pending");
        assert_eq!(normalise_check_status("UNKNOWN_VAL"), "unknown");
    }

    #[test]
    fn ci_status_rejects_missing_ticket() {
        let ctx = McpContext::from_cwd(false).expect("context");
        let err = status(&ctx, json!({})).expect_err("missing ticket should fail");
        assert!(
            err.to_string().contains("ticket"),
            "error should mention 'ticket': {err}"
        );
    }

    #[test]
    fn ci_status_rejects_invalid_ticket() {
        let ctx = McpContext::from_cwd(false).expect("context");
        let err =
            status(&ctx, json!({"ticket": "bad;ticket"})).expect_err("invalid ticket should fail");
        assert!(
            err.to_string().contains("ticket") || err.to_string().contains("unsafe"),
            "error should mention validation failure: {err}"
        );
    }

    #[test]
    fn ci_status_fails_for_nonexistent_worktree() {
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
