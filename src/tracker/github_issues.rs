use anyhow::Result;
use super::{Ticket, TicketTracker};

pub struct GithubIssueTracker;

impl GithubIssueTracker {
    pub fn new() -> Self {
        Self
    }
}

impl TicketTracker for GithubIssueTracker {
    fn fetch_ticket(&self, id: &str) -> Result<Ticket> {
        // Stub: parse #123 format, return basic ticket
        let issue_num = id.trim_start_matches('#');

        // Try to use PARSEC_GITHUB_TOKEN to fetch from GitHub API
        // For now, return a basic ticket without API call
        Ok(Ticket {
            id: id.to_string(),
            title: format!("GitHub Issue {}", issue_num),
            status: None,
            assignee: None,
            url: None, // Would need repo context to build URL
        })
    }
}
