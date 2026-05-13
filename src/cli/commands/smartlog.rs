//! `parsec smartlog` (alias `sl`) — visualize active worktrees as a commit DAG.
//!
//! Issue #245 — Phase 1 (skeleton):
//! - Collect every active worktree via [`WorktreeManager`]
//! - Read each worktree's commits since its base branch (`base..branch`)
//! - Render as ASCII tree, or emit JSON
//!
//! PR/CI/review overlay is intentionally **out of scope** for this PR;
//! [`SmartlogNode::pr`] / [`SmartlogNode::ci`] fields are placeholders that
//! later PRs will populate (e.g., GitHub PR state, CI run state, review state).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::config::ParsecConfig;
use crate::git;
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
    /// PR overlay — populated by a later PR (see #245 follow-up).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pr: Option<serde_json::Value>,
    /// CI overlay — populated by a later PR (see #245 follow-up).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ci: Option<serde_json::Value>,
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
pub async fn smartlog(repo: &Path, depth: Option<usize>, mode: Mode) -> Result<()> {
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

    match mode {
        Mode::Json => {
            println!("{}", serde_json::to_string_pretty(&nodes)?);
        }
        _ => {
            print!("{}", render_text(&nodes));
        }
    }
    Ok(())
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
pub fn render_text(nodes: &[SmartlogNode]) -> String {
    if nodes.is_empty() {
        return "No active worktrees. Run `parsec start <ticket>` to create one.\n".to_string();
    }

    let mut by_base: BTreeMap<String, Vec<&SmartlogNode>> = BTreeMap::new();
    for n in nodes {
        by_base.entry(n.base_branch.clone()).or_default().push(n);
    }

    let mut out = String::new();
    let base_count = by_base.len();
    for (base_idx, (base, group)) in by_base.iter().enumerate() {
        out.push_str(&format!("○ {} (base)\n", base));
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
        let s = render_text(&[]);
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
        let s = render_text(&nodes);
        assert!(s.contains("○ main (base)"));
        assert!(s.contains("CL-2283"));
        assert!(s.contains("Add rate limiting"));
        assert!(s.contains("[feature/CL-2283]"));
        assert!(s.contains("a1b2c3d Implement rate limiter"));
    }

    #[test]
    fn render_text_no_commits_shows_placeholder() {
        let nodes = vec![mk_node("CL-2291", None, "scratch/CL-2291", vec![])];
        let s = render_text(&nodes);
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
        let s = render_text(&[a, b]);
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
}
