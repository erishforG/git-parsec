use anyhow::{bail, Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};

/// Result of PR creation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrResult {
    pub url: String,
    pub number: u64,
}

/// Parsed GitHub remote info including the host (for Enterprise support).
#[derive(Debug, Clone)]
pub struct GitHubRemote {
    pub host: String,
    pub owner: String,
    pub repo: String,
}

impl GitHubRemote {
    /// Return the API base URL for this remote.
    /// - `github.com` → `https://api.github.com`
    /// - Enterprise (e.g. `github.daumkakao.com`) → `https://{host}/api/v3`
    pub fn api_base(&self) -> String {
        if self.host == "github.com" {
            "https://api.github.com".to_string()
        } else {
            format!("https://{}/api/v3", self.host)
        }
    }

    /// Return the browse URL for this remote.
    #[allow(dead_code)]
    pub fn browse_url(&self, path: &str) -> String {
        format!(
            "https://{}/{}/{}/{}",
            self.host, self.owner, self.repo, path
        )
    }
}

/// Parse any GitHub remote URL (github.com or Enterprise) into GitHubRemote.
///
/// Supports:
/// - SSH: `git@github.com:owner/repo.git`, `git@github.enterprise.com:owner/repo.git`
/// - HTTPS: `https://github.com/owner/repo.git`, `https://github.enterprise.com/owner/repo.git`
pub fn parse_github_remote(url: &str) -> Option<GitHubRemote> {
    // SSH form: git@<host>:owner/repo.git
    if url.starts_with("git@") {
        let rest = url.strip_prefix("git@")?;
        let (host, path) = rest.split_once(':')?;
        let path = path.trim_end_matches(".git");
        let mut parts = path.splitn(2, '/');
        let owner = parts.next()?.to_owned();
        let repo = parts.next()?.to_owned();
        return Some(GitHubRemote {
            host: host.to_owned(),
            owner,
            repo,
        });
    }

    // HTTPS form: https://<host>/owner/repo.git
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let (host, path) = rest.split_once('/')?;
    let path = path.trim_end_matches(".git");
    let mut parts = path.splitn(2, '/');
    let owner = parts.next()?.to_owned();
    let repo = parts.next()?.to_owned();
    Some(GitHubRemote {
        host: host.to_owned(),
        owner,
        repo,
    })
}

/// Resolve a GitHub token from environment variables.
/// Checks: PARSEC_GITHUB_TOKEN > GITHUB_TOKEN > GH_TOKEN
fn resolve_github_token() -> Option<String> {
    for var in &["PARSEC_GITHUB_TOKEN", "GITHUB_TOKEN", "GH_TOKEN"] {
        if let Ok(token) = std::env::var(var) {
            if !token.is_empty() {
                return Some(token);
            }
        }
    }
    None
}

/// PR status information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrStatus {
    pub number: u64,
    pub title: String,
    pub state: String,
    pub mergeable: Option<bool>,
    pub ci_status: String,
    pub review_status: String,
    pub url: String,
}

/// Fetch the status of a GitHub PR by number.
pub async fn get_pr_status(remote_url: &str, pr_number: u64) -> Result<Option<PrStatus>> {
    let token = match resolve_github_token() {
        Some(t) => t,
        None => return Ok(None),
    };

    let remote = parse_github_remote(remote_url).ok_or_else(|| {
        anyhow::anyhow!("could not parse owner/repo from remote URL: {}", remote_url)
    })?;

    let api_base = remote.api_base();
    let client = Client::new();

    // Fetch PR details
    let pr_url = format!(
        "{}/repos/{}/{}/pulls/{}",
        api_base, remote.owner, remote.repo, pr_number
    );
    let pr_resp: serde_json::Value = client
        .get(&pr_url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .header("User-Agent", "git-parsec")
        .bearer_auth(&token)
        .send()
        .await?
        .json()
        .await?;

    let title = pr_resp["title"].as_str().unwrap_or("").to_string();
    let state = pr_resp["state"].as_str().unwrap_or("unknown").to_string();
    let mergeable = pr_resp["mergeable"].as_bool();
    let html_url = pr_resp["html_url"].as_str().unwrap_or("").to_string();
    let head_sha = pr_resp["head"]["sha"].as_str().unwrap_or("");

    // Fetch combined commit status
    let ci_status = if !head_sha.is_empty() {
        let status_url = format!(
            "{}/repos/{}/{}/commits/{}/status",
            api_base, remote.owner, remote.repo, head_sha
        );
        let status_resp: serde_json::Value = client
            .get(&status_url)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("User-Agent", "git-parsec")
            .bearer_auth(&token)
            .send()
            .await?
            .json()
            .await?;
        status_resp["state"]
            .as_str()
            .unwrap_or("unknown")
            .to_string()
    } else {
        "unknown".to_string()
    };

    // Fetch reviews
    let reviews_url = format!(
        "{}/repos/{}/{}/pulls/{}/reviews",
        api_base, remote.owner, remote.repo, pr_number
    );
    let reviews_resp: Vec<serde_json::Value> = client
        .get(&reviews_url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .header("User-Agent", "git-parsec")
        .bearer_auth(&token)
        .send()
        .await?
        .json()
        .await?;

    let review_status = if reviews_resp.iter().any(|r| {
        r["state"]
            .as_str()
            .is_some_and(|s| s == "CHANGES_REQUESTED")
    }) {
        "changes_requested".to_string()
    } else if reviews_resp
        .iter()
        .any(|r| r["state"].as_str().is_some_and(|s| s == "APPROVED"))
    {
        "approved".to_string()
    } else if reviews_resp.is_empty() {
        "no reviews".to_string()
    } else {
        "pending".to_string()
    };

    Ok(Some(PrStatus {
        number: pr_number,
        title,
        state,
        mergeable,
        ci_status,
        review_status,
        url: html_url,
    }))
}

/// Create a GitHub pull request.
/// Returns None if no GitHub token is available.
pub async fn create_pr(
    remote_url: &str,
    branch: &str,
    base: &str,
    title: &str,
    body: &str,
    draft: bool,
) -> Result<Option<PrResult>> {
    let token = match resolve_github_token() {
        Some(t) => t,
        None => return Ok(None),
    };

    let remote = parse_github_remote(remote_url).ok_or_else(|| {
        anyhow::anyhow!("could not parse owner/repo from remote URL: {}", remote_url)
    })?;

    let api_url = format!(
        "{}/repos/{}/{}/pulls",
        remote.api_base(),
        remote.owner,
        remote.repo
    );

    let payload = serde_json::json!({
        "title": title,
        "head": branch,
        "base": base,
        "body": body,
        "draft": draft,
    });

    let client = Client::new();
    let response = client
        .post(&api_url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .header("User-Agent", "git-parsec")
        .bearer_auth(&token)
        .json(&payload)
        .send()
        .await
        .context("Failed to send PR creation request to GitHub")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        bail!("GitHub API returned {}: {}", status, body);
    }

    let resp: serde_json::Value = response
        .json()
        .await
        .context("Failed to parse GitHub API response")?;

    let html_url = resp["html_url"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("GitHub response missing html_url"))?
        .to_owned();

    let number = resp["number"].as_u64().unwrap_or(0);

    Ok(Some(PrResult {
        url: html_url,
        number,
    }))
}
