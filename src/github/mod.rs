use std::time::Duration;

use anyhow::{bail, Context, Result};
use reqwest::{Client, Response};
use serde::{Deserialize, Serialize};

use crate::config::ParsecConfig;

// ---------------------------------------------------------------------------
// HTTP helpers (private)
// ---------------------------------------------------------------------------

/// Create a configured HTTP client with timeout.
fn http_client() -> Result<Client> {
    Client::builder()
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(10))
        .user_agent("git-parsec")
        .build()
        .context("failed to build HTTP client")
}

/// Execute an HTTP request with retry for transient failures (429, 5xx).
async fn send_with_retry(request_builder: reqwest::RequestBuilder) -> Result<Response> {
    let mut attempts = 0;
    let max_retries = 3;
    loop {
        // We need to clone the builder since send() consumes it
        let builder = request_builder
            .try_clone()
            .ok_or_else(|| anyhow::anyhow!("request cannot be retried (streaming body)"))?;
        let response = builder.send().await?;
        let status = response.status().as_u16();
        attempts += 1;
        if status == 429 || (500..600).contains(&status) {
            if attempts >= max_retries {
                return Ok(response); // let caller handle the error status
            }
            let wait = if status == 429 {
                // Check Retry-After header
                response
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(2)
            } else {
                2u64.pow(attempts as u32 - 1) // exponential backoff: 1, 2, 4
            };
            tokio::time::sleep(Duration::from_secs(wait)).await;
            continue;
        }
        return Ok(response);
    }
}

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

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

/// Result of merging a PR
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeResult {
    pub sha: String,
    pub message: String,
    pub merged: bool,
}

/// Basic PR info for checkout/review workflows.
#[derive(Debug, Clone)]
pub struct PrInfo {
    pub title: String,
    pub head_branch: String,
}

/// Result of issue creation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueResult {
    pub number: u64,
    pub url: String,
}

// ---------------------------------------------------------------------------
// Free functions (parsing, token resolution)
// ---------------------------------------------------------------------------

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

/// Returns true when `host` looks like a GitHub host. github.com and any host
/// with `.github.` (GHE) substring qualifies. Used to gate env-var and
/// `gh auth token` fallbacks so they don't leak into other forges.
pub fn is_github_host(host: &str) -> bool {
    let h = host.trim().to_ascii_lowercase();
    h == "github.com" || h.contains(".github.") || h.ends_with(".ghe.com")
}

/// Resolve a GitHub token for the given host.
///
/// Resolution priority:
/// 1. `config.github.<host>.token` — host-specific config (any host)
/// 2. `PARSEC_GITHUB_TOKEN` / `GITHUB_TOKEN` / `GH_TOKEN` env vars (GitHub host only)
/// 3. `gh auth token` shell fallback (GitHub host only) — issue #281 parity
///
/// 2 & 3 are gated on host being a GitHub host so that bitbucket / gitlab remotes
/// don't accidentally pick up a GitHub token via `gh auth login`.
pub fn resolve_github_token(host: &str, config: &ParsecConfig) -> Option<String> {
    // 1. Host-specific config token (any host — opt-in via config)
    if let Some(host_cfg) = config.github.get(host) {
        if let Some(ref token) = host_cfg.token {
            if !token.is_empty() {
                return Some(token.clone());
            }
        }
    }

    // 2 & 3: env / gh CLI fallback — only for actual GitHub hosts.
    if !is_github_host(host) {
        return None;
    }
    crate::env::github_token()
}

// ---------------------------------------------------------------------------
// GitHubClient
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// GitHub API response types (private, for deserialization)
// ---------------------------------------------------------------------------

#[derive(Deserialize, Default)]
struct ApiPrHead {
    #[serde(default)]
    sha: String,
    #[serde(rename = "ref", default)]
    ref_name: String,
}

#[derive(Deserialize)]
struct ApiPr {
    #[serde(default)]
    number: u64,
    #[serde(default)]
    title: String,
    #[serde(default)]
    state: String,
    mergeable: Option<bool>,
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    head: ApiPrHead,
    /// Present on error responses (e.g. 404 "Not Found").
    message: Option<String>,
}

#[derive(Deserialize)]
struct ApiCombinedStatus {
    #[serde(default)]
    state: String,
}

#[derive(Deserialize)]
struct ApiReview {
    #[serde(default)]
    state: String,
}

#[derive(Deserialize)]
struct ApiCheckRunEntry {
    #[serde(default)]
    name: String,
    #[serde(default)]
    status: String,
    conclusion: Option<String>,
    started_at: Option<String>,
    completed_at: Option<String>,
    html_url: Option<String>,
}

#[derive(Deserialize)]
struct ApiCheckRunsResponse {
    #[serde(default)]
    check_runs: Vec<ApiCheckRunEntry>,
}

#[derive(Deserialize)]
struct ApiMergeResponse {
    #[serde(default)]
    sha: String,
    #[serde(default)]
    message: String,
}

#[derive(Deserialize)]
struct ApiCreateResponse {
    number: Option<u64>,
    html_url: Option<String>,
}

#[derive(Deserialize)]
struct ApiPrListItem {
    number: Option<u64>,
}

#[derive(Deserialize)]
struct ApiUserResponse {
    #[serde(default)]
    login: String,
}

#[derive(Deserialize)]
struct ApiSearchItem {
    #[serde(default)]
    number: u64,
    #[serde(default)]
    title: String,
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    state: String,
}

#[derive(Deserialize)]
struct ApiSearchResponse {
    #[serde(default)]
    items: Vec<ApiSearchItem>,
}

// ---------------------------------------------------------------------------
// GitHubClient
// ---------------------------------------------------------------------------

/// Authenticated GitHub API client that eliminates per-call boilerplate.
///
/// Encapsulates remote parsing, token resolution, HTTP client creation,
/// and standard header injection. Construct via `GitHubClient::new()` which
/// returns `Ok(None)` when no token is available.
pub struct GitHubClient {
    client: Client,
    remote: GitHubRemote,
    token: String,
    api_base: String,
}

impl GitHubClient {
    /// Create a new client for the given remote URL.
    /// Returns `Ok(None)` when no GitHub token is available.
    pub fn new(remote_url: &str, config: &ParsecConfig) -> Result<Option<Self>> {
        let remote = parse_github_remote(remote_url).ok_or_else(|| {
            anyhow::anyhow!("could not parse owner/repo from remote URL: {}", remote_url)
        })?;

        let token = match resolve_github_token(&remote.host, config) {
            Some(t) => t,
            None => return Ok(None),
        };

        // Allow test overrides (and GHE custom endpoints) via env var.
        let api_base = crate::env::github_api_base().unwrap_or_else(|| remote.api_base());
        let client = http_client()?;

        Ok(Some(Self {
            client,
            remote,
            token,
            api_base,
        }))
    }

    /// Access the parsed remote info.
    pub fn remote(&self) -> &GitHubRemote {
        &self.remote
    }

    /// `/repos/{owner}/{repo}` path prefix.
    fn repo_path(&self) -> String {
        format!("/repos/{}/{}", self.remote.owner, self.remote.repo)
    }

    // -- HTTP verb helpers with standard GitHub headers ----------------------

    fn get(&self, path: &str) -> reqwest::RequestBuilder {
        self.client
            .get(format!("{}{}", self.api_base, path))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .bearer_auth(&self.token)
    }

    fn post(&self, path: &str) -> reqwest::RequestBuilder {
        self.client
            .post(format!("{}{}", self.api_base, path))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .bearer_auth(&self.token)
    }

    fn put(&self, path: &str) -> reqwest::RequestBuilder {
        self.client
            .put(format!("{}{}", self.api_base, path))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .bearer_auth(&self.token)
    }

    fn patch(&self, path: &str) -> reqwest::RequestBuilder {
        self.client
            .patch(format!("{}{}", self.api_base, path))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .bearer_auth(&self.token)
    }

    fn delete_req(&self, path: &str) -> reqwest::RequestBuilder {
        self.client
            .delete(format!("{}{}", self.api_base, path))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .bearer_auth(&self.token)
    }

    // -- API methods ---------------------------------------------------------

    /// Fetch the status of a GitHub PR by number.
    pub async fn get_pr_status(&self, pr_number: u64) -> Result<PrStatus> {
        let rp = self.repo_path();

        // Fetch PR details
        let pr: ApiPr = send_with_retry(self.get(&format!("{}/pulls/{}", rp, pr_number)))
            .await?
            .json()
            .await?;

        let title = pr.title;
        let state = if pr.state.is_empty() {
            "unknown".to_string()
        } else {
            pr.state
        };
        let mergeable = pr.mergeable;
        let html_url = pr.html_url;
        let head_sha = pr.head.sha;

        // Fetch combined commit status
        let ci_status = if !head_sha.is_empty() {
            let status_resp: ApiCombinedStatus =
                send_with_retry(self.get(&format!("{}/commits/{}/status", rp, head_sha)))
                    .await?
                    .json()
                    .await?;
            if status_resp.state.is_empty() {
                "unknown".to_string()
            } else {
                status_resp.state
            }
        } else {
            "unknown".to_string()
        };

        // Fetch reviews
        let reviews_resp: Vec<ApiReview> =
            send_with_retry(self.get(&format!("{}/pulls/{}/reviews", rp, pr_number)))
                .await?
                .json()
                .await?;

        let review_status = if reviews_resp.iter().any(|r| r.state == "CHANGES_REQUESTED") {
            "changes_requested".to_string()
        } else if reviews_resp.iter().any(|r| r.state == "APPROVED") {
            "approved".to_string()
        } else if reviews_resp.is_empty() {
            "no reviews".to_string()
        } else {
            "pending".to_string()
        };

        Ok(PrStatus {
            number: pr_number,
            title,
            state,
            mergeable,
            ci_status,
            review_status,
            url: html_url,
        })
    }

    /// Fetch check runs for a PR by number.
    pub async fn get_check_runs(&self, pr_number: u64) -> Result<CiStatus> {
        let rp = self.repo_path();

        // Fetch PR to get head SHA
        let pr_resp: ApiPr = send_with_retry(self.get(&format!("{}/pulls/{}", rp, pr_number)))
            .await?
            .json()
            .await?;

        let head_sha = pr_resp.head.sha;
        if head_sha.is_empty() {
            bail!("could not determine head SHA for PR #{}", pr_number);
        }

        // Fetch check runs for the head SHA
        let checks_resp: ApiCheckRunsResponse =
            send_with_retry(self.get(&format!("{}/commits/{}/check-runs", rp, head_sha)))
                .await?
                .json()
                .await?;

        let checks: Vec<CheckRun> = checks_resp
            .check_runs
            .into_iter()
            .map(|c| CheckRun {
                name: c.name,
                status: c.status,
                conclusion: c.conclusion,
                started_at: c.started_at,
                completed_at: c.completed_at,
                html_url: c.html_url,
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

        Ok(CiStatus {
            pr_number,
            head_sha,
            overall,
            checks,
        })
    }

    /// Fetch check runs directly for a commit SHA (no PR lookup).
    ///
    /// Used by `smartlog` Phase 3 of issue #310 to show CI status for
    /// worktree branches that have no open PR yet.  The result is identical
    /// in shape to [`get_check_runs`]; `pr_number` is set to 0 as a sentinel
    /// (the caller uses the struct fields, not `pr_number`).
    pub async fn get_check_runs_by_sha(&self, sha: &str) -> Result<CiStatus> {
        let rp = self.repo_path();
        let checks_resp: ApiCheckRunsResponse =
            send_with_retry(self.get(&format!("{}/commits/{}/check-runs", rp, sha)))
                .await?
                .json()
                .await?;

        let checks: Vec<CheckRun> = checks_resp
            .check_runs
            .into_iter()
            .map(|c| CheckRun {
                name: c.name,
                status: c.status,
                conclusion: c.conclusion,
                started_at: c.started_at,
                completed_at: c.completed_at,
                html_url: c.html_url,
            })
            .collect();

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

        Ok(CiStatus {
            pr_number: 0,
            head_sha: sha.to_string(),
            overall,
            checks,
        })
    }

    /// Find an open PR by branch name.
    /// Returns the PR number if found.
    pub async fn find_pr_by_branch(&self, branch: &str) -> Result<Option<u64>> {
        let url = format!(
            "{}/pulls?head={}:{}&state=open",
            self.repo_path(),
            self.remote.owner,
            branch
        );
        let resp: Vec<ApiPrListItem> = send_with_retry(self.get(&url)).await?.json().await?;

        Ok(resp.first().and_then(|pr| pr.number))
    }

    /// Return the login of the authenticated GitHub user.
    ///
    /// Used by `parsec reviews --requested` to build the search query.
    pub async fn get_authenticated_user(&self) -> Result<String> {
        let resp: ApiUserResponse = send_with_retry(self.get("/user")).await?.json().await?;
        if resp.login.is_empty() {
            anyhow::bail!("GitHub API returned empty login; is the token valid?");
        }
        Ok(resp.login)
    }

    /// Search for open PRs in *this repo* where `login` is a requested reviewer.
    ///
    /// Uses the GitHub Search Issues API:
    /// `GET /search/issues?q=repo:{owner}/{repo}+type:pr+state:open+review-requested:{login}`
    ///
    /// Returns a list of `(pr_number, title, html_url, state)` tuples.
    /// Up to 30 results (GitHub Search default page size).
    pub async fn search_review_requested_prs(
        &self,
        login: &str,
    ) -> Result<Vec<(u64, String, String, String)>> {
        let q = format!(
            "repo:{}/{} type:pr state:open review-requested:{}",
            self.remote.owner, self.remote.repo, login
        );
        // Use reqwest's .query() so the value is properly percent-encoded.
        let resp: ApiSearchResponse =
            send_with_retry(self.get("/search/issues").query(&[("q", &q)]))
                .await?
                .json()
                .await?;
        Ok(resp
            .items
            .into_iter()
            .map(|item| (item.number, item.title, item.html_url, item.state))
            .collect())
    }

    /// Merge a GitHub PR.
    /// `method` should be "squash", "rebase", or "merge".
    pub async fn merge_pr(
        &self,
        pr_number: u64,
        method: &str,
        delete_branch: bool,
    ) -> Result<MergeResult> {
        let rp = self.repo_path();

        let payload = serde_json::json!({ "merge_method": method });

        let response = self
            .put(&format!("{}/pulls/{}/merge", rp, pr_number))
            .json(&payload)
            .send()
            .await
            .context("Failed to send merge request to GitHub")?;

        if !response.status().is_success() {
            let status_code = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            if status_code == 405 || status_code == 409 {
                bail!("not mergeable: {}", body);
            }
            bail!("GitHub merge API returned {}: {}", status_code, body);
        }

        let resp: ApiMergeResponse = response.json().await?;
        let sha = resp.sha;
        let message = resp.message;

        // Delete remote branch if requested
        if delete_branch {
            let pr_resp: ApiPr = send_with_retry(self.get(&format!("{}/pulls/{}", rp, pr_number)))
                .await?
                .json()
                .await?;

            let branch_name = &pr_resp.head.ref_name;
            if !branch_name.is_empty() {
                let del_url = format!("{}/git/refs/heads/{}", rp, branch_name);
                match self.delete_req(&del_url).send().await {
                    Ok(resp) if !resp.status().is_success() => {
                        let status = resp.status();
                        let body = resp.text().await.unwrap_or_default();
                        eprintln!(
                            "warning: failed to delete remote branch '{}': {} {}",
                            branch_name, status, body
                        );
                    }
                    Err(e) => {
                        eprintln!(
                            "warning: failed to delete remote branch '{}': {}",
                            branch_name, e
                        );
                    }
                    _ => {} // success
                }
            }
        }

        Ok(MergeResult {
            sha,
            message,
            merged: true,
        })
    }

    /// Update a PR branch with the base branch (to make it mergeable).
    /// Returns Ok(true) on success, Ok(false) if conflicts prevent update.
    pub async fn update_pr_branch(&self, pr_number: u64) -> Result<bool> {
        let url = format!("{}/pulls/{}/update-branch", self.repo_path(), pr_number);

        let response = self
            .put(&url)
            .send()
            .await
            .context("Failed to send update-branch request to GitHub")?;

        if response.status().is_success() {
            Ok(true)
        } else if response.status().as_u16() == 422 {
            // 422 = conflicts, cannot auto-update
            Ok(false)
        } else {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            bail!("GitHub update-branch API returned {}: {}", status, body);
        }
    }

    /// Fetch basic info about a GitHub PR by number.
    pub async fn get_pr_info(&self, pr_number: u64) -> Result<Option<PrInfo>> {
        let pr_resp: ApiPr =
            send_with_retry(self.get(&format!("{}/pulls/{}", self.repo_path(), pr_number)))
                .await?
                .json()
                .await?;

        // GitHub returns a JSON object with a "message" field when not found
        if pr_resp.message.is_some() && pr_resp.number == 0 {
            return Ok(None);
        }

        if pr_resp.number == 0 {
            return Ok(None);
        }
        let title = pr_resp.title;
        let head_branch = pr_resp.head.ref_name;

        if head_branch.is_empty() {
            anyhow::bail!("PR #{} response missing head.ref field", pr_number);
        }

        Ok(Some(PrInfo { title, head_branch }))
    }

    /// Create a GitHub Release.
    /// Returns the html_url of the created release.
    pub async fn create_release(&self, tag: &str, name: &str, body: &str) -> Result<String> {
        let payload = serde_json::json!({
            "tag_name": tag,
            "name": name,
            "body": body,
            "draft": false,
            "prerelease": false,
        });

        let response = self
            .post(&format!("{}/releases", self.repo_path()))
            .json(&payload)
            .send()
            .await
            .context("Failed to send release creation request to GitHub")?;

        if !response.status().is_success() {
            let status = response.status();
            let body_text = response.text().await.unwrap_or_default();
            bail!("GitHub API returned {}: {}", status, body_text);
        }

        let resp: ApiCreateResponse = response
            .json()
            .await
            .context("Failed to parse GitHub API response")?;

        resp.html_url
            .ok_or_else(|| anyhow::anyhow!("GitHub response missing html_url"))
    }

    /// Create a GitHub issue.
    pub async fn create_issue(
        &self,
        title: &str,
        body: Option<&str>,
        labels: &[String],
    ) -> Result<IssueResult> {
        let mut payload = serde_json::json!({ "title": title });
        if let Some(b) = body {
            payload["body"] = serde_json::json!(b);
        }
        if !labels.is_empty() {
            payload["labels"] = serde_json::json!(labels);
        }

        let response = self
            .post(&format!("{}/issues", self.repo_path()))
            .json(&payload)
            .send()
            .await
            .context("Failed to send issue creation request to GitHub")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            bail!("GitHub API returned {}: {}", status, body);
        }

        let resp: ApiCreateResponse = response
            .json()
            .await
            .context("Failed to parse GitHub API response")?;

        let html_url = resp
            .html_url
            .ok_or_else(|| anyhow::anyhow!("GitHub response missing html_url"))?;

        let number = resp
            .number
            .ok_or_else(|| anyhow::anyhow!("GitHub response missing number"))?;

        Ok(IssueResult {
            number,
            url: html_url,
        })
    }

    /// Close a GitHub issue by number.
    pub async fn close_issue(&self, issue_number: u64) -> Result<bool> {
        let payload = serde_json::json!({
            "state": "closed",
            "state_reason": "completed",
        });

        let response = self
            .patch(&format!("{}/issues/{}", self.repo_path(), issue_number))
            .json(&payload)
            .send()
            .await
            .context("Failed to send close issue request to GitHub")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            eprintln!(
                "warning: failed to close issue #{}: {} {}",
                issue_number, status, body
            );
            return Ok(false);
        }

        Ok(true)
    }

    /// Create a GitHub pull request.
    pub async fn create_pr(
        &self,
        branch: &str,
        base: &str,
        title: &str,
        body: &str,
        draft: bool,
    ) -> Result<PrResult> {
        let payload = serde_json::json!({
            "title": title,
            "head": branch,
            "base": base,
            "body": body,
            "draft": draft,
        });

        let response = self
            .post(&format!("{}/pulls", self.repo_path()))
            .json(&payload)
            .send()
            .await
            .context("Failed to send PR creation request to GitHub")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            bail!("GitHub API returned {}: {}", status, body);
        }

        let resp: ApiCreateResponse = response
            .json()
            .await
            .context("Failed to parse GitHub API response")?;

        let html_url = resp
            .html_url
            .ok_or_else(|| anyhow::anyhow!("GitHub response missing html_url"))?;

        let number = resp.number.unwrap_or(0);

        Ok(PrResult {
            url: html_url,
            number,
        })
    }

    /// Request reviews from GitHub users on a PR.
    pub async fn request_reviewers(&self, pr_number: u64, reviewers: &[String]) -> Result<()> {
        if reviewers.is_empty() {
            return Ok(());
        }
        let payload = serde_json::json!({ "reviewers": reviewers });
        let response = self
            .post(&format!(
                "{}/pulls/{}/requested_reviewers",
                self.repo_path(),
                pr_number
            ))
            .json(&payload)
            .send()
            .await
            .context("Failed to request reviewers")?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            bail!("Failed to request reviewers: {} {}", status, body);
        }
        Ok(())
    }

    /// Add labels to a PR/issue.
    pub async fn add_labels(&self, issue_number: u64, labels: &[String]) -> Result<()> {
        if labels.is_empty() {
            return Ok(());
        }
        let payload = serde_json::json!({ "labels": labels });
        let response = self
            .post(&format!(
                "{}/issues/{}/labels",
                self.repo_path(),
                issue_number
            ))
            .json(&payload)
            .send()
            .await
            .context("Failed to add labels")?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            bail!("Failed to add labels: {} {}", status, body);
        }
        Ok(())
    }
}
