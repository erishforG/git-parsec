//! `parsec triage` — rule-based issue auto-labelling (#302).
//!
//! - Load `[[triage.rules]]` from parsec config.
//! - Fetch open GitHub issues for the repo.
//! - Apply each rule (case-insensitive substring match on title).
//! - Print a table: issue #, title, matched rule, proposed labels, trust score.
//! - Write proposed labels only when `--apply` is explicitly passed.
//!
//! Trust-score logic:
//! - First (highest-priority) rule to match → **1.0**
//! - A second rule also matches the same issue  → **0.7**
//! - Three or more rules match                  → **0.5**

use std::path::Path;

use anyhow::{Context, Result};
use colored::Colorize;
use tabled::{settings::Style, Table, Tabled};

use crate::config::{ParsecConfig, TriageRule};
use crate::git;
use crate::github::GitHubClient;
use crate::output::Mode;

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Entry point for `parsec triage`.
///
/// Fetches open issues, applies the configured rules, and prints the proposed
/// labels. Labels are written only when `apply` is true.
pub async fn triage(repo: &Path, limit: u8, apply: bool, mode: Mode) -> Result<()> {
    let config = ParsecConfig::load()?;

    if config.triage.rules.is_empty() {
        match mode {
            Mode::Json => println!("[]"),
            Mode::Quiet => {}
            Mode::Human => {
                println!("{}", "No triage rules configured.".yellow());
                println!();
                println!("Add rules to your parsec config:");
                println!();
                println!("  [[triage.rules]]");
                println!("  pattern  = \"feat\"");
                println!("  label    = \"type/feature\"");
                println!();
                println!("  [[triage.rules]]");
                println!("  pattern  = \"fix\"");
                println!("  label    = \"type/bug\"");
                println!("  priority = \"priority/high\"");
            }
        }
        return Ok(());
    }

    let remote_url = git::get_remote_url(repo).unwrap_or_default();
    let gh = match GitHubClient::new(&remote_url, &config)? {
        Some(c) => c,
        None => {
            anyhow::bail!(
                "no GitHub token found\n\
                 caused by: GITHUB_TOKEN not set and no token in parsec config\n\
                 help: run `gh auth login` or set GITHUB_TOKEN=<pat> in your environment"
            );
        }
    };

    let issues = gh.list_open_issues(limit).await?;

    if issues.is_empty() {
        match mode {
            Mode::Json => println!("[]"),
            Mode::Quiet => {}
            Mode::Human => println!("No open issues found."),
        }
        return Ok(());
    }

    let entries: Vec<TriageEntry> = issues
        .iter()
        .filter_map(|(number, title, existing_labels)| {
            build_entry(*number, title, existing_labels, &config.triage.rules)
        })
        .collect();

    if entries.is_empty() {
        match mode {
            Mode::Json => println!("[]"),
            Mode::Quiet => {}
            Mode::Human => println!(
                "No issues matched any triage rule (checked {}).",
                issues.len()
            ),
        }
        return Ok(());
    }

    if apply {
        for entry in &entries {
            gh.add_labels(entry.number, &entry.labels)
                .await
                .with_context(|| format!("failed to apply labels to issue #{}", entry.number))?;
        }
    }

    match mode {
        Mode::Json => {
            println!("{}", serde_json::to_string_pretty(&entries)?);
        }
        Mode::Quiet => {
            for e in &entries {
                println!("#{} → {}", e.number, e.proposed_labels);
            }
        }
        Mode::Human => {
            println!();
            println!(
                "{}",
                format!(
                    " parsec triage — {} ({} rule(s), {} issue(s) matched of {})",
                    if apply { "applied" } else { "dry-run" },
                    config.triage.rules.len(),
                    entries.len(),
                    issues.len()
                )
                .bold()
            );
            println!();

            // Build display rows
            let rows: Vec<DisplayRow> = entries
                .iter()
                .map(|e| DisplayRow {
                    number: format!("#{}", e.number),
                    title: truncate(&e.title, 42),
                    matched_rule: e.matched_rule.clone(),
                    proposed_labels: e.proposed_labels.clone(),
                    trust: format!("{:.1}", e.trust_score),
                })
                .collect();

            let table = Table::new(rows).with(Style::rounded()).to_string();
            println!("{table}");
            println!();
            if !apply {
                println!(
                    "{}",
                    "Tip: No labels were written. Pass --apply to apply.".dimmed()
                );
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Compute a triage entry for one issue.
///
/// Returns `None` when no rule matches.
fn build_entry(
    number: u64,
    title: &str,
    existing_labels: &[String],
    rules: &[TriageRule],
) -> Option<TriageEntry> {
    let lower = title.to_lowercase();
    let matched: Vec<&TriageRule> = rules
        .iter()
        .filter(|r| lower.contains(&r.pattern.to_lowercase()))
        .collect();

    if matched.is_empty() {
        return None;
    }

    // Trust score degrades with additional matches (first rule wins).
    let trust_score: f32 = match matched.len() {
        1 => 1.0,
        2 => 0.7,
        _ => 0.5,
    };

    // Collect all proposed labels (first match has highest priority).
    let first = matched[0];
    let mut labels: Vec<String> = vec![first.label.clone()];
    if let Some(ref p) = first.priority {
        labels.push(p.clone());
    }
    let proposed_labels = labels.join(", ");

    Some(TriageEntry {
        number,
        title: title.to_string(),
        matched_rule: first.pattern.clone(),
        proposed_labels,
        labels,
        trust_score,
        already_labelled: !existing_labels.is_empty(),
    })
}

fn truncate(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        s.to_string()
    } else {
        format!("{}…", chars[..max - 1].iter().collect::<String>())
    }
}

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// One triage proposal — one issue matched one or more rules.
#[derive(Debug, serde::Serialize)]
pub struct TriageEntry {
    pub number: u64,
    pub title: String,
    pub matched_rule: String,
    pub proposed_labels: String,
    #[serde(skip)]
    labels: Vec<String>,
    pub trust_score: f32,
    pub already_labelled: bool,
}

/// Table row for `tabled` human output.
#[derive(Tabled)]
struct DisplayRow {
    #[tabled(rename = "#")]
    number: String,
    #[tabled(rename = "Title")]
    title: String,
    #[tabled(rename = "Matched rule")]
    matched_rule: String,
    #[tabled(rename = "Proposed labels")]
    proposed_labels: String,
    #[tabled(rename = "Trust")]
    trust: String,
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TriageRule;

    fn rule(pattern: &str, label: &str) -> TriageRule {
        TriageRule {
            pattern: pattern.to_string(),
            label: label.to_string(),
            priority: None,
        }
    }

    fn rule_with_priority(pattern: &str, label: &str, priority: &str) -> TriageRule {
        TriageRule {
            pattern: pattern.to_string(),
            label: label.to_string(),
            priority: Some(priority.to_string()),
        }
    }

    #[test]
    fn no_match_returns_none() {
        let rules = vec![rule("feat", "type/feature")];
        assert!(build_entry(1, "chore: update deps", &[], &rules).is_none());
    }

    #[test]
    fn single_match_trust_is_1() {
        let rules = vec![rule("feat", "type/feature")];
        let e = build_entry(42, "feat: add triage command", &[], &rules).unwrap();
        assert_eq!(e.number, 42);
        assert_eq!(e.matched_rule, "feat");
        assert_eq!(e.proposed_labels, "type/feature");
        assert!((e.trust_score - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn two_matches_trust_is_0_7() {
        let rules = vec![rule("feat", "type/feature"), rule("triage", "scope/triage")];
        let e = build_entry(1, "feat: add triage", &[], &rules).unwrap();
        assert!((e.trust_score - 0.7).abs() < f32::EPSILON);
    }

    #[test]
    fn three_plus_matches_trust_is_0_5() {
        let rules = vec![
            rule("feat", "type/feature"),
            rule("triage", "scope/triage"),
            rule("add", "scope/addition"),
        ];
        let e = build_entry(1, "feat: add triage support", &[], &rules).unwrap();
        assert!((e.trust_score - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn priority_label_included() {
        let rules = vec![rule_with_priority("fix", "type/bug", "priority/high")];
        let e = build_entry(7, "fix: critical crash", &[], &rules).unwrap();
        assert_eq!(e.proposed_labels, "type/bug, priority/high");
    }

    #[test]
    fn case_insensitive_match() {
        let rules = vec![rule("FEAT", "type/feature")];
        let e = build_entry(3, "feat: new feature", &[], &rules).unwrap();
        assert!(e.trust_score > 0.0);
    }

    #[test]
    fn truncate_long_title() {
        let long = "a".repeat(50);
        let out = truncate(&long, 42);
        let chars: Vec<char> = out.chars().collect();
        assert!(chars.len() <= 42);
        assert!(out.ends_with('…'));
    }

    #[test]
    fn truncate_short_title_unchanged() {
        let short = "short title";
        assert_eq!(truncate(short, 42), short);
    }

    #[test]
    fn already_labelled_flag() {
        let rules = vec![rule("feat", "type/feature")];
        let e = build_entry(5, "feat: something", &["type/feature".to_string()], &rules).unwrap();
        assert!(e.already_labelled);
    }
}
