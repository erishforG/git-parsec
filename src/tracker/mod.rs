pub mod github_issues;
pub mod jira;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::config::{ParsecConfig, TrackerProvider};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ticket {
    pub id: String,
    pub title: String,
    pub status: Option<String>,
    pub assignee: Option<String>,
    pub url: Option<String>,
}

/// Load environment variables from `~/.claude/.atlassian-env` if the file exists.
/// This provides seamless integration with Claude's Jira skill.
pub fn load_atlassian_env() {
    let env_path: PathBuf = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".claude")
        .join(".atlassian-env");

    if let Ok(contents) = std::fs::read_to_string(&env_path) {
        for line in contents.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((key, value)) = line.split_once('=') {
                let key = key.trim();
                let value = value.trim();
                // Only set if not already in environment (env vars take precedence)
                if std::env::var(key).is_err() {
                    std::env::set_var(key, value);
                }
            }
        }
    }
}

/// Fetch a ticket from the configured tracker. Returns None if no tracker configured.
///
/// Auto-detects Jira from `~/.claude/.atlassian-env` even if config says provider = "none".
/// Auto-detects GitHub Issues when the git remote points to github.com and the ticket
/// looks like a bare number or `#N`.
pub async fn fetch_ticket(
    config: &ParsecConfig,
    id: &str,
    repo_root: Option<&Path>,
) -> Result<Option<Ticket>> {
    // Load atlassian env file for seamless Claude Jira skill integration
    load_atlassian_env();

    match config.tracker.provider {
        TrackerProvider::Jira => fetch_jira_ticket(config, id).await,
        TrackerProvider::Github => {
            let tracker = github_issues::GithubIssueTracker::new(repo_root);
            let ticket = tracker.fetch_ticket(id).await?;
            Ok(Some(ticket))
        }
        TrackerProvider::Gitlab | TrackerProvider::None => {
            // Auto-detect Jira: try if env vars available, but don't block on failure
            if std::env::var(crate::env::JIRA_BASE_URL).is_ok()
                && (std::env::var(crate::env::JIRA_PAT).is_ok()
                    || std::env::var(crate::env::PARSEC_JIRA_TOKEN).is_ok())
            {
                if let Ok(Some(ticket)) = fetch_jira_ticket(config, id).await {
                    return Ok(Some(ticket));
                }
                // Jira failed (404, wrong ID format, etc.) — fall through to GitHub
            }

            // Auto-detect GitHub: if remote is github.com and ticket looks numeric
            if let Some(root) = repo_root {
                if let Ok(remote_url) = crate::git::get_remote_url(root) {
                    if crate::github::parse_github_remote(&remote_url).is_some() {
                        let clean_id = id.trim_start_matches('#');
                        if clean_id.chars().all(|c| c.is_ascii_digit()) {
                            let tracker = github_issues::GithubIssueTracker::new(repo_root);
                            if let Ok(ticket) = tracker.fetch_ticket(id).await {
                                return Ok(Some(ticket));
                            }
                        }
                    }
                }
            }

            Ok(None)
        }
    }
}

/// Internal: fetch from Jira, resolving base_url from config or env var.
async fn fetch_jira_ticket(config: &ParsecConfig, id: &str) -> Result<Option<Ticket>> {
    // Resolve base_url: config > JIRA_BASE_URL env var
    let base_url = config
        .tracker
        .jira
        .as_ref()
        .map(|j| j.base_url.clone())
        .or_else(|| std::env::var(crate::env::JIRA_BASE_URL).ok())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Jira base URL not found. Set it in config or {} env var.",
                crate::env::JIRA_BASE_URL,
            )
        })?;

    let email = config.tracker.jira.as_ref().and_then(|j| j.email.clone());

    let tracker = jira::JiraTracker::new(&base_url, email.as_deref());
    let ticket = tracker.fetch_ticket(id).await?;
    Ok(Some(ticket))
}
