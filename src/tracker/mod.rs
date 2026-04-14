pub mod jira;
pub mod github_issues;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::config::{ParsecConfig, TrackerProvider};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ticket {
    pub id: String,
    pub title: String,
    pub status: Option<String>,
    pub assignee: Option<String>,
    pub url: Option<String>,
}

/// Fetch a ticket from the configured tracker. Returns None if no tracker configured.
/// This is async because it makes HTTP calls.
pub async fn fetch_ticket(config: &ParsecConfig, id: &str) -> Result<Option<Ticket>> {
    match config.tracker.provider {
        TrackerProvider::Jira => {
            let jira_config = config.tracker.jira.as_ref()
                .ok_or_else(|| anyhow::anyhow!("Jira configured but no jira settings in config"))?;
            let tracker = jira::JiraTracker::new(
                &jira_config.base_url,
                jira_config.email.as_deref(),
            );
            let ticket = tracker.fetch_ticket(id).await?;
            Ok(Some(ticket))
        }
        TrackerProvider::Github => {
            let tracker = github_issues::GithubIssueTracker::new();
            let ticket = tracker.fetch_ticket(id).await?;
            Ok(Some(ticket))
        }
        TrackerProvider::None => Ok(None),
    }
}
