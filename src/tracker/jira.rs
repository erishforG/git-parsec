use anyhow::{Context, Result, bail};
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

    pub async fn fetch_ticket(&self, id: &str) -> Result<Ticket> {
        let token = std::env::var("PARSEC_JIRA_TOKEN")
            .context("PARSEC_JIRA_TOKEN environment variable not set")?;

        let url = format!("{}/rest/api/3/issue/{}", self.base_url, id);

        let mut request = self.client.get(&url)
            .header("Content-Type", "application/json");

        // Jira Cloud: Basic auth with email:api_token
        // Jira Server/DC: Bearer token
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
            bail!("Jira API returned {}: {}", status, body);
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
