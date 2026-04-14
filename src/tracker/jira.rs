use anyhow::{bail, Context, Result};
use reqwest::Client;
use super::Ticket;

pub struct JiraTracker {
    base_url: String,
    email: Option<String>,
    client: Client,
}

impl JiraTracker {
    pub fn new(base_url: &str, email: Option<&str>) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            email: email.map(String::from),
            client: Client::new(),
        }
    }

    /// Resolve the Jira API token from environment.
    /// Priority: PARSEC_JIRA_TOKEN > JIRA_PAT
    fn resolve_token() -> Result<String> {
        if let Ok(token) = std::env::var("PARSEC_JIRA_TOKEN") {
            if !token.is_empty() {
                return Ok(token);
            }
        }
        if let Ok(token) = std::env::var("JIRA_PAT") {
            if !token.is_empty() {
                return Ok(token);
            }
        }
        anyhow::bail!(
            "No Jira token found. Set PARSEC_JIRA_TOKEN or JIRA_PAT environment variable."
        )
    }

    pub async fn fetch_ticket(&self, id: &str) -> Result<Ticket> {
        let token = Self::resolve_token()?;

        // Use API v2 (compatible with both Cloud and Server/DC)
        let url = format!("{}/rest/api/2/issue/{}", self.base_url, id);

        let mut request = self.client.get(&url)
            .header("Content-Type", "application/json");

        // If email is configured: Basic auth (Jira Cloud with API token)
        // Otherwise: Bearer token (Jira Server/DC with PAT)
        if let Some(ref email) = self.email {
            request = request.basic_auth(email, Some(&token));
        } else {
            request = request.bearer_auth(&token);
        }

        let response = request.send().await
            .context("Failed to send request to Jira")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            bail!("Jira API returned {} for {}: {}", status, id, body);
        }

        let body: serde_json::Value = response.json().await
            .context("Failed to parse Jira response")?;

        let title = body["fields"]["summary"]
            .as_str()
            .unwrap_or("Untitled")
            .to_string();

        let status = body["fields"]["status"]["name"]
            .as_str()
            .map(String::from);

        let assignee = body["fields"]["assignee"]["displayName"]
            .as_str()
            .map(String::from);

        Ok(Ticket {
            id: id.to_string(),
            title,
            status,
            assignee,
            url: Some(format!("{}/browse/{}", self.base_url, id)),
        })
    }
}
