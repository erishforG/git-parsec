pub mod jira;
pub mod github_issues;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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
fn load_atlassian_env() {
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
pub async fn fetch_ticket(config: &ParsecConfig, id: &str) -> Result<Option<Ticket>> {
    // Load atlassian env file for seamless Claude Jira skill integration
    load_atlassian_env();

    match config.tracker.provider {
        TrackerProvider::Jira => {
            fetch_jira_ticket(config, id).await
        }
        TrackerProvider::Github => {
            let tracker = github_issues::GithubIssueTracker::new();
            let ticket = tracker.fetch_ticket(id).await?;
            Ok(Some(ticket))
        }
        TrackerProvider::None => {
            // Auto-detect: if JIRA_BASE_URL is available, try Jira anyway
            if std::env::var("JIRA_BASE_URL").is_ok()
                && (std::env::var("JIRA_PAT").is_ok() || std::env::var("PARSEC_JIRA_TOKEN").is_ok())
            {
                fetch_jira_ticket(config, id).await
            } else {
                Ok(None)
            }
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
        .or_else(|| std::env::var("JIRA_BASE_URL").ok())
        .ok_or_else(|| anyhow::anyhow!(
            "Jira base URL not found. Set it in config or JIRA_BASE_URL env var."
        ))?;

    let email = config
        .tracker
        .jira
        .as_ref()
        .and_then(|j| j.email.clone());

    let tracker = jira::JiraTracker::new(&base_url, email.as_deref());
    let ticket = tracker.fetch_ticket(id).await?;
    Ok(Some(ticket))
}
