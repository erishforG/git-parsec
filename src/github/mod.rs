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
    crate::env::github_token()
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

/// A single CI check run
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckRun {
    pub name: String,
    pub status: String,
    pub conclusion: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub html_url: Option<String>,
}

/// Aggregated CI status for a PR
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CiStatus {
    pub pr_number: u64,
    pub head_sha: String,
    pub overall: String,
    pub checks: Vec<CheckRun>,
}

/// Fetch check runs for a PR by number.
/// Returns None if no GitHub token is available.
pub async fn get_check_runs(remote_url: &str, pr_number: u64) -> Result<Option<CiStatus>> {
    let token = match resolve_github_token() {
        Some(t) => t,
        None => return Ok(None),
    };

    let remote = parse_github_remote(remote_url).ok_or_else(|| {
        anyhow::anyhow!("could not parse owner/repo from remote URL: {}", remote_url)
    })?;

    let api_base = remote.api_base();
    let client = Client::new();

    // Fetch PR to get head SHA
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

    let head_sha = pr_resp["head"]["sha"].as_str().unwrap_or("").to_string();

    if head_sha.is_empty() {
        bail!("could not determine head SHA for PR #{}", pr_number);
    }

    // Fetch check runs for the head SHA
    let checks_url = format!(
        "{}/repos/{}/{}/commits/{}/check-runs",
        api_base, remote.owner, remote.repo, head_sha
    );
    let checks_resp: serde_json::Value = client
        .get(&checks_url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .header("User-Agent", "git-parsec")
        .bearer_auth(&token)
        .send()
        .await?
        .json()
        .await?;

    let checks: Vec<CheckRun> = checks_resp["check_runs"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .map(|c| CheckRun {
            name: c["name"].as_str().unwrap_or("").to_string(),
            status: c["status"].as_str().unwrap_or("").to_string(),
            conclusion: c["conclusion"].as_str().map(|s| s.to_string()),
            started_at: c["started_at"].as_str().map(|s| s.to_string()),
            completed_at: c["completed_at"].as_str().map(|s| s.to_string()),
            html_url: c["html_url"].as_str().map(|s| s.to_string()),
        })
        .collect();

    // Derive overall status
    let overall = if checks.is_empty() {
        "no checks".to_string()
    } else if checks
        .iter()
        .any(|c| c.conclusion.as_deref() == Some("failure"))
    {
        "failing".to_string()
    } else if checks.iter().all(|c| {
        c.conclusion.as_deref() == Some("success") || c.conclusion.as_deref() == Some("skipped")
    }) {
        "passing".to_string()
    } else {
        "pending".to_string()
    };

    Ok(Some(CiStatus {
        pr_number,
        head_sha,
        overall,
        checks,
    }))
}

/// Find an open PR by branch name.
/// Returns the PR number if found, None if no token or no matching PR.
pub async fn find_pr_by_branch(remote_url: &str, branch: &str) -> Result<Option<u64>> {
    let token = match resolve_github_token() {
        Some(t) => t,
        None => return Ok(None),
    };

    let remote = parse_github_remote(remote_url).ok_or_else(|| {
        anyhow::anyhow!("could not parse owner/repo from remote URL: {}", remote_url)
    })?;

    let api_base = remote.api_base();
    let client = Client::new();

    let url = format!(
        "{}/repos/{}/{}/pulls?head={}:{}&state=open",
        api_base, remote.owner, remote.repo, remote.owner, branch
    );
    let resp: Vec<serde_json::Value> = client
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .header("User-Agent", "git-parsec")
        .bearer_auth(&token)
        .send()
        .await?
        .json()
        .await?;

    Ok(resp.first().and_then(|pr| pr["number"].as_u64()))
}

/// Result of merging a PR
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeResult {
    pub sha: String,
    pub message: String,
    pub merged: bool,
}

/// Merge a GitHub PR.
/// `method` should be "squash", "rebase", or "merge".
/// Returns None if no token, Some(MergeResult) on success.
pub async fn merge_pr(
    remote_url: &str,
    pr_number: u64,
    method: &str,
    delete_branch: bool,
) -> Result<Option<MergeResult>> {
    let token = match resolve_github_token() {
        Some(t) => t,
        None => return Ok(None),
    };

    let remote = parse_github_remote(remote_url).ok_or_else(|| {
        anyhow::anyhow!("could not parse owner/repo from remote URL: {}", remote_url)
    })?;

    let api_base = remote.api_base();
    let client = Client::new();

    // Merge the PR
    let url = format!(
        "{}/repos/{}/{}/pulls/{}/merge",
        api_base, remote.owner, remote.repo, pr_number
    );
    let payload = serde_json::json!({
        "merge_method": method,
    });

    let response = client
        .put(&url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .header("User-Agent", "git-parsec")
        .bearer_auth(&token)
        .json(&payload)
        .send()
        .await
        .context("Failed to send merge request to GitHub")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        bail!("GitHub merge API returned {}: {}", status, body);
    }

    let resp: serde_json::Value = response.json().await?;
    let sha = resp["sha"].as_str().unwrap_or("").to_string();
    let message = resp["message"].as_str().unwrap_or("").to_string();

    // Delete remote branch if requested
    if delete_branch {
        let branch_url = format!(
            "{}/repos/{}/{}/pulls/{}",
            api_base, remote.owner, remote.repo, pr_number
        );
        let pr_resp: serde_json::Value = client
            .get(&branch_url)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("User-Agent", "git-parsec")
            .bearer_auth(&token)
            .send()
            .await?
            .json()
            .await?;

        if let Some(branch_name) = pr_resp["head"]["ref"].as_str() {
            let del_url = format!(
                "{}/repos/{}/{}/git/refs/heads/{}",
                api_base, remote.owner, remote.repo, branch_name
            );
            let _ = client
                .delete(&del_url)
                .header("Accept", "application/vnd.github+json")
                .header("X-GitHub-Api-Version", "2022-11-28")
                .header("User-Agent", "git-parsec")
                .bearer_auth(&token)
                .send()
                .await;
        }
    }

    Ok(Some(MergeResult {
        sha,
        message,
        merged: true,
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
