//! `parsec smartlog` (alias `sl`) — visualize active worktrees as a commit DAG.
//!
//! Issue #245
//!
//! Phase 1 (PR #305 skeleton):
//! - Collect every active worktree via [`WorktreeManager`]
//! - Read each worktree's commits since its base branch (`base..branch`)
//! - Render as ASCII tree, or emit JSON
//!
//! Phase 2 (PR #327 — PR/CI overlay):
//! - For each worktree, look up the GitHub PR by branch name and attach a
//!   compact overlay ([`SmartlogPrOverlay`]) describing the PR number, state,
//!   CI status and review status.
//! - Overlay is best-effort: missing token / no PR / network errors all
//!   degrade gracefully to "no overlay" without failing the command.
//! - Users can opt out with `--no-overlay` for a fully offline run.
//!
//! Phase 3 (this PR — filter · color · stack indicators):
//! - `--worktree <pattern>`: show only worktrees whose ticket or branch contains
//!   the pattern (case-insensitive substring match).
//! - ANSI color in the PR/CI badge: green=success, red=failure, yellow=pending,
//!   blue=open PR, dim=draft. Automatically disabled when `NO_COLOR` is set or
//!   stdout is not a TTY.
//! - Stack indicator: when a worktree's base branch is itself another active
//!   worktree's branch, annotate it with `⤷ stacked on <ticket>` so stacked-PR
//!   flows are immediately visible.

use std::collections::{BTreeMap, HashMap};
use std::io::IsTerminal as _;
use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::config::ParsecConfig;
use crate::git;
use crate::github::GitHubClient;
use crate::output::Mode;
use crate::worktree::WorktreeManager;

/// Default number of commits per worktree shown in the DAG.
const DEFAULT_DEPTH: usize = 10;

/// One worktree's row in the smartlog output.
#[derive(Debug, Clone, Serialize)]
pub struct SmartlogNode {
    pub ticket: String,
    pub ticket_title: Option<String>,
    pub branch: String,
    pub base_branch: String,
    pub worktree_path: PathBuf,
    pub commits: Vec<CommitSummary>,
    /// PR overlay — populated by Phase 2 when a matching GitHub PR is found.
    /// Omitted from JSON entirely when no PR was attached (skip_serializing_if).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pr: Option<SmartlogPrOverlay>,
    /// CI overlay — reserved for a follow-up that emits per-check detail
    /// (Phase 2 folds the CI summary into [`SmartlogPrOverlay::ci_status`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ci: Option<serde_json::Value>,
}

/// Compact PR/CI summary attached to a smartlog row.
///
/// Subset of [`crate::github::PrStatus`] kept intentionally small: only the
/// fields that fit on the one-line ticket row in the ASCII renderer, plus the
/// browse URL so JSON consumers can click through. CI detail (per-check) is
/// out of scope here — `parsec ci` already prints that view.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SmartlogPrOverlay {
    pub number: u64,
    /// `open` / `closed` / `merged` / `draft` / `unknown`.
    pub state: String,
    /// `success` / `failure` / `pending` / `unknown`.
    pub ci_status: String,
    /// `approved` / `changes_requested` / `pending` / `no reviews`.
    pub review_status: String,
    pub url: String,
}

/// Single commit in a worktree's diff against its base.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CommitSummary {
    pub sha_short: String,
    pub subject: String,
    pub author: String,
    pub timestamp: DateTime<Utc>,
}

/// Entry point for the `smartlog` subcommand.
pub async fn smartlog(
    repo: &Path,
    depth: Option<usize>,
    no_overlay: bool,
    worktree_filter: Option<&str>,
    mode: Mode,
) -> Result<()> {
    let depth = depth.unwrap_or(DEFAULT_DEPTH);
    let config = ParsecConfig::load()?;
    let manager = WorktreeManager::new(repo, &config)?;
    let workspaces = manager.list()?;

    let mut nodes = Vec::with_capacity(workspaces.len());
    for ws in workspaces {
        let commits = collect_commits(&ws.path, &ws.base_branch, &ws.branch, depth)
            // Soft-fail per worktree: a corrupt worktree shouldn't take the whole
            // command down. Empty list is rendered as "(no commits)" instead.
            .unwrap_or_default();
        nodes.push(SmartlogNode {
            ticket: ws.ticket,
            ticket_title: ws.ticket_title,
            branch: ws.branch,
            base_branch: ws.base_branch,
            worktree_path: ws.path,
            commits,
            pr: None,
            ci: None,
        });
    }

    // Phase 3: apply --worktree filter (case-insensitive substring match on
    // ticket or branch name).
    if let Some(pat) = worktree_filter {
        let pat_lower = pat.to_lowercase();
        nodes.retain(|n| {
            n.ticket.to_lowercase().contains(&pat_lower)
                || n.branch.to_lowercase().contains(&pat_lower)
        });
    }

    if !no_overlay {
        attach_pr_overlay(repo, &config, &mut nodes).await;
    }

    let color = color_enabled();
    match mode {
        Mode::Json => {
            println!("{}", serde_json::to_string_pretty(&nodes)?);
        }
        _ => {
            print!("{}", render_text(&nodes, color));
        }
    }
    Ok(())
}

/// Look up each node's PR via GitHub and populate `node.pr`.
///
/// Best-effort: no token, unknown remote host, or any HTTP failure all
/// degrade to "no overlay" silently (the operator can re-run with
/// `parsec pr status` for a full error report). Network errors are logged to
/// stderr at info-level via `eprintln!` so a flaky run doesn't look like a
/// silent bug, but they never fail the whole command.
async fn attach_pr_overlay(repo: &Path, config: &ParsecConfig, nodes: &mut [SmartlogNode]) {
    if nodes.is_empty() {
        return;
    }
    let remote_url = match git::run_output(repo, &["remote", "get-url", "origin"]) {
        Ok(url) => url.trim().to_string(),
        Err(_) => return, // no origin → nothing to overlay
    };
    let client = match GitHubClient::new(&remote_url, config) {
        Ok(Some(c)) => c,
        // Either non-GitHub remote, no token, or a parse error — all of which
        // mean "skip overlay" rather than fail.
        _ => return,
    };

    for node in nodes.iter_mut() {
        match fetch_overlay(&client, &node.branch).await {
            Ok(Some(overlay)) => node.pr = Some(overlay),
            Ok(None) => {} // no open PR for this branch
            Err(e) => {
                eprintln!(
                    "smartlog: GitHub overlay failed for {} ({}): {}",
                    node.ticket, node.branch, e
                );
            }
        }
    }
}

/// Resolve a single branch to a [`SmartlogPrOverlay`], or `None` if no open PR.
async fn fetch_overlay(client: &GitHubClient, branch: &str) -> Result<Option<SmartlogPrOverlay>> {
    let pr_num = match client.find_pr_by_branch(branch).await? {
        Some(n) => n,
        None => return Ok(None),
    };
    let status = client.get_pr_status(pr_num).await?;
    Ok(Some(SmartlogPrOverlay {
        number: status.number,
        state: status.state,
        ci_status: status.ci_status,
        review_status: status.review_status,
        url: status.url,
    }))
}

/// Read commits in `base..branch` from a worktree, capped at `depth`.
///
/// Pure shell-out to `git log` — no `git2` dependency, matches the rest of the
/// `git/` module's style. Returns empty `Vec` (not error) when range is empty
/// or git refuses to walk (e.g., orphan branch).
fn collect_commits(
    worktree: &Path,
    base: &str,
    branch: &str,
    depth: usize,
) -> Result<Vec<CommitSummary>> {
    let range = format!("{}..{}", base, branch);
    let limit = format!("-n{}", depth);
    let raw = git::run_output(
        worktree,
        &["log", &range, "--pretty=format:%h\t%s\t%an\t%aI", &limit],
    )?;
    Ok(raw.lines().filter_map(parse_commit_line).collect())
}

/// Format a one-line PR/CI summary for the ASCII tree.
///
/// Glyphs match `parsec pr status` / `parsec ci` conventions so users see the
/// same vocabulary across commands:
/// - state: `open` → `●`, `merged`/`closed` → `✓`, `draft` → `○`, other → `?`
/// - CI: `success` → `✓ CI` (green), `failure` → `✗ CI` (red),
///   `pending` → `● CI` (yellow), else `? CI`
/// - review: `approved` → `✓ approved` (green), `changes_requested` → `✗ changes` (red),
///   `pending` → `● review` (yellow), `no reviews` → omitted
///
/// `color` enables ANSI SGR codes. Pass `false` in tests / piped output.
fn format_pr_badge(pr: &SmartlogPrOverlay, color: bool) -> String {
    // ANSI SGR codes used:  32=green  31=red  33=yellow  34=blue  2=dim
    let (state_glyph, state_str) = match pr.state.as_str() {
        "open" => (
            ansi_wrap(34, "●", color), // blue
            ansi_wrap(34, &pr.state, color),
        ),
        "merged" => (
            ansi_wrap(32, "✓", color), // green
            ansi_wrap(32, &pr.state, color),
        ),
        "closed" => (
            ansi_wrap(2, "✓", color), // dim
            ansi_wrap(2, &pr.state, color),
        ),
        "draft" => (
            ansi_wrap(2, "○", color), // dim
            ansi_wrap(2, &pr.state, color),
        ),
        _ => ("?".to_string(), pr.state.clone()),
    };
    let ci = match pr.ci_status.as_str() {
        "success" => ansi_wrap(32, "✓ CI", color), // green
        "failure" => ansi_wrap(31, "✗ CI", color), // red
        "pending" => ansi_wrap(33, "● CI", color), // yellow
        _ => "? CI".to_string(),
    };
    let mut out = format!("[PR #{} {} {} {}]", pr.number, state_glyph, state_str, ci);
    let review = match pr.review_status.as_str() {
        "approved" => Some(ansi_wrap(32, "✓ approved", color)),
        "changes_requested" => Some(ansi_wrap(31, "✗ changes", color)),
        "pending" => Some(ansi_wrap(33, "● review", color)),
        _ => None, // "no reviews" or unknown → omit
    };
    if let Some(r) = review {
        // Strip the closing bracket, append review, re-close.
        out.pop();
        out.push_str(&format!(" {}]", r));
    }
    out
}

/// Parse a single tab-separated line emitted by our `git log --pretty` format.
///
/// Format: `<sha_short>\t<subject>\t<author_name>\t<author_iso8601>`.
/// Any line that doesn't conform is silently dropped; this keeps the parser
/// resilient to commit messages containing tabs (we splitn by 4 so the first
/// three tabs are guaranteed to be the field separators).
fn parse_commit_line(line: &str) -> Option<CommitSummary> {
    let mut parts = line.splitn(4, '\t');
    let sha_short = parts.next()?.trim().to_string();
    let subject = parts.next()?.to_string();
    let author = parts.next()?.to_string();
    let ts_raw = parts.next()?.trim();
    let timestamp = DateTime::parse_from_rfc3339(ts_raw)
        .ok()?
        .with_timezone(&Utc);
    if sha_short.is_empty() {
        return None;
    }
    Some(CommitSummary {
        sha_short,
        subject,
        author,
        timestamp,
    })
}

/// Render an ASCII commit DAG, grouped by base branch.
///
/// Returns the rendered string (instead of printing) so it's testable. Empty
/// node list returns a single explanatory line.
///
/// `color` enables ANSI escape codes in the PR/CI badge. Pass `false` in tests
/// or when `NO_COLOR` is set to keep output predictable.
pub fn render_text(nodes: &[SmartlogNode], color: bool) -> String {
    if nodes.is_empty() {
        return "No active worktrees. Run `parsec start <ticket>` to create one.\n".to_string();
    }

    // Phase 3: build branch → ticket lookup for stack indicator.
    let branch_to_ticket: HashMap<&str, &str> = nodes
        .iter()
        .map(|n| (n.branch.as_str(), n.ticket.as_str()))
        .collect();

    let mut by_base: BTreeMap<String, Vec<&SmartlogNode>> = BTreeMap::new();
    for n in nodes {
        by_base.entry(n.base_branch.clone()).or_default().push(n);
    }

    let mut out = String::new();
    let base_count = by_base.len();
    for (base_idx, (base, group)) in by_base.iter().enumerate() {
        // Phase 3: if the base branch is itself a worktree branch, mark it as
        // a stacked group rather than a plain base label.
        if let Some(parent_ticket) = branch_to_ticket.get(base.as_str()) {
            out.push_str(&format!("○ {} (stacked on {})\n", base, parent_ticket));
        } else {
            out.push_str(&format!("○ {} (base)\n", base));
        }
        let last_idx = group.len().saturating_sub(1);
        for (i, node) in group.iter().enumerate() {
            let is_last = i == last_idx;
            let branch_glyph = if is_last { "└" } else { "├" };
            let title = node.ticket_title.as_deref().unwrap_or("(no title)");
            out.push_str("│\n");
            out.push_str(&format!(
                "{}─● {} {} [{}]\n",
                branch_glyph, node.ticket, title, node.branch
            ));

            let prefix = if is_last { "   " } else { "│  " };
            // PR overlay (Phase 2): one line above commits when overlay set.
            // Phase 3: badge is now optionally colorized.
            if let Some(pr) = &node.pr {
                out.push_str(&format!("{}├─ {}\n", prefix, format_pr_badge(pr, color)));
            }
            if node.commits.is_empty() {
                out.push_str(&format!("{}└─ (no commits since {})\n", prefix, base));
            } else {
                let last_c = node.commits.len() - 1;
                for (ci, c) in node.commits.iter().enumerate() {
                    let glyph = if ci == last_c { "└" } else { "├" };
                    out.push_str(&format!(
                        "{}{}─ {} {}\n",
                        prefix, glyph, c.sha_short, c.subject
                    ));
                }
            }
        }
        if base_idx + 1 < base_count {
            out.push('\n');
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Phase 3 helpers: color support
// ---------------------------------------------------------------------------

/// Returns `true` when ANSI color output is appropriate.
///
/// Rules (first-match):
/// 1. `NO_COLOR` env var set (any value) → false (XDG spec).
/// 2. `PARSEC_COLOR=always` → true (force-on override).
/// 3. Stdout is not a TTY → false (piped / redirected output).
/// 4. Otherwise → true.
fn color_enabled() -> bool {
    if std::env::var_os("NO_COLOR").is_some() {
        return false;
    }
    if std::env::var("PARSEC_COLOR").as_deref() == Ok("always") {
        return true;
    }
    std::io::stdout().is_terminal()
}

/// Wrap `text` in the given ANSI SGR code when `color` is true.
///
/// Always appends SGR reset (0) after the text so colors don't bleed.
fn ansi_wrap(code: u8, text: &str, color: bool) -> String {
    if color {
        format!("\x1b[{}m{}\x1b[0m", code, text)
    } else {
        text.to_string()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn mk_commit(sha: &str, subject: &str) -> CommitSummary {
        CommitSummary {
            sha_short: sha.to_string(),
            subject: subject.to_string(),
            author: "Eric".to_string(),
            timestamp: Utc.with_ymd_and_hms(2026, 5, 13, 0, 0, 0).unwrap(),
        }
    }

    fn mk_node(
        ticket: &str,
        title: Option<&str>,
        branch: &str,
        commits: Vec<CommitSummary>,
    ) -> SmartlogNode {
        SmartlogNode {
            ticket: ticket.to_string(),
            ticket_title: title.map(|t| t.to_string()),
            branch: branch.to_string(),
            base_branch: "main".to_string(),
            worktree_path: PathBuf::from(format!("/tmp/{}", ticket)),
            commits,
            pr: None,
            ci: None,
        }
    }

    #[test]
    fn parse_commit_line_basic() {
        let c =
            parse_commit_line("a1b2c3d\tFix auth bug\tEric\t2026-05-13T09:30:00+09:00").unwrap();
        assert_eq!(c.sha_short, "a1b2c3d");
        assert_eq!(c.subject, "Fix auth bug");
        assert_eq!(c.author, "Eric");
    }

    #[test]
    fn parse_commit_line_subject_with_tabs() {
        // splitn(4) means tabs in the subject are preserved (first 3 tabs are separators)
        let c = parse_commit_line("aa\tsub\twith\ttab\tEric\t2026-05-13T09:30:00+09:00");
        // splitn(4, '\t') → ["aa", "sub", "with", "tab\tEric\t2026-05-13T09:30:00+09:00"]
        // Last segment isn't a valid timestamp → None.
        assert!(c.is_none(), "ambiguous line should be rejected");
    }

    #[test]
    fn parse_commit_line_rejects_garbage() {
        assert!(parse_commit_line("").is_none());
        assert!(parse_commit_line("only_one_field").is_none());
        assert!(parse_commit_line("a\tb\tc\tnot-a-date").is_none());
    }

    #[test]
    fn parse_commit_line_rejects_empty_sha() {
        assert!(parse_commit_line("\tsubject\tEric\t2026-05-13T09:30:00+09:00").is_none());
    }

    #[test]
    fn render_text_empty() {
        let s = render_text(&[], false);
        assert!(s.contains("No active worktrees"));
    }

    #[test]
    fn render_text_single_node() {
        let nodes = vec![mk_node(
            "CL-2283",
            Some("Add rate limiting"),
            "feature/CL-2283",
            vec![mk_commit("a1b2c3d", "Implement rate limiter")],
        )];
        let s = render_text(&nodes, false);
        assert!(s.contains("○ main (base)"));
        assert!(s.contains("CL-2283"));
        assert!(s.contains("Add rate limiting"));
        assert!(s.contains("[feature/CL-2283]"));
        assert!(s.contains("a1b2c3d Implement rate limiter"));
    }

    #[test]
    fn render_text_no_commits_shows_placeholder() {
        let nodes = vec![mk_node("CL-2291", None, "scratch/CL-2291", vec![])];
        let s = render_text(&nodes, false);
        assert!(s.contains("(no commits since main)"));
        assert!(s.contains("(no title)"));
    }

    #[test]
    fn render_text_multiple_nodes_groups_by_base() {
        let mut a = mk_node(
            "CL-1",
            Some("A"),
            "f/CL-1",
            vec![mk_commit("aaaaaaa", "first commit")],
        );
        a.base_branch = "main".to_string();
        let mut b = mk_node(
            "CL-2",
            Some("B"),
            "f/CL-2",
            vec![mk_commit("bbbbbbb", "second commit")],
        );
        b.base_branch = "develop".to_string();
        let s = render_text(&[a, b], false);
        assert!(s.contains("○ main (base)"));
        assert!(s.contains("○ develop (base)"));
        // Both nodes should render their commits.
        assert!(s.contains("first commit"));
        assert!(s.contains("second commit"));
    }

    #[test]
    fn smartlog_node_serializes_without_overlay_fields() {
        let node = mk_node("CL-1", Some("A"), "f/CL-1", vec![]);
        let json = serde_json::to_string(&node).unwrap();
        // PR/CI placeholder fields use skip_serializing_if so they don't pollute
        // the JSON output until a follow-up PR populates them.
        assert!(!json.contains("\"pr\""));
        assert!(!json.contains("\"ci\""));
        assert!(json.contains("\"ticket\":\"CL-1\""));
    }

    fn mk_overlay(state: &str, ci: &str, review: &str) -> SmartlogPrOverlay {
        SmartlogPrOverlay {
            number: 42,
            state: state.to_string(),
            ci_status: ci.to_string(),
            review_status: review.to_string(),
            url: "https://github.com/erishforG/git-parsec/pull/42".to_string(),
        }
    }

    #[test]
    fn format_pr_badge_open_passing_approved() {
        let badge = format_pr_badge(&mk_overlay("open", "success", "approved"), false);
        assert!(badge.starts_with("[PR #42 ● open ✓ CI"));
        assert!(badge.ends_with("✓ approved]"));
    }

    #[test]
    fn format_pr_badge_no_reviews_drops_review_segment() {
        let badge = format_pr_badge(&mk_overlay("open", "pending", "no reviews"), false);
        assert_eq!(badge, "[PR #42 ● open ● CI]");
    }

    #[test]
    fn format_pr_badge_merged_pr() {
        let badge = format_pr_badge(&mk_overlay("merged", "success", "approved"), false);
        // `merged` carries no special CI semantics — render the API-reported CI as-is.
        assert!(badge.contains("✓ merged"));
        assert!(badge.contains("✓ CI"));
        assert!(badge.contains("✓ approved"));
    }

    #[test]
    fn render_text_with_pr_overlay_attaches_badge_above_commits() {
        let mut node = mk_node(
            "CL-2283",
            Some("Add rate limiting"),
            "feature/CL-2283",
            vec![mk_commit("a1b2c3d", "Implement rate limiter")],
        );
        node.pr = Some(mk_overlay("open", "success", "approved"));
        let s = render_text(&[node], false);
        assert!(s.contains("CL-2283"), "ticket line still present");
        assert!(s.contains("[PR #42"), "PR badge rendered");
        assert!(s.contains("✓ approved"), "review badge rendered");
        // Badge must appear above the commit line (above as in earlier in the string).
        let badge_pos = s.find("[PR #42").unwrap();
        let commit_pos = s.find("a1b2c3d").unwrap();
        assert!(
            badge_pos < commit_pos,
            "PR badge should render above commits, got:\n{}",
            s
        );
    }

    #[test]
    fn smartlog_node_serializes_pr_overlay_when_set() {
        let mut node = mk_node("CL-1", Some("A"), "f/CL-1", vec![]);
        node.pr = Some(mk_overlay("open", "success", "approved"));
        let v: serde_json::Value = serde_json::to_value(&node).unwrap();
        let pr = v.get("pr").expect("pr field should serialize when set");
        assert_eq!(pr.get("number").and_then(|n| n.as_u64()), Some(42));
        assert_eq!(pr.get("state").and_then(|s| s.as_str()), Some("open"));
        assert_eq!(
            pr.get("ci_status").and_then(|s| s.as_str()),
            Some("success")
        );
        assert_eq!(
            pr.get("review_status").and_then(|s| s.as_str()),
            Some("approved")
        );
        // ci field still omitted — Phase 2 folds CI into the overlay.
        assert!(v.get("ci").is_none(), "ci field stays omitted in Phase 2");
    }

    // -----------------------------------------------------------------------
    // Phase 3 tests: filter, color, stack indicator
    // -----------------------------------------------------------------------

    #[test]
    fn worktree_filter_matches_ticket_substring() {
        // Only PROJ-1 should survive a "PROJ-1" filter applied before render.
        let nodes = vec![
            mk_node("PROJ-10", Some("Ten"), "feat/PROJ-10", vec![]),
            mk_node("PROJ-20", Some("Twenty"), "feat/PROJ-20", vec![]),
        ];
        let pat = "proj-1";
        let pat_lower = pat.to_lowercase();
        let filtered: Vec<_> = nodes
            .into_iter()
            .filter(|n| {
                n.ticket.to_lowercase().contains(&pat_lower)
                    || n.branch.to_lowercase().contains(&pat_lower)
            })
            .collect();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].ticket, "PROJ-10");
    }

    #[test]
    fn worktree_filter_branch_fallback() {
        // Pattern matches branch name even when ticket differs.
        let mut node = mk_node("CL-99", Some("T"), "feat/special-ui", vec![]);
        node.base_branch = "main".to_string();
        let nodes = vec![node];
        let pat_lower = "special".to_lowercase();
        let filtered: Vec<_> = nodes
            .into_iter()
            .filter(|n| {
                n.ticket.to_lowercase().contains(&pat_lower)
                    || n.branch.to_lowercase().contains(&pat_lower)
            })
            .collect();
        assert_eq!(filtered.len(), 1);
    }

    #[test]
    fn stack_indicator_appears_when_base_is_sibling_branch() {
        // PROJ-2 stacks on top of PROJ-1's branch.
        let parent = mk_node("PROJ-1", Some("Parent"), "feat/PROJ-1", vec![]);
        let mut child = mk_node("PROJ-2", Some("Child"), "feat/PROJ-2", vec![]);
        child.base_branch = "feat/PROJ-1".to_string(); // base = parent's branch

        let nodes = vec![parent, child];
        let s = render_text(&nodes, false);
        assert!(
            s.contains("stacked on PROJ-1"),
            "stack indicator missing in:\n{}",
            s
        );
    }

    #[test]
    fn color_badge_contains_ansi_codes_when_enabled() {
        let overlay = mk_overlay("open", "success", "approved");
        let badge_color = format_pr_badge(&overlay, true);
        let badge_plain = format_pr_badge(&overlay, false);
        // Color badge should contain ESC character; plain should not.
        assert!(
            badge_color.contains('\x1b'),
            "colored badge should contain ANSI escape"
        );
        assert!(
            !badge_plain.contains('\x1b'),
            "plain badge should not contain ANSI escape"
        );
    }

    #[test]
    fn color_badge_failure_ci_is_red() {
        let overlay = mk_overlay("open", "failure", "pending");
        let badge = format_pr_badge(&overlay, true);
        // Red = ESC[31m before "✗ CI"
        assert!(
            badge.contains("\x1b[31m"),
            "failure CI should use red (31) ANSI code, got: {:?}",
            badge
        );
    }
}
