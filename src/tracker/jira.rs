use super::Ticket;
use anyhow::{bail, Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SprintInfo {
    pub id: u64,
    pub name: String,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardTicket {
    pub key: String,
    pub summary: String,
    pub status: String,
    pub assignee: Option<String>,
}

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
        crate::env::jira_token().ok_or_else(|| {
            anyhow::anyhow!(
                "No Jira token found. Set {} or {} environment variable.",
                crate::env::PARSEC_JIRA_TOKEN,
                crate::env::JIRA_PAT,
            )
        })
    }

    /// Fetch a single issue via Jira REST API v2.
    /// Requires: Jira (Cloud or Server/DC 7.x+)
    /// Endpoint: GET /rest/api/2/issue/{id}
    pub async fn fetch_ticket(&self, id: &str) -> Result<Ticket> {
        let token = Self::resolve_token()?;

        let url = format!("{}/rest/api/2/issue/{}", self.base_url, id);

        let mut request = self
            .client
            .get(&url)
            .header("Content-Type", "application/json");

        // If email is configured: Basic auth (Jira Cloud with API token)
        // Otherwise: Bearer token (Jira Server/DC with PAT)
        if let Some(ref email) = self.email {
            request = request.basic_auth(email, Some(&token));
        } else {
            request = request.bearer_auth(&token);
        }

        let response = request
            .send()
            .await
            .context("Failed to send request to Jira")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            bail!("Jira API returned {} for {}: {}", status, id, body);
        }

        let body: serde_json::Value = response
            .json()
            .await
            .context("Failed to parse Jira response")?;

        let title = body["fields"]["summary"]
            .as_str()
            .unwrap_or("Untitled")
            .to_string();

        let status = body["fields"]["status"]["name"].as_str().map(String::from);

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

    /// Fetch the first board ID for a project via Jira Agile REST API v1.0.
    /// Requires: Jira Software (Cloud or Server/DC 7.x+)
    /// Endpoint: GET /rest/agile/1.0/board?projectKeyOrId={project}
    pub async fn fetch_board_id(&self, project: &str) -> Result<u64> {
        let token = Self::resolve_token()?;
        let url = format!(
            "{}/rest/agile/1.0/board?projectKeyOrId={}",
            self.base_url, project
        );

        let mut request = self
            .client
            .get(&url)
            .header("Content-Type", "application/json");

        if let Some(ref email) = self.email {
            request = request.basic_auth(email, Some(&token));
        } else {
            request = request.bearer_auth(&token);
        }

        let response = request
            .send()
            .await
            .context("Failed to fetch boards from Jira")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            bail!("Jira Agile API returned {} for boards: {}", status, body);
        }

        let body: serde_json::Value = response
            .json()
            .await
            .context("Failed to parse board response")?;

        body["values"]
            .as_array()
            .and_then(|boards| boards.first())
            .and_then(|b| b["id"].as_u64())
            .ok_or_else(|| anyhow::anyhow!("No board found for project {project}"))
    }

    /// Fetch the active sprint for a board via Jira Agile REST API v1.0.
    /// Requires: Jira Software (Cloud or Server/DC 7.x+)
    /// Endpoint: GET /rest/agile/1.0/board/{id}/sprint?state=active
    pub async fn fetch_active_sprint(&self, board_id: u64) -> Result<SprintInfo> {
        let token = Self::resolve_token()?;
        let url = format!(
            "{}/rest/agile/1.0/board/{}/sprint?state=active",
            self.base_url, board_id
        );

        let mut request = self
            .client
            .get(&url)
            .header("Content-Type", "application/json");

        if let Some(ref email) = self.email {
            request = request.basic_auth(email, Some(&token));
        } else {
            request = request.bearer_auth(&token);
        }

        let response = request
            .send()
            .await
            .context("Failed to fetch sprints from Jira")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            bail!("Jira Agile API returned {} for sprints: {}", status, body);
        }

        let body: serde_json::Value = response
            .json()
            .await
            .context("Failed to parse sprint response")?;

        let sprint = body["values"]
            .as_array()
            .and_then(|sprints| sprints.first())
            .ok_or_else(|| anyhow::anyhow!("No active sprint found for board {board_id}"))?;

        Ok(SprintInfo {
            id: sprint["id"].as_u64().unwrap_or(0),
            name: sprint["name"].as_str().unwrap_or("").to_string(),
            start_date: sprint["startDate"].as_str().map(String::from),
            end_date: sprint["endDate"].as_str().map(String::from),
        })
    }

    /// Fetch all issues in a sprint via Jira Agile REST API v1.0.
    /// Requires: Jira Software (Cloud or Server/DC 7.x+)
    /// Endpoint: GET /rest/agile/1.0/sprint/{id}/issue?fields=summary,status,assignee
    pub async fn fetch_sprint_issues(&self, sprint_id: u64) -> Result<Vec<BoardTicket>> {
        let token = Self::resolve_token()?;
        let url = format!(
            "{}/rest/agile/1.0/sprint/{}/issue?fields=summary,status,assignee&maxResults=200",
            self.base_url, sprint_id
        );

        let mut request = self
            .client
            .get(&url)
            .header("Content-Type", "application/json");

        if let Some(ref email) = self.email {
            request = request.basic_auth(email, Some(&token));
        } else {
            request = request.bearer_auth(&token);
        }

        let response = request
            .send()
            .await
            .context("Failed to fetch sprint issues from Jira")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            bail!(
                "Jira Agile API returned {} for sprint issues: {}",
                status,
                body
            );
        }

        let body: serde_json::Value = response
            .json()
            .await
            .context("Failed to parse sprint issues response")?;

        let issues = body["issues"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .map(|issue| BoardTicket {
                        key: issue["key"].as_str().unwrap_or("").to_string(),
                        summary: issue["fields"]["summary"]
                            .as_str()
                            .unwrap_or("")
                            .to_string(),
                        status: issue["fields"]["status"]["name"]
                            .as_str()
                            .unwrap_or("Unknown")
                            .to_string(),
                        assignee: issue["fields"]["assignee"]["displayName"]
                            .as_str()
                            .map(String::from),
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(issues)
    }
}
