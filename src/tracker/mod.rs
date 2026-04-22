pub mod github_issues;
pub mod jira;

use anyhow::Result;
use colored::Colorize;
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
                // Strip surrounding quotes (common in .env files)
                let value = if (value.starts_with('"') && value.ends_with('"'))
                    || (value.starts_with('\'') && value.ends_with('\''))
                {
                    &value[1..value.len() - 1]
                } else {
                    value
                };
                // Only allow specific prefixes for security
                let allowed_prefixes = ["JIRA_", "PARSEC_", "CONFLUENCE_", "ATLASSIAN_"];
                if !allowed_prefixes.iter().any(|p| key.starts_with(p)) {
                    eprintln!("warning: ignoring disallowed env var '{}' in .atlassian-env (allowed prefixes: JIRA_, PARSEC_, CONFLUENCE_, ATLASSIAN_)", key);
                    continue;
                }
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
            let tracker = github_issues::GithubIssueTracker::new(repo_root, config);
            let ticket = tracker.fetch_ticket(id).await?;
            Ok(Some(ticket))
        }
        TrackerProvider::Gitlab | TrackerProvider::None => {
            // Auto-detect Jira: try if env vars or config token available
            let has_jira_url =
                std::env::var(crate::env::JIRA_BASE_URL).is_ok() || config.tracker.jira.is_some();
            let config_token = config
                .tracker
                .jira
                .as_ref()
                .and_then(|j| j.token.as_deref());
            let has_jira_token = crate::env::jira_token(config_token).is_some();
            if has_jira_url && has_jira_token {
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
                            let tracker = github_issues::GithubIssueTracker::new(repo_root, config);
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

/// Try to transition a ticket's status. Warns on failure but never blocks.
pub async fn try_transition(config: &ParsecConfig, ticket: &str, target_status: &str) {
    // Only works for Jira currently
    if !matches!(
        config.tracker.provider,
        TrackerProvider::Jira | TrackerProvider::None
    ) {
        return;
    }

    // Need atlassian env loaded
    load_atlassian_env();

    let base_url = config
        .tracker
        .jira
        .as_ref()
        .map(|j| j.base_url.clone())
        .or_else(|| std::env::var(crate::env::JIRA_BASE_URL).ok());

    let base_url = match base_url {
        Some(url) => url,
        None => return,
    };

    let email = config.tracker.jira.as_ref().and_then(|j| j.email.clone());
    let config_token = config
        .tracker
        .jira
        .as_ref()
        .and_then(|j| j.token.as_deref());
    let jira = jira::JiraTracker::new(&base_url, email.as_deref(), config_token);

    match jira.transition_issue(ticket, target_status).await {
        Ok(()) => {
            eprintln!("  {} Ticket status → {}", "✓".green(), target_status);
        }
        Err(e) => {
            eprintln!("  warning: failed to transition ticket: {e}");
        }
    }
}

/// Post a comment on a ticket via the configured tracker.
///
/// Follows the same auto-detection logic as `fetch_ticket`: explicit provider
/// first, then Jira env-var auto-detect, then GitHub remote auto-detect.
pub async fn post_comment(
    config: &ParsecConfig,
    id: &str,
    body: &str,
    repo_root: Option<&Path>,
) -> Result<()> {
    load_atlassian_env();

    match config.tracker.provider {
        TrackerProvider::Jira => {
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
            let config_token = config
                .tracker
                .jira
                .as_ref()
                .and_then(|j| j.token.as_deref());
            let tracker = jira::JiraTracker::new(&base_url, email.as_deref(), config_token);
            tracker.add_comment(id, body).await
        }
        TrackerProvider::Github => {
            let tracker = github_issues::GithubIssueTracker::new(repo_root, config);
            tracker.add_comment(id, body).await
        }
        TrackerProvider::Gitlab | TrackerProvider::None => {
            // Auto-detect Jira
            let has_jira_url =
                std::env::var(crate::env::JIRA_BASE_URL).is_ok() || config.tracker.jira.is_some();
            let ct = config
                .tracker
                .jira
                .as_ref()
                .and_then(|j| j.token.as_deref());
            let has_jira_token = crate::env::jira_token(ct).is_some();
            if has_jira_url && has_jira_token {
                let base_url = config
                    .tracker
                    .jira
                    .as_ref()
                    .map(|j| j.base_url.clone())
                    .or_else(|| std::env::var(crate::env::JIRA_BASE_URL).ok())
                    .ok_or_else(|| anyhow::anyhow!("Jira base URL not configured"))?;
                let email = config.tracker.jira.as_ref().and_then(|j| j.email.clone());
                let config_token = config
                    .tracker
                    .jira
                    .as_ref()
                    .and_then(|j| j.token.as_deref());
                let tracker = jira::JiraTracker::new(&base_url, email.as_deref(), config_token);
                if tracker.add_comment(id, body).await.is_ok() {
                    return Ok(());
                }
            }

            // Auto-detect GitHub
            if let Some(root) = repo_root {
                if let Ok(remote_url) = crate::git::get_remote_url(root) {
                    if crate::github::parse_github_remote(&remote_url).is_some() {
                        let clean_id = id.trim_start_matches('#');
                        if clean_id.chars().all(|c| c.is_ascii_digit()) {
                            let tracker = github_issues::GithubIssueTracker::new(repo_root, config);
                            return tracker.add_comment(id, body).await;
                        }
                    }
                }
            }

            anyhow::bail!(
                "No tracker configured to post comments. \
                 Set tracker.provider in config or configure environment variables."
            )
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

    let config_token = config
        .tracker
        .jira
        .as_ref()
        .and_then(|j| j.token.as_deref());
    let tracker = jira::JiraTracker::new(&base_url, email.as_deref(), config_token);
    let ticket = tracker.fetch_ticket(id).await?;
    Ok(Some(ticket))
}
