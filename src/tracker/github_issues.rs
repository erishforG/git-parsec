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
        crate::env::github_token()
    }

    pub async fn fetch_ticket(&self, id: &str) -> Result<Ticket> {
        let issue_num = id.trim_start_matches('#');

        let token = match Self::resolve_token() {
            Some(t) => t,
            None => return Ok(self.fallback_ticket(id)),
        };

        let remote = match self.resolve_remote() {
            Some(r) => r,
            None => return Ok(self.fallback_ticket(id)),
        };

        let url = format!(
            "{}/repos/{}/{}/issues/{}",
            remote.api_base(),
            remote.owner,
            remote.repo,
            issue_num
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

    /// Post a comment on a GitHub issue.
    /// Endpoint: POST /repos/{owner}/{repo}/issues/{number}/comments
    pub async fn add_comment(&self, id: &str, body: &str) -> Result<()> {
        let issue_num = id.trim_start_matches('#');

        let token = Self::resolve_token().ok_or_else(|| {
            anyhow::anyhow!("No GitHub token found. Set GITHUB_TOKEN or GH_TOKEN.")
        })?;

        let remote = self
            .resolve_remote()
            .ok_or_else(|| anyhow::anyhow!("Could not determine GitHub remote from repository."))?;

        let url = format!(
            "{}/repos/{}/{}/issues/{}/comments",
            remote.api_base(),
            remote.owner,
            remote.repo,
            issue_num
        );

        let payload = serde_json::json!({
            "body": body
        });

        let response = self
            .client
            .post(&url)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("User-Agent", "git-parsec")
            .bearer_auth(&token)
            .json(&payload)
            .send()
            .await
            .context("Failed to send comment request to GitHub Issues API")?;

        if !response.status().is_success() {
            let status = response.status();
            let resp_body = response.text().await.unwrap_or_default();
            anyhow::bail!(
                "GitHub Issues comment API returned {} for #{}: {}",
                status,
                issue_num,
                resp_body
            );
        }

        Ok(())
    }

    fn resolve_remote(&self) -> Option<crate::github::GitHubRemote> {
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
