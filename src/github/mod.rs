use anyhow::{bail, Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};

/// Result of PR creation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrResult {
    pub url: String,
    pub number: u64,
}

/// Parse `git@github.com:owner/repo.git` or `https://github.com/owner/repo.git`
/// into `(owner, repo)`.
pub fn parse_github_remote(url: &str) -> Option<(String, String)> {
    // SSH form: git@github.com:owner/repo.git
    if let Some(rest) = url.strip_prefix("git@github.com:") {
        let rest = rest.trim_end_matches(".git");
        let mut parts = rest.splitn(2, '/');
        let owner = parts.next()?.to_owned();
        let repo = parts.next()?.to_owned();
        return Some((owner, repo));
    }

    // HTTPS form
    if let Some(rest) = url
        .strip_prefix("https://github.com/")
        .or_else(|| url.strip_prefix("http://github.com/"))
    {
        let rest = rest.trim_end_matches(".git");
        let mut parts = rest.splitn(2, '/');
        let owner = parts.next()?.to_owned();
        let repo = parts.next()?.to_owned();
        return Some((owner, repo));
    }

    None
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

    let (owner, repo) = parse_github_remote(remote_url).ok_or_else(|| {
        anyhow::anyhow!("could not parse owner/repo from remote URL: {}", remote_url)
    })?;

    let api_url = format!("https://api.github.com/repos/{}/{}/pulls", owner, repo);

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
