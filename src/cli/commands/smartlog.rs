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
//! Phase 3 (PR #333 — filter · color · stack indicators):
//! - `--worktree <pattern>`: show only worktrees whose ticket or branch contains
//!   the pattern (case-insensitive substring match).
//! - ANSI color in the PR/CI badge: green=success, red=failure, yellow=pending,
//!   blue=open PR, dim=draft. Automatically disabled when `NO_COLOR` is set or
//!   stdout is not a TTY.
//! - Stack indicator: when a worktree's base branch is itself another active
//!   worktree's branch, annotate it with `⤷ stacked on <ticket>` so stacked-PR
//!   flows are immediately visible.
//!
//! Phase 4 (Issue #308 — topological DAG ordering + summary header):
//! - Groups are now rendered in topological order so stacked worktrees appear
//!   immediately below their parent in the output (no more alphabetical jumps).
//! - A one-line summary header: `smartlog  N worktrees · M stacked` gives a
//!   quick count before the tree.
//! - Multi-level stacks (depth > 1) are ordered correctly by the topo-sort so
//!   grandparent → parent → child ordering is preserved.
//! - Cycle-safe: a `placed` set ensures no group is emitted twice even if
//!   worktree branches form unusual reference loops.
//!
//! Phase 1 — #310 (CI overlay): `SmartlogCiOverlay` replaces the
//!   `serde_json::Value` placeholder in `SmartlogNode.ci`. After the PR
//!   overlay is attached, `attach_ci_overlay` calls `get_check_runs()` for
//!   each PR-linked node and populates the typed struct. Text renderer shows
//!   an inline `[CI: ✓ passed (N/N)]` / `[CI: ✗ failed (F/N)]` line.
//!
//! Phase 2 — #310 (CI overlay running count): `SmartlogCiOverlay` gains a
//!   `running: usize` field tracking how many check runs are still in-progress
//!   or queued. `format_ci_badge` now renders `● running (R running / N)`
//!   instead of the previously hardcoded `(0/N)`. The `running` field is
//!   `#[serde(default)]` so existing JSON snapshots remain valid on
//!   deserialization. `attach_ci_overlay` counts runs whose `status` is
//!   `"in_progress"` or `"queued"` (i.e., started but not yet concluded).
//!
//! Phase 3 — #310 (CI overlay for PR-less branches): `attach_ci_overlay` now
//!   also covers worktrees that have no open PR.  For each such node it
//!   resolves the branch-tip SHA with `git rev-parse <branch>` and calls
//!   `GitHubClient::get_check_runs_by_sha` to fetch check-run aggregate
//!   directly, without a PR round-trip.  The resulting `SmartlogCiOverlay` is
//!   attached exactly as for PR-linked nodes.  Network failures degrade
//!   gracefully (CI badge omitted) without failing the command.
//!
//! Phase 5 (Issue #309 — PR merge readiness overlay):
//! - `SmartlogPrOverlay::merge_ready: Option<bool>` surface the GitHub
//!   `mergeable` field that `get_pr_status()` already fetches but previously
//!   discarded before reaching the smartlog layer.
//! - `format_pr_badge()` appends `⬆ ready` (green) or `⚡ conflicts` (red)
//!   when the field is populated; open PRs with unknown mergeable state or
//!   non-open PRs omit the segment so output stays compact.
//! - Backward-compatible: `merge_ready` is `#[serde(skip_serializing_if =
//!   "Option::is_none")]` so existing JSON consumers and golden fixtures keep
//!   working.

use std::collections::{BTreeMap, HashMap, HashSet};
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
    /// CI overlay — populated by `attach_ci_overlay` (Phase 1 of #310).
    /// Holds aggregated check-run counts for the PR linked to this worktree.
    /// Omitted from JSON when no CI data was fetched (backward-compatible).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ci: Option<SmartlogCiOverlay>,
}

/// Aggregated CI check-run overlay for a smartlog node (Phase 1+2 of #310).
///
/// Populated by [`attach_ci_overlay`] when a PR is linked.  Four states
/// map the raw `CiStatus.overall` from [`GitHubClient::get_check_runs`]:
///
/// | `overall` field | meaning |
/// |---|---|
/// | `"passed"` | all checks succeeded (or were skipped) |
/// | `"running"` | at least one check is still in-progress or queued |
/// | `"failed"` | at least one check failed or timed out |
/// | `"none"` | no check-runs found for the PR |
///
/// `total`, `failed`, and `running` expose counts so the text renderer can
/// display `✗ failed (2/7)` or `● running (3 running / 7)` without an
/// extra API call.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SmartlogCiOverlay {
    /// `"passed"` / `"running"` / `"failed"` / `"none"`.
    pub overall: String,
    /// Total number of check runs returned by the GitHub Checks API.
    pub total: usize,
    /// Number of failed or timed-out check runs.
    pub failed: usize,
    /// Number of check runs still in-progress or queued (Phase 2 of #310).
    /// Defaults to 0 for backward-compatible JSON deserialization.
    #[serde(default)]
    pub running: usize,
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
    /// GitHub merge readiness — `Some(true)` when the PR can be merged
    /// (no conflicts, all checks green per GitHub's internal verdict),
    /// `Some(false)` when there are conflicts or blocking checks.
    /// `None` when GitHub has not yet computed the state (e.g., immediately
    /// after a push) or the PR is already merged/closed.
    /// Skip-serialised when absent so existing JSON consumers see no change.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merge_ready: Option<bool>,
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
        attach_ci_overlay(repo, &config, &mut nodes).await;
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

/// Fetch GitHub check-run aggregates for each PR-linked node and populate
/// `node.ci` with a typed [`SmartlogCiOverlay`].
///
/// Mirrors the soft-fail pattern of [`attach_pr_overlay`]: network errors are
/// logged to stderr but never fail the parent command.  Nodes without a PR
/// overlay are skipped silently.
async fn attach_ci_overlay(repo: &Path, config: &ParsecConfig, nodes: &mut [SmartlogNode]) {
    // Always init the client: Phase 3 (#310) needs it for PR-less nodes too.
    let remote_url = match git::run_output(repo, &["remote", "get-url", "origin"]) {
        Ok(url) => url.trim().to_string(),
        Err(_) => return,
    };
    let client = match GitHubClient::new(&remote_url, config) {
        Ok(Some(c)) => c,
        _ => return,
    };

    for node in nodes.iter_mut() {
        let ci_status_result = if let Some(pr) = &node.pr {
            // PR-linked node: fetch check-runs via the existing PR-based lookup
            // (which resolves the PR head SHA internally).
            client.get_check_runs(pr.number).await
        } else {
            // Phase 3 (#310): PR-less branch — resolve branch-tip SHA with git
            // and fetch check-runs directly.  Skip if git lookup fails (e.g.
            // orphan branch or remote not synced).
            match resolve_branch_tip_sha(repo, &node.branch) {
                Some(sha) => client.get_check_runs_by_sha(&sha).await,
                None => continue,
            }
        };

        match ci_status_result {
            Ok(ci_status) => {
                let overall = map_ci_overall(&ci_status.overall);
                let total = ci_status.checks.len();
                let failed = ci_status
                    .checks
                    .iter()
                    .filter(|c| {
                        matches!(c.conclusion.as_deref(), Some("failure") | Some("timed_out"))
                    })
                    .count();
                // Phase 2 (#310): count checks still actively running or
                // waiting in the queue so the badge can display an accurate
                // "N running / M" count instead of the previous "(0/N)".
                let running = ci_status
                    .checks
                    .iter()
                    .filter(|c| matches!(c.status.as_str(), "in_progress" | "queued"))
                    .count();
                node.ci = Some(SmartlogCiOverlay {
                    overall,
                    total,
                    failed,
                    running,
                });
            }
            Err(e) => {
                eprintln!(
                    "smartlog: CI overlay failed for {} ({}): {}",
                    node.ticket, node.branch, e
                );
            }
        }
    }
}

/// Resolve a branch to its tip commit SHA using `git rev-parse`.
///
/// Returns the full 40-character SHA, or `None` when the branch cannot be
/// resolved (e.g., unborn, orphan, or not yet pushed to remote).
fn resolve_branch_tip_sha(repo: &Path, branch: &str) -> Option<String> {
    let sha = git::run_output(repo, &["rev-parse", branch]).ok()?;
    let sha = sha.trim().to_string();
    if sha.is_empty() || sha.len() < 7 {
        return None;
    }
    Some(sha)
}

/// Map the raw `CiStatus.overall` string from [`GitHubClient::get_check_runs`]
/// to the four canonical `SmartlogCiOverlay.overall` values.
fn map_ci_overall(raw: &str) -> String {
    match raw {
        "passing" => "passed".to_string(),
        "failing" => "failed".to_string(),
        "pending" => "running".to_string(),
        _ => "none".to_string(), // "no checks" or anything unexpected
    }
}

/// Resolve a single branch to a [`SmartlogPrOverlay`], or `None` if no open PR.
async fn fetch_overlay(client: &GitHubClient, branch: &str) -> Result<Option<SmartlogPrOverlay>> {
    let pr_num = match client.find_pr_by_branch(branch).await? {
        Some(n) => n,
        None => return Ok(None),
    };
    let status = client.get_pr_status(pr_num).await?;
    // Populate merge_ready only for open PRs; merged/closed PRs don't have a
    // meaningful "can be merged" state from GitHub's perspective.
    let merge_ready = if status.state == "open" {
        status.mergeable
    } else {
        None
    };
    Ok(Some(SmartlogPrOverlay {
        number: status.number,
        state: status.state,
        ci_status: status.ci_status,
        review_status: status.review_status,
        url: status.url,
        merge_ready,
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
    // Phase 5: merge readiness segment — only for open PRs.
    if pr.state == "open" {
        if let Some(ready) = pr.merge_ready {
            let segment = if ready {
                ansi_wrap(32, "⬆ ready", color) // green
            } else {
                ansi_wrap(31, "⚡ conflicts", color) // red
            };
            out.pop(); // remove ']'
            out.push_str(&format!(" {}]", segment));
        }
    }
    out
}

/// Format a one-line CI check-run summary badge for the smartlog ASCII tree.
///
/// Examples (no color):
/// - `[CI: ✓ passed (7/7)]`
/// - `[CI: ✗ failed (2/7)]`
/// - `[CI: ● running (0/5)]`
/// - `[CI: none]`
fn format_ci_badge(ci: &SmartlogCiOverlay, color: bool) -> String {
    let label = match ci.overall.as_str() {
        "passed" => ansi_wrap(
            32,
            &format!("✓ passed ({}/{})", ci.total - ci.failed, ci.total),
            color,
        ),
        "failed" => ansi_wrap(31, &format!("✗ failed ({}/{})", ci.failed, ci.total), color),
        // Phase 2 (#310): show accurate running count instead of hardcoded 0.
        "running" => ansi_wrap(
            33,
            &format!("● running ({} running/{})", ci.running, ci.total),
            color,
        ),
        _ => "none".to_string(),
    };
    format!("[CI: {}]", label)
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

    // Phase 4: summary header.
    let stacked_count = nodes
        .iter()
        .filter(|n| branch_to_ticket.contains_key(n.base_branch.as_str()))
        .count();
    let mut out = format!(
        "smartlog  {} worktree{} · {} stacked\n",
        nodes.len(),
        if nodes.len() == 1 { "" } else { "s" },
        stacked_count,
    );

    // Phase 4: topological ordering — stacked groups follow their parent.
    let ordered = topo_sort_groups(&by_base, &branch_to_ticket);
    for (base, group) in &ordered {
        // Phase 3/4: if the base branch is itself a worktree branch, mark it as
        // a stacked group and show its parent ticket with ⤷ arrow.
        if let Some(parent_ticket) = branch_to_ticket.get(base.as_str()) {
            out.push_str(&format!("\n○ {} ⤷ stacked on {}\n", base, parent_ticket));
        } else {
            out.push_str(&format!("\n○ {} (base)\n", base));
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
            // CI overlay (Phase 1 of #310): check-run counts line.
            if let Some(ci) = &node.ci {
                out.push_str(&format!("{}├─ {}\n", prefix, format_ci_badge(ci, color)));
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
    }
    out
}

// ---------------------------------------------------------------------------
// Phase 4: topological DAG group ordering
// ---------------------------------------------------------------------------

/// Sort `by_base` groups so that stacked groups appear immediately after the
/// group containing their parent worktree.
///
/// Algorithm:
/// 1. Emit root bases (bases not matching any worktree branch) first, in
///    alphabetical order.
/// 2. After each group, immediately emit any stacked group whose base equals
///    one of the branches in the just-emitted group.
/// 3. Any remaining groups (e.g., whose parent was filtered out) are appended
///    at the end, also in alphabetical order.
///
/// A `placed` set prevents infinite loops when an unusual worktree graph has
/// a cycle in its stacking relationships.
fn topo_sort_groups<'a>(
    by_base: &'a BTreeMap<String, Vec<&'a SmartlogNode>>,
    branch_to_ticket: &HashMap<&str, &str>,
) -> Vec<(&'a String, &'a Vec<&'a SmartlogNode>)> {
    let mut ordered: Vec<(&'a String, &'a Vec<&'a SmartlogNode>)> = Vec::new();
    let mut placed: HashSet<&str> = HashSet::new();

    fn visit<'a>(
        base: &str,
        by_base: &'a BTreeMap<String, Vec<&'a SmartlogNode>>,
        placed: &mut HashSet<&'a str>,
        ordered: &mut Vec<(&'a String, &'a Vec<&'a SmartlogNode>)>,
    ) {
        if placed.contains(base) {
            return;
        }
        if let Some((key, group)) = by_base.get_key_value(base) {
            placed.insert(key.as_str());
            ordered.push((key, group));
            // Recurse: for each node in this group, check if any by_base entry
            // has that node's branch as its base.
            for node in group {
                visit(node.branch.as_str(), by_base, placed, ordered);
            }
        }
    }

    // First pass: process root bases (those whose base key is NOT a worktree
    // branch).  BTreeMap iteration is alphabetical, giving a stable order.
    for base in by_base.keys() {
        if !branch_to_ticket.contains_key(base.as_str()) {
            visit(base, by_base, &mut placed, &mut ordered);
        }
    }
    // Second pass: any groups not yet placed (parents filtered out, orphans).
    for base in by_base.keys() {
        visit(base, by_base, &mut placed, &mut ordered);
    }
    ordered
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
            merge_ready: None,
        }
    }

    fn mk_overlay_with_readiness(
        state: &str,
        ci: &str,
        review: &str,
        merge_ready: Option<bool>,
    ) -> SmartlogPrOverlay {
        SmartlogPrOverlay {
            number: 99,
            state: state.to_string(),
            ci_status: ci.to_string(),
            review_status: review.to_string(),
            url: "https://github.com/erishforG/git-parsec/pull/99".to_string(),
            merge_ready,
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
        // ci field omitted — attach_ci_overlay was not called on this manually
        // constructed node (ci is only populated by the async overlay function).
        assert!(
            v.get("ci").is_none(),
            "ci field stays None for manually-built node"
        );
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

    // -----------------------------------------------------------------------
    // Phase 4 tests: topological ordering + summary header
    // -----------------------------------------------------------------------

    #[test]
    fn summary_header_appears_with_counts() {
        let nodes = vec![
            mk_node("PROJ-1", Some("A"), "feat/PROJ-1", vec![]),
            mk_node("PROJ-2", Some("B"), "feat/PROJ-2", vec![]),
        ];
        let s = render_text(&nodes, false);
        // Summary header must be present and contain worktree count.
        assert!(s.starts_with("smartlog "), "summary header missing");
        assert!(s.contains("2 worktrees"), "worktree count wrong: {}", s);
        assert!(s.contains("0 stacked"), "stacked count wrong: {}", s);
    }

    #[test]
    fn summary_header_singular_worktree() {
        let nodes = vec![mk_node("CL-1", Some("A"), "feat/CL-1", vec![])];
        let s = render_text(&nodes, false);
        assert!(s.contains("1 worktree ·"), "singular form wrong: {}", s);
    }

    #[test]
    fn topo_sort_stacked_group_follows_parent() {
        // PROJ-2 stacks on PROJ-1.  In alphabetical order PROJ-1 < PROJ-2
        // so both orderings happen to agree; use a lexicographically-later
        // parent name to exercise the non-trivial case.
        let parent = mk_node("PROJ-Z", Some("Parent"), "feat/PROJ-Z", vec![]);
        let mut child = mk_node("PROJ-A", Some("Child"), "feat/PROJ-A", vec![]);
        child.base_branch = "feat/PROJ-Z".to_string();

        // Alphabetically PROJ-A's base (feat/PROJ-A) would come before
        // feat/PROJ-Z, but with topo sort the child group should appear
        // directly after the parent group in the rendered output.
        let nodes = vec![parent, child];
        let s = render_text(&nodes, false);

        // Both groups must appear.
        assert!(s.contains("PROJ-Z"), "parent not rendered");
        assert!(s.contains("PROJ-A"), "child not rendered");

        // The child's stacked header must appear AFTER the parent section.
        let parent_pos = s.find("PROJ-Z").unwrap();
        let child_stack_pos = s.find("⤷ stacked on PROJ-Z").unwrap();
        assert!(
            parent_pos < child_stack_pos,
            "child group should follow parent group; got:\n{}",
            s
        );

        // Stacked count should be 1.
        assert!(s.contains("1 stacked"), "stacked count wrong: {}", s);
    }

    #[test]
    fn topo_sort_multi_level_stack_order() {
        // grandparent → parent → child (three levels)
        let gp = mk_node("GP", Some("Grandparent"), "feat/GP", vec![]);
        let mut parent = mk_node("PA", Some("Parent"), "feat/PA", vec![]);
        parent.base_branch = "feat/GP".to_string();
        let mut child = mk_node("CH", Some("Child"), "feat/CH", vec![]);
        child.base_branch = "feat/PA".to_string();

        let nodes = vec![gp, parent, child];
        let s = render_text(&nodes, false);

        // GP sits on "main" (the mk_node default base), so the root label is
        // "main (base)", not "GP (base)".  PA and CH are stacked and use the
        // ⤷ arrow with their parent's ticket name.
        let gp_pos = s.find("main (base)").unwrap();
        let pa_pos = s.find("⤷ stacked on GP").unwrap();
        let ch_pos = s.find("⤷ stacked on PA").unwrap();
        assert!(gp_pos < pa_pos, "parent should follow grandparent");
        assert!(pa_pos < ch_pos, "child should follow parent");
        assert!(s.contains("2 stacked"), "stacked count wrong: {}", s);
    }

    #[test]
    fn topo_sort_stable_for_independent_roots() {
        // Two independent root bases: `develop` and `main`.
        // BTreeMap alphabetical order: develop < main.
        let a = mk_node("CL-A", Some("A"), "feat/CL-A", vec![]);
        let mut b = mk_node("CL-B", Some("B"), "feat/CL-B", vec![]);
        b.base_branch = "develop".to_string();

        let nodes = vec![a, b];
        let s = render_text(&nodes, false);
        // Both roots (main, develop) should appear in the output.
        assert!(s.contains("main (base)") || s.contains("develop (base)"));
        assert!(s.contains("CL-A") && s.contains("CL-B"));
        assert!(s.contains("0 stacked"), "no stacks expected");
    }

    // -----------------------------------------------------------------------
    // Phase 5 tests: merge readiness overlay (#309)
    // -----------------------------------------------------------------------

    #[test]
    fn merge_ready_true_appends_ready_segment() {
        let overlay = mk_overlay_with_readiness("open", "success", "approved", Some(true));
        let badge = format_pr_badge(&overlay, false);
        assert!(
            badge.contains("⬆ ready"),
            "expected '⬆ ready' in badge but got: {}",
            badge
        );
        assert!(
            badge.ends_with("⬆ ready]"),
            "ready segment should be last: {}",
            badge
        );
    }

    #[test]
    fn merge_ready_false_appends_conflicts_segment() {
        let overlay = mk_overlay_with_readiness("open", "success", "approved", Some(false));
        let badge = format_pr_badge(&overlay, false);
        assert!(
            badge.contains("⚡ conflicts"),
            "expected '⚡ conflicts' in badge but got: {}",
            badge
        );
    }

    #[test]
    fn merge_ready_none_omits_readiness_segment() {
        let overlay = mk_overlay_with_readiness("open", "success", "no reviews", None);
        let badge = format_pr_badge(&overlay, false);
        assert!(
            !badge.contains("ready") && !badge.contains("conflicts"),
            "unknown readiness should omit segment: {}",
            badge
        );
    }

    #[test]
    fn merge_ready_skipped_for_merged_pr() {
        // A merged PR with merge_ready=Some(true) should NOT show the segment
        // because the PR is no longer open.
        let overlay = mk_overlay_with_readiness("merged", "success", "approved", Some(true));
        let badge = format_pr_badge(&overlay, false);
        assert!(
            !badge.contains("⬆ ready"),
            "merged PR should not show ready segment: {}",
            badge
        );
    }

    #[test]
    fn merge_ready_skipped_for_draft_pr() {
        // Draft PRs are open but merge_ready is typically None from GitHub;
        // even if Some(true) somehow arrives, verify the format is correct.
        let overlay = mk_overlay_with_readiness("draft", "pending", "no reviews", Some(false));
        let badge = format_pr_badge(&overlay, false);
        // draft is rendered as state="draft"; open check uses pr.state == "open"
        // so draft should NOT emit the readiness segment.
        assert!(
            !badge.contains("⚡ conflicts"),
            "draft PR should not show conflicts segment: {}",
            badge
        );
    }

    #[test]
    fn merge_ready_serde_roundtrip_omits_none() {
        // When merge_ready is None, the serialised JSON must not contain
        // the field (backward-compat for existing JSON consumers).
        let overlay = mk_overlay("open", "pending", "no reviews");
        assert!(overlay.merge_ready.is_none());
        let json = serde_json::to_string(&overlay).expect("serialize");
        assert!(
            !json.contains("merge_ready"),
            "merge_ready=None should be omitted from JSON, got: {}",
            json
        );
    }

    #[test]
    fn merge_ready_serde_roundtrip_present() {
        // When merge_ready is Some, it must survive a JSON round-trip.
        let overlay = mk_overlay_with_readiness("open", "success", "approved", Some(true));
        let json = serde_json::to_string(&overlay).expect("serialize");
        assert!(
            json.contains("\"merge_ready\":true"),
            "missing field: {}",
            json
        );
        let back: SmartlogPrOverlay = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.merge_ready, Some(true));
    }

    // -----------------------------------------------------------------------
    // Phase 1 + Phase 2 tests: SmartlogCiOverlay (#310)
    // -----------------------------------------------------------------------

    /// Construct a `SmartlogCiOverlay` with `running` defaulting to 0
    /// (mirrors Phase-1 era callers; Phase-2 tests use `mk_ci_running`).
    fn mk_ci(overall: &str, total: usize, failed: usize) -> SmartlogCiOverlay {
        SmartlogCiOverlay {
            overall: overall.to_string(),
            total,
            failed,
            running: 0,
        }
    }

    /// Construct a `SmartlogCiOverlay` with an explicit `running` count
    /// (Phase 2 of #310).
    fn mk_ci_running(
        overall: &str,
        total: usize,
        failed: usize,
        running: usize,
    ) -> SmartlogCiOverlay {
        SmartlogCiOverlay {
            overall: overall.to_string(),
            total,
            failed,
            running,
        }
    }

    #[test]
    fn ci_badge_passed_shows_count() {
        let ci = mk_ci("passed", 7, 0);
        let badge = format_ci_badge(&ci, false);
        assert_eq!(badge, "[CI: ✓ passed (7/7)]");
    }

    #[test]
    fn ci_badge_failed_shows_failure_count() {
        let ci = mk_ci("failed", 7, 2);
        let badge = format_ci_badge(&ci, false);
        assert_eq!(badge, "[CI: ✗ failed (2/7)]");
    }

    // Phase 2: running badge now shows accurate running count, not hardcoded 0.
    #[test]
    fn ci_badge_running_shows_running_count() {
        let ci = mk_ci_running("running", 5, 0, 3);
        let badge = format_ci_badge(&ci, false);
        assert_eq!(badge, "[CI: ● running (3 running/5)]");
    }

    #[test]
    fn ci_badge_running_zero_running_shows_zero() {
        // Edge case: overall=running but running count is 0 (queued or unusual state).
        let ci = mk_ci_running("running", 4, 0, 0);
        let badge = format_ci_badge(&ci, false);
        assert_eq!(badge, "[CI: ● running (0 running/4)]");
    }

    #[test]
    fn ci_badge_none_no_counts() {
        let ci = mk_ci("none", 0, 0);
        let badge = format_ci_badge(&ci, false);
        assert_eq!(badge, "[CI: none]");
    }

    #[test]
    fn ci_overlay_serde_roundtrip() {
        let ci = mk_ci_running("failed", 7, 2, 1);
        let json = serde_json::to_string(&ci).expect("serialize");
        let back: SmartlogCiOverlay = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, ci);
    }

    // Phase 2: JSON produced before Phase 2 (no "running" key) should
    // deserialize with running=0 thanks to #[serde(default)].
    #[test]
    fn ci_overlay_phase1_json_backward_compat() {
        let phase1_json = r#"{"overall":"running","total":5,"failed":0}"#;
        let back: SmartlogCiOverlay =
            serde_json::from_str(phase1_json).expect("should deserialize without running field");
        assert_eq!(back.overall, "running");
        assert_eq!(back.total, 5);
        assert_eq!(back.failed, 0);
        assert_eq!(back.running, 0, "missing running key must default to 0");
    }

    #[test]
    fn ci_overlay_omitted_from_node_json_when_none() {
        let node = mk_node("CL-5", Some("E"), "feat/CL-5", vec![]);
        assert!(node.ci.is_none());
        let v: serde_json::Value = serde_json::to_value(&node).unwrap();
        assert!(v.get("ci").is_none(), "ci=None must be skip-serialized");
    }

    #[test]
    fn ci_overlay_present_in_node_json_when_set() {
        let mut node = mk_node("CL-6", Some("F"), "feat/CL-6", vec![]);
        node.ci = Some(mk_ci_running("passed", 7, 0, 0));
        let v: serde_json::Value = serde_json::to_value(&node).unwrap();
        let ci = v.get("ci").expect("ci must serialize when Some");
        assert_eq!(ci.get("overall").and_then(|s| s.as_str()), Some("passed"));
        assert_eq!(ci.get("total").and_then(|n| n.as_u64()), Some(7));
        assert_eq!(ci.get("failed").and_then(|n| n.as_u64()), Some(0));
        assert_eq!(ci.get("running").and_then(|n| n.as_u64()), Some(0));
    }

    #[test]
    fn render_text_shows_ci_badge_line_when_ci_set() {
        let mut node = mk_node("CL-7", Some("G"), "feat/CL-7", vec![]);
        node.ci = Some(mk_ci("failed", 7, 2));
        let out = render_text(&[node], false);
        assert!(
            out.contains("[CI: ✗ failed (2/7)]"),
            "expected CI badge in render_text output: {}",
            out
        );
    }

    // Phase 2: running badge renders correctly in the full text output.
    #[test]
    fn render_text_shows_running_ci_badge_with_accurate_count() {
        let mut node = mk_node("CL-8", Some("H"), "feat/CL-8", vec![]);
        node.ci = Some(mk_ci_running("running", 6, 0, 2));
        let out = render_text(&[node], false);
        assert!(
            out.contains("[CI: ● running (2 running/6)]"),
            "expected running CI badge with accurate count in: {}",
            out
        );
    }

    #[test]
    fn map_ci_overall_maps_all_variants() {
        assert_eq!(map_ci_overall("passing"), "passed");
        assert_eq!(map_ci_overall("failing"), "failed");
        assert_eq!(map_ci_overall("pending"), "running");
        assert_eq!(map_ci_overall("no checks"), "none");
        assert_eq!(map_ci_overall("unexpected"), "none");
    }

    // -----------------------------------------------------------------------
    // Phase 3 tests: branch-tip CI overlay for PR-less nodes (#310)
    // -----------------------------------------------------------------------

    /// A node whose CI field is set from a branch-tip lookup (no PR) should
    /// render the CI badge identically to a PR-linked node.  This validates
    /// that the `SmartlogCiOverlay` path is shared between both code paths.
    #[test]
    fn ci_badge_renders_same_for_pr_less_node() {
        // Simulate a node that has no PR but got its CI overlay via SHA lookup.
        let mut node = mk_node("CL-9", Some("I"), "feat/CL-9", vec![]);
        assert!(node.pr.is_none(), "node must have no PR for this test");
        node.ci = Some(mk_ci("passed", 4, 0));
        let out = render_text(&[node], false);
        assert!(
            out.contains("[CI: ✓ passed (4/4)]"),
            "PR-less node should render CI badge: {}",
            out
        );
    }

    /// A node with no PR and CI=none renders the `[CI: none]` badge so the
    /// user can see that CI is present but no checks were found (e.g., a fresh
    /// branch not yet linked to a CI pipeline).
    #[test]
    fn ci_badge_none_renders_for_pr_less_node() {
        let mut node = mk_node("CL-10", Some("J"), "feat/CL-10", vec![]);
        node.ci = Some(mk_ci("none", 0, 0));
        let out = render_text(&[node], false);
        assert!(
            out.contains("[CI: none]"),
            "PR-less node with no checks should render [CI: none]: {}",
            out
        );
    }

    /// `resolve_branch_tip_sha` must return `None` for obviously invalid inputs
    /// without panicking (e.g., empty string from a malformed git command).
    #[test]
    fn resolve_branch_tip_sha_rejects_empty() {
        use std::path::Path;
        // A path that is definitely not a git repo will cause `git rev-parse`
        // to fail, which should return None rather than panic.
        let result = resolve_branch_tip_sha(Path::new("/tmp"), "non-existent-branch");
        assert!(
            result.is_none(),
            "non-git-repo path should return None, got: {:?}",
            result
        );
    }
}
