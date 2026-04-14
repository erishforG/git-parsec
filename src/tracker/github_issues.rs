use anyhow::{Context, Result};
use reqwest::Client;
use std::path::{Path, PathBuf};

use super::Ticket;

pub struct GithubIssueTracker {
    repo_root: Option<PathBuf>,
    client: Client,
}

impl GithubIssueTracker {
    pub fn new(repo_root: Option<&Path>) -> Self {
        Self {
            repo_root: repo_root.map(|p| p.to_path_buf()),
            client: Client::new(),
        }
    }

    fn resolve_token() -> Option<String> {
        for var in &["PARSEC_GITHUB_TOKEN", "GITHUB_TOKEN", "GH_TOKEN"] {
            if let Ok(token) = std::env::var(var) {
                if !token.is_empty() {
                    return Some(token);
                }
            }
        }
        None
    }

    pub async fn fetch_ticket(&self, id: &str) -> Result<Ticket> {
        let issue_num = id.trim_start_matches('#');

        let token = match Self::resolve_token() {
            Some(t) => t,
            None => return Ok(self.fallback_ticket(id)),
        };

        let (owner, repo) = match self.resolve_owner_repo() {
            Some(pair) => pair,
            None => return Ok(self.fallback_ticket(id)),
        };

        let url = format!(
            "https://api.github.com/repos/{}/{}/issues/{}",
            owner, repo, issue_num
        );

        let response = self
            .client
            .get(&url)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("User-Agent", "git-parsec")
            .bearer_auth(&token)
            .send()
            .await
            .context("Failed to send request to GitHub Issues API")?;

        if !response.status().is_success() {
            // Don't error - fall back gracefully
            return Ok(self.fallback_ticket(id));
        }

        let body: serde_json::Value = response
            .json()
            .await
            .context("Failed to parse GitHub Issues response")?;

        let title = body["title"].as_str().unwrap_or("Untitled").to_string();

        let status = body["state"].as_str().map(String::from);

        let assignee = body["assignee"]["login"].as_str().map(String::from);

        let html_url = body["html_url"].as_str().map(String::from);

        Ok(Ticket {
            id: id.to_string(),
            title,
            status,
            assignee,
            url: html_url,
        })
    }

    fn resolve_owner_repo(&self) -> Option<(String, String)> {
        let repo_root = self.repo_root.as_ref()?;
        let remote_url = crate::git::get_remote_url(repo_root).ok()?;
        crate::github::parse_github_remote(&remote_url)
    }

    fn fallback_ticket(&self, id: &str) -> Ticket {
        let issue_num = id.trim_start_matches('#');
        Ticket {
            id: id.to_string(),
            title: format!("#{}", issue_num),
            status: None,
            assignee: None,
            url: None,
        }
    }
}
