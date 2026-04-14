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

pub trait TicketTracker {
    fn fetch_ticket(&self, id: &str) -> Result<Ticket>;
}

/// Create a tracker from config. Returns None if provider is None.
pub fn create_tracker(config: &ParsecConfig) -> Option<Box<dyn TicketTracker>> {
    match config.tracker.provider {
        TrackerProvider::Jira => {
            let jira_config = config.tracker.jira.as_ref()?;
            Some(Box::new(jira::JiraTracker::new(
                &jira_config.base_url,
                jira_config.email.as_deref(),
            )))
        }
        TrackerProvider::Github => {
            Some(Box::new(github_issues::GithubIssueTracker::new()))
        }
        TrackerProvider::None => None,
    }
}
