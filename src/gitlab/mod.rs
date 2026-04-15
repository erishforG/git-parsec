use anyhow::{bail, Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};

/// Result of MR creation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MrResult {
    pub url: String,
    pub iid: u64,
}

/// Parsed GitLab remote info
#[derive(Debug, Clone)]
pub struct GitLabRemote {
    pub host: String,
    pub project_path: String, // "owner/repo" or "group/subgroup/repo"
}

impl GitLabRemote {
    pub fn api_base(&self) -> String {
        format!("https://{}/api/v4", self.host)
    }
}

/// Parse a GitLab remote URL into GitLabRemote.
/// Supports SSH and HTTPS forms, including nested groups.
pub fn parse_gitlab_remote(url: &str) -> Option<GitLabRemote> {
    // SSH: git@gitlab.com:group/subgroup/repo.git
    if url.starts_with("git@") {
        let rest = url.strip_prefix("git@")?;
        let (host, path) = rest.split_once(':')?;
        let path = path.trim_end_matches(".git");
        return Some(GitLabRemote {
            host: host.to_owned(),
            project_path: path.to_owned(),
        });
    }

    // HTTPS: https://gitlab.com/group/subgroup/repo.git
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let (host, path) = rest.split_once('/')?;
    let path = path.trim_end_matches(".git");
    Some(GitLabRemote {
        host: host.to_owned(),
        project_path: path.to_owned(),
    })
}

/// Resolve a GitLab token from environment variables.
/// Checks: PARSEC_GITLAB_TOKEN > GITLAB_TOKEN
fn resolve_gitlab_token() -> Option<String> {
    for var in &["PARSEC_GITLAB_TOKEN", "GITLAB_TOKEN"] {
        if let Ok(token) = std::env::var(var) {
            if !token.is_empty() {
                return Some(token);
            }
        }
    }
    None
}

/// Create a GitLab merge request.
/// Returns None if no GitLab token is available.
pub async fn create_mr(
    remote_url: &str,
    branch: &str,
    base: &str,
    title: &str,
    description: &str,
    draft: bool,
) -> Result<Option<MrResult>> {
    let token = match resolve_gitlab_token() {
        Some(t) => t,
        None => return Ok(None),
    };

    let remote = parse_gitlab_remote(remote_url).ok_or_else(|| {
        anyhow::anyhow!(
            "could not parse project path from remote URL: {}",
            remote_url
        )
    })?;

    // URL-encode the project path for the API
    let encoded_path = remote.project_path.replace('/', "%2F");
    let api_url = format!(
        "{}/projects/{}/merge_requests",
        remote.api_base(),
        encoded_path
    );

    let mr_title = if draft {
        format!("Draft: {}", title)
    } else {
        title.to_owned()
    };

    let payload = serde_json::json!({
        "source_branch": branch,
        "target_branch": base,
        "title": mr_title,
        "description": description,
    });

    let client = Client::new();
    let response = client
        .post(&api_url)
        .header("Content-Type", "application/json")
        .header("User-Agent", "git-parsec")
        .header("PRIVATE-TOKEN", &token)
        .json(&payload)
        .send()
        .await
        .context("Failed to send MR creation request to GitLab")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        bail!("GitLab API returned {}: {}", status, body);
    }

    let resp: serde_json::Value = response
        .json()
        .await
        .context("Failed to parse GitLab API response")?;

    let web_url = resp["web_url"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("GitLab response missing web_url"))?
        .to_owned();

    let iid = resp["iid"].as_u64().unwrap_or(0);

    Ok(Some(MrResult { url: web_url, iid }))
}
