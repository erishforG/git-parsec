//! Bitbucket Cloud REST API v2 integration.
//!
//! Provides PR creation, status, merge, CI pipeline monitoring,
//! and branch-based PR lookup for Bitbucket Cloud repositories.

use std::time::Duration;

use anyhow::{bail, Context, Result};
use reqwest::Client;
use serde::Deserialize;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// Result of PR creation
#[derive(Debug, Clone)]
pub struct PrResult {
    pub url: String,
    pub id: u64,
}

/// PR status information
#[derive(Debug, Clone)]
pub struct PrStatus {
    pub id: u64,
    pub title: String,
    pub state: String,
    pub url: String,
}

/// Result of merging a PR
#[derive(Debug, Clone)]
pub struct MergeResult {
    pub merged: bool,
    pub message: String,
}

/// A single pipeline step/result
#[derive(Debug, Clone)]
pub struct PipelineStatus {
    pub name: String,
    pub state: String,
    pub result: Option<String>,
    pub url: Option<String>,
}

/// Parsed Bitbucket remote info
#[derive(Debug, Clone)]
pub struct BitbucketRemote {
    pub workspace: String,
    pub repo_slug: String,
}

// ---------------------------------------------------------------------------
// API response types (private)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct ApiPr {
    id: Option<u64>,
    title: Option<String>,
    state: Option<String>,
    links: Option<ApiLinks>,
}

#[derive(Deserialize)]
struct ApiLinks {
    html: Option<ApiHref>,
}

#[derive(Deserialize)]
struct ApiHref {
    href: Option<String>,
}

#[derive(Deserialize)]
struct ApiPrList {
    values: Option<Vec<ApiPr>>,
}

#[derive(Deserialize)]
struct ApiPipeline {
    uuid: Option<String>,
    state: Option<ApiPipelineState>,
    target: Option<ApiPipelineTarget>,
}

#[derive(Deserialize)]
struct ApiPipelineState {
    name: Option<String>,
    result: Option<ApiPipelineResult>,
}

#[derive(Deserialize)]
struct ApiPipelineResult {
    name: Option<String>,
}

#[derive(Deserialize)]
struct ApiPipelineTarget {
    ref_name: Option<String>,
}

#[derive(Deserialize)]
struct ApiPipelineList {
    values: Option<Vec<ApiPipeline>>,
}

// ---------------------------------------------------------------------------
// Remote URL parsing
// ---------------------------------------------------------------------------

/// Parse a Bitbucket Cloud remote URL into BitbucketRemote.
/// Supports SSH and HTTPS forms for bitbucket.org.
pub fn parse_bitbucket_remote(url: &str) -> Option<BitbucketRemote> {
    // SSH: git@bitbucket.org:workspace/repo.git
    if url.starts_with("git@bitbucket.org:") {
        let path = url.strip_prefix("git@bitbucket.org:")?;
        let path = path.trim_end_matches(".git");
        let mut parts = path.splitn(2, '/');
        let workspace = parts.next()?.to_owned();
        let repo_slug = parts.next()?.to_owned();
        return Some(BitbucketRemote {
            workspace,
            repo_slug,
        });
    }

    // HTTPS: https://bitbucket.org/workspace/repo.git
    let rest = url
        .strip_prefix("https://bitbucket.org/")
        .or_else(|| url.strip_prefix("http://bitbucket.org/"))?;
    let path = rest.trim_end_matches(".git");
    let mut parts = path.splitn(2, '/');
    let workspace = parts.next()?.to_owned();
    let repo_slug = parts.next()?.to_owned();
    Some(BitbucketRemote {
        workspace,
        repo_slug,
    })
}

/// Check if a remote URL is a Bitbucket Cloud URL.
pub fn is_bitbucket_remote(url: &str) -> bool {
    url.contains("bitbucket.org")
}

// ---------------------------------------------------------------------------
// BitbucketClient
// ---------------------------------------------------------------------------

/// Authenticated Bitbucket Cloud API client.
pub struct BitbucketClient {
    client: Client,
    remote: BitbucketRemote,
    token: String,
}

impl BitbucketClient {
    /// Create a new client for the given remote URL.
    /// Returns `Ok(None)` when no Bitbucket token is available or URL is not Bitbucket.
    pub fn new(remote_url: &str) -> Result<Option<Self>> {
        if !is_bitbucket_remote(remote_url) {
            return Ok(None);
        }

        let remote = parse_bitbucket_remote(remote_url).ok_or_else(|| {
            anyhow::anyhow!(
                "could not parse workspace/repo from Bitbucket remote URL: {}",
                remote_url
            )
        })?;

        let token = match crate::env::bitbucket_token() {
            Some(t) => t,
            None => return Ok(None),
        };

        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .user_agent("git-parsec")
            .build()
            .context("failed to build HTTP client")?;

        Ok(Some(Self {
            client,
            remote,
            token,
        }))
    }

    /// Access the parsed remote info.
    pub fn remote(&self) -> &BitbucketRemote {
        &self.remote
    }

    /// Repo API path prefix.
    fn repo_url(&self) -> String {
        format!(
            "https://api.bitbucket.org/2.0/repositories/{}/{}",
            self.remote.workspace, self.remote.repo_slug
        )
    }

    fn auth_get(&self, url: &str) -> reqwest::RequestBuilder {
        self.client
            .get(url)
            .bearer_auth(&self.token)
            .header("Accept", "application/json")
    }

    fn auth_post(&self, url: &str) -> reqwest::RequestBuilder {
        self.client
            .post(url)
            .bearer_auth(&self.token)
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
    }

    // -- API methods ---------------------------------------------------------

    /// Create a pull request.
    pub async fn create_pr(
        &self,
        branch: &str,
        base: &str,
        title: &str,
        description: &str,
        _draft: bool, // Bitbucket Cloud doesn't support draft PRs natively
    ) -> Result<PrResult> {
        let url = format!("{}/pullrequests", self.repo_url());

        let payload = serde_json::json!({
            "title": title,
            "description": description,
            "source": {
                "branch": { "name": branch }
            },
            "destination": {
                "branch": { "name": base }
            },
            "close_source_branch": true
        });

        let response = self
            .auth_post(&url)
            .json(&payload)
            .send()
            .await
            .context("Failed to send PR creation request to Bitbucket")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            bail!("Bitbucket API returned {}: {}", status, body);
        }

        let pr: ApiPr = response
            .json()
            .await
            .context("Failed to parse Bitbucket API response")?;

        let id = pr.id.unwrap_or(0);
        let html_url = pr
            .links
            .and_then(|l| l.html)
            .and_then(|h| h.href)
            .unwrap_or_else(|| {
                format!(
                    "https://bitbucket.org/{}/{}/pull-requests/{}",
                    self.remote.workspace, self.remote.repo_slug, id
                )
            });

        Ok(PrResult { url: html_url, id })
    }

    /// Find an open PR by source branch name.
    pub async fn find_pr_by_branch(&self, branch: &str) -> Result<Option<u64>> {
        let url = format!(
            "{}/pullrequests?q=source.branch.name=\"{}\" AND state=\"OPEN\"",
            self.repo_url(),
            branch
        );

        let response = self
            .auth_get(&url)
            .send()
            .await
            .context("Failed to query Bitbucket PRs")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            bail!("Bitbucket API returned {}: {}", status, body);
        }

        let list: ApiPrList = response.json().await?;
        Ok(list
            .values
            .and_then(|v| v.first().and_then(|pr| pr.id)))
    }

    /// Get PR status by ID.
    pub async fn get_pr_status(&self, pr_id: u64) -> Result<PrStatus> {
        let url = format!("{}/pullrequests/{}", self.repo_url(), pr_id);

        let response = self
            .auth_get(&url)
            .send()
            .await
            .context("Failed to fetch Bitbucket PR")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            bail!("Bitbucket API returned {}: {}", status, body);
        }

        let pr: ApiPr = response.json().await?;
        let id = pr.id.unwrap_or(pr_id);
        let html_url = pr
            .links
            .and_then(|l| l.html)
            .and_then(|h| h.href)
            .unwrap_or_default();

        Ok(PrStatus {
            id,
            title: pr.title.unwrap_or_default(),
            state: pr.state.unwrap_or_else(|| "unknown".to_string()),
            url: html_url,
        })
    }

    /// Merge a PR.
    pub async fn merge_pr(&self, pr_id: u64, strategy: &str) -> Result<MergeResult> {
        let url = format!("{}/pullrequests/{}/merge", self.repo_url(), pr_id);

        // Bitbucket merge strategies: merge_commit, squash, fast_forward
        let bb_strategy = match strategy {
            "squash" => "squash",
            "rebase" => "fast_forward",
            _ => "merge_commit",
        };

        let payload = serde_json::json!({
            "merge_strategy": bb_strategy,
            "close_source_branch": true
        });

        let response = self
            .auth_post(&url)
            .json(&payload)
            .send()
            .await
            .context("Failed to merge Bitbucket PR")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            bail!("Bitbucket merge failed ({}): {}", status, body);
        }

        Ok(MergeResult {
            merged: true,
            message: format!("PR #{} merged via {}", pr_id, bb_strategy),
        })
    }

    /// Get pipeline status for a branch.
    pub async fn get_pipelines(&self, branch: &str) -> Result<Vec<PipelineStatus>> {
        let url = format!(
            "{}/pipelines/?sort=-created_on&pagelen=5&target.ref_name={}",
            self.repo_url(),
            branch
        );

        let response = self
            .auth_get(&url)
            .send()
            .await
            .context("Failed to fetch Bitbucket pipelines")?;

        if !response.status().is_success() {
            // Pipelines may not be enabled — return empty
            return Ok(Vec::new());
        }

        let list: ApiPipelineList = response.json().await?;
        let pipelines = list
            .values
            .unwrap_or_default()
            .into_iter()
            .map(|p| {
                let state_name = p
                    .state
                    .as_ref()
                    .and_then(|s| s.name.clone())
                    .unwrap_or_else(|| "unknown".to_string());
                let result_name = p
                    .state
                    .as_ref()
                    .and_then(|s| s.result.as_ref())
                    .and_then(|r| r.name.clone());
                let uuid = p.uuid.unwrap_or_default();
                let ref_name = p
                    .target
                    .and_then(|t| t.ref_name)
                    .unwrap_or_else(|| branch.to_string());
                let pipeline_url = format!(
                    "https://bitbucket.org/{}/{}/pipelines/results/{}",
                    self.remote.workspace,
                    self.remote.repo_slug,
                    uuid.trim_matches(|c| c == '{' || c == '}')
                );
                PipelineStatus {
                    name: format!("pipeline ({})", ref_name),
                    state: state_name,
                    result: result_name,
                    url: Some(pipeline_url),
                }
            })
            .collect();

        Ok(pipelines)
    }
}
