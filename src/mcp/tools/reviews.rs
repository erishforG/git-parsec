//! MCP tool handler for `reviews`.
//!
//! Phase 27 (#293): wires the `reviews` tool to real GitHub PR data via the
//! `gh` CLI. Read-only; requires `gh` on PATH with valid credentials (the
//! keyring or `GITHUB_TOKEN` / `GH_TOKEN` env vars).
//!
//! Auth tokens may also be injected via `McpContext.github_token` (delegated
//! PAT from the caller) — see `docs/mcp/auth.md`.
//!
//! ## Input
//!
//! ```json
//! {
//!   "repo": "/optional/abs/path",
//!   "mode": "authored" | "requested" | "all"
//! }
//! ```
//!
//! `mode` defaults to `"all"`. `"authored"` shows open PRs created by `@me`;
//! `"requested"` shows open PRs where `@me` is a requested reviewer.
//!
//! ## Output shape
//!
//! ```json
//! {
//!   "mode": "all",
//!   "total_count": 2,
//!   "authored_count": 1,
//!   "requested_count": 1,
//!   "entries": [
//!     {
//!       "source": "authored",
//!       "pr_number": 42,
//!       "title": "feat: something",
//!       "state": "open",
//!       "review_decision": "approved",
//!       "approvals": 2,
//!       "changes_requested": 0,
//!       "url": "https://github.com/org/repo/pull/42",
//!       "draft": false
//!     }
//!   ]
//! }
//! ```

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::mcp::McpContext;

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

/// Raw PR list entry returned by `gh pr list --json`.
#[derive(Debug, Deserialize)]
struct GhPrEntry {
    number: u64,
    title: String,
    url: String,
    state: String,
    #[serde(rename = "isDraft")]
    is_draft: bool,
    #[serde(rename = "reviewDecision")]
    review_decision: Option<String>,
    reviews: Vec<GhReview>,
}

/// A single review on a PR.
#[derive(Debug, Deserialize)]
struct GhReview {
    state: String,
}

const PR_FIELDS: &str = "number,title,url,state,isDraft,reviewDecision,reviews";

/// Run `gh pr list` with caller-supplied extra args and return parsed entries.
///
/// When `token` is supplied it is forwarded as `GH_TOKEN` so the delegated
/// PAT takes precedence over any ambient credential.
fn gh_pr_list(repo: &Path, extra_args: &[&str], token: Option<&str>) -> Result<Vec<GhPrEntry>> {
    let mut cmd = Command::new("gh");
    cmd.args(["pr", "list", "--state", "open", "--json", PR_FIELDS])
        .args(extra_args)
        .current_dir(repo);

    if let Some(t) = token {
        cmd.env("GH_TOKEN", t);
    }

    let output = cmd.output().context("failed to spawn 'gh pr list'")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("gh pr list failed: {}", stderr.trim());
    }

    let raw = String::from_utf8(output.stdout).context("gh pr list produced non-UTF-8 output")?;
    serde_json::from_str::<Vec<GhPrEntry>>(&raw).context("failed to parse gh pr list JSON")
}

/// Count APPROVED and CHANGES_REQUESTED reviews from a list.
fn count_reviews(reviews: &[GhReview]) -> (u64, u64) {
    let approvals = reviews
        .iter()
        .filter(|r| r.state.eq_ignore_ascii_case("APPROVED"))
        .count() as u64;
    let changes = reviews
        .iter()
        .filter(|r| r.state.eq_ignore_ascii_case("CHANGES_REQUESTED"))
        .count() as u64;
    (approvals, changes)
}

/// Convert a `GhPrEntry` to the unified output object tagged with `source`.
fn entry_json(pr: GhPrEntry, source: &str) -> Value {
    let (approvals, changes_requested) = count_reviews(&pr.reviews);
    let review_decision = pr
        .review_decision
        .as_deref()
        .map(|s| s.to_lowercase().replace('-', "_"))
        .unwrap_or_else(|| "pending".to_string());

    json!({
        "source": source,
        "pr_number": pr.number,
        "title": pr.title,
        "state": pr.state.to_lowercase(),
        "review_decision": review_decision,
        "approvals": approvals,
        "changes_requested": changes_requested,
        "url": pr.url,
        "draft": pr.is_draft,
    })
}

// ---------------------------------------------------------------------------
// Tool handler
// ---------------------------------------------------------------------------

/// `reviews` — list authored and/or incoming review requests via GitHub.
///
/// # Errors
///
/// Returns an error when:
/// - `mode` is not one of `"authored"`, `"requested"`, or `"all"`.
/// - `gh` is not on PATH or has no valid credentials.
pub fn list(ctx: &McpContext, input: Value) -> Result<Value> {
    let repo = repo_path(ctx, &input);
    let mode = input.get("mode").and_then(Value::as_str).unwrap_or("all");

    if !matches!(mode, "authored" | "requested" | "all") {
        anyhow::bail!(
            "reviews: 'mode' must be \"authored\", \"requested\", or \"all\" (got {mode:?})"
        );
    }

    let token = ctx.github_token.as_ref().map(|t| t.expose_secret());

    // Authored: open PRs created by the authenticated user.
    let authored: Vec<Value> = if matches!(mode, "authored" | "all") {
        gh_pr_list(&repo, &["--author", "@me"], token)?
            .into_iter()
            .map(|pr| entry_json(pr, "authored"))
            .collect()
    } else {
        vec![]
    };

    // Requested: open PRs where the authenticated user is a requested reviewer.
    let requested: Vec<Value> = if matches!(mode, "requested" | "all") {
        gh_pr_list(&repo, &["--search", "review-requested:@me"], token)?
            .into_iter()
            .map(|pr| entry_json(pr, "requested"))
            .collect()
    } else {
        vec![]
    };

    let authored_count = authored.len();
    let requested_count = requested.len();

    let mut entries = authored;
    entries.extend(requested);

    Ok(json!({
        "mode": mode,
        "total_count": entries.len(),
        "authored_count": authored_count,
        "requested_count": requested_count,
        "entries": entries,
    }))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reviews_rejects_invalid_mode() {
        let ctx = McpContext::from_cwd(false).expect("context");
        let err = list(&ctx, json!({"mode": "bogus"})).expect_err("invalid mode should fail");
        assert!(
            err.to_string().contains("mode"),
            "error should mention 'mode': {err}"
        );
    }

    #[test]
    fn reviews_accepts_valid_mode_authored() {
        // Mode validation passes — the result is either Ok (empty list) or
        // a gh error. Either way the error must NOT be a mode-validation error.
        let ctx = McpContext::from_cwd(false).expect("context");
        match list(&ctx, json!({"mode": "authored"})) {
            Ok(v) => assert_eq!(v["mode"], "authored"),
            Err(e) => assert!(
                !e.to_string().contains("must be"),
                "error should not be a mode-validation error: {e}"
            ),
        }
    }

    #[test]
    fn reviews_defaults_to_all_mode() {
        // No `mode` → defaults to "all".
        let ctx = McpContext::from_cwd(false).expect("context");
        match list(&ctx, json!({})) {
            Ok(v) => assert_eq!(v["mode"], "all"),
            Err(e) => assert!(
                !e.to_string().contains("must be"),
                "error should not be a mode-validation error: {e}"
            ),
        }
    }

    #[test]
    fn count_reviews_empty() {
        let (approvals, changes) = count_reviews(&[]);
        assert_eq!(approvals, 0);
        assert_eq!(changes, 0);
    }

    #[test]
    fn count_reviews_mixed() {
        let reviews = vec![
            GhReview {
                state: "APPROVED".to_string(),
            },
            GhReview {
                state: "APPROVED".to_string(),
            },
            GhReview {
                state: "CHANGES_REQUESTED".to_string(),
            },
            GhReview {
                state: "COMMENTED".to_string(),
            },
        ];
        let (approvals, changes) = count_reviews(&reviews);
        assert_eq!(approvals, 2);
        assert_eq!(changes, 1);
    }

    #[test]
    fn entry_json_maps_all_fields() {
        let pr = GhPrEntry {
            number: 42,
            title: "feat: something".to_string(),
            url: "https://github.com/org/repo/pull/42".to_string(),
            state: "OPEN".to_string(),
            is_draft: false,
            review_decision: Some("APPROVED".to_string()),
            reviews: vec![GhReview {
                state: "APPROVED".to_string(),
            }],
        };
        let entry = entry_json(pr, "authored");
        assert_eq!(entry["source"], "authored");
        assert_eq!(entry["pr_number"], 42);
        assert_eq!(entry["state"], "open");
        assert_eq!(entry["review_decision"], "approved");
        assert_eq!(entry["approvals"], 1_u64);
        assert_eq!(entry["changes_requested"], 0_u64);
        assert_eq!(entry["draft"], false);
    }

    #[test]
    fn entry_json_pending_when_no_review_decision() {
        let pr = GhPrEntry {
            number: 7,
            title: "feat: wip".to_string(),
            url: "https://github.com/org/repo/pull/7".to_string(),
            state: "open".to_string(),
            is_draft: true,
            review_decision: None,
            reviews: vec![],
        };
        let entry = entry_json(pr, "requested");
        assert_eq!(entry["review_decision"], "pending");
        assert_eq!(entry["draft"], true);
        assert_eq!(entry["source"], "requested");
    }
}
