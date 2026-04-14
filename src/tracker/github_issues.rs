use anyhow::Result;
use super::Ticket;

pub struct GithubIssueTracker;

impl GithubIssueTracker {
    pub fn new() -> Self {
        Self
    }

    pub async fn fetch_ticket(&self, id: &str) -> Result<Ticket> {
        let issue_num = id.trim_start_matches('#');

        // Try GitHub API if token is available
        if let Ok(token) = std::env::var("PARSEC_GITHUB_TOKEN") {
            // We'd need repo context to build the full URL
            // For now return a basic ticket with the number
            // In the future, this could use the repo's remote URL
            let _ = token; // suppress unused warning
        }

        Ok(Ticket {
            id: id.to_string(),
            title: format!("GitHub Issue {}", issue_num),
            status: None,
            assignee: None,
            url: None,
        })
    }
}
