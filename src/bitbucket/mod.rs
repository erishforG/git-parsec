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
#[allow(dead_code)]
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

/// A PR participant (reviewer/commenter) used for review_status mapping.
#[derive(Debug, Clone)]
pub struct Participant {
    /// Bitbucket review state: "approved", "changes_requested", or None.
    pub state: Option<String>,
    /// Convenience boolean flag from the Bitbucket API.
    pub approved: bool,
    /// Role: "REVIEWER" or "PARTICIPANT".
    pub role: Option<String>,
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
    #[serde(default)]
    source: Option<ApiPrEndpoint>,
    #[serde(default)]
    participants: Option<Vec<ApiParticipant>>,
}

#[derive(Deserialize)]
struct ApiPrEndpoint {
    branch: Option<ApiBranch>,
}

#[derive(Deserialize)]
struct ApiBranch {
    name: Option<String>,
}

#[derive(Deserialize)]
struct ApiParticipant {
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    approved: Option<bool>,
    #[serde(default)]
    role: Option<String>,
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
#[allow(dead_code)]
struct ApiPipeline {
    uuid: Option<String>,
    state: Option<ApiPipelineState>,
    target: Option<ApiPipelineTarget>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct ApiPipelineState {
    name: Option<String>,
    result: Option<ApiPipelineResult>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct ApiPipelineResult {
    name: Option<String>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct ApiPipelineTarget {
    ref_name: Option<String>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
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

/// Default Bitbucket Cloud API base URL (without trailing slash).
const DEFAULT_API_BASE: &str = "https://api.bitbucket.org/2.0";

/// Authenticated Bitbucket Cloud API client.
pub struct BitbucketClient {
    client: Client,
    remote: BitbucketRemote,
    token: String,
    api_base: String,
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

        let api_base =
            crate::env::bitbucket_api_base().unwrap_or_else(|| DEFAULT_API_BASE.to_string());

        Ok(Some(Self {
            client,
            remote,
            token,
            api_base,
        }))
    }

    /// Access the parsed remote info.
    pub fn remote(&self) -> &BitbucketRemote {
        &self.remote
    }

    /// Repo API path prefix.
    fn repo_url(&self) -> String {
        format!(
            "{}/repositories/{}/{}",
            self.api_base, self.remote.workspace, self.remote.repo_slug
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
        Ok(list.values.and_then(|v| v.first().and_then(|pr| pr.id)))
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

    /// Get the latest pipeline (most recently created) for a branch, if any.
    pub async fn get_latest_pipeline_for_branch(
        &self,
        branch: &str,
    ) -> Result<Option<PipelineStatus>> {
        Ok(self.get_pipelines(branch).await?.into_iter().next())
    }

    /// Fetch the source branch name for a PR.
    pub async fn get_pr_source_branch(&self, pr_id: u64) -> Result<Option<String>> {
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
        Ok(pr
            .source
            .and_then(|s| s.branch)
            .and_then(|b| b.name)
            .filter(|n| !n.is_empty()))
    }

    /// Fetch participants for a PR. Returns an empty vec when the PR has no participants
    /// or the API call fails (callers may interpret this as "unknown / pending").
    pub async fn get_pr_participants(&self, pr_id: u64) -> Result<Vec<Participant>> {
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
        Ok(pr
            .participants
            .unwrap_or_default()
            .into_iter()
            .map(|p| Participant {
                state: p.state,
                approved: p.approved.unwrap_or(false),
                role: p.role,
            })
            .collect())
    }
}

// ---------------------------------------------------------------------------
// Pure mapping functions (forge-agnostic vocabulary)
// ---------------------------------------------------------------------------

/// Map a Bitbucket pipeline (state + optional result) to the same `ci_status`
/// vocabulary that the GitHub path emits: `passing` | `failing` | `pending`
/// | `no checks` | `unknown`.
///
/// Bitbucket pipeline `state.name` values: `PENDING`, `IN_PROGRESS`,
/// `COMPLETED`, `HALTED`, `STOPPED`. When `state.name == "COMPLETED"`,
/// `state.result.name` is one of `SUCCESSFUL`, `FAILED`, `ERROR`,
/// `STOPPED`, `EXPIRED`.
pub fn pipeline_to_ci_status(state: &str, result: Option<&str>) -> String {
    match state.to_ascii_uppercase().as_str() {
        "COMPLETED" => match result.map(|r| r.to_ascii_uppercase()).as_deref() {
            Some("SUCCESSFUL") => "passing".to_string(),
            Some("FAILED") | Some("ERROR") | Some("STOPPED") | Some("EXPIRED") => {
                "failing".to_string()
            }
            _ => "unknown".to_string(),
        },
        "PENDING" | "IN_PROGRESS" | "HALTED" => "pending".to_string(),
        _ => "unknown".to_string(),
    }
}

/// Convenience wrapper: map an optional `PipelineStatus` to a `ci_status` string.
/// `None` → `"no checks"` (consistent with GitHub's empty-checks rendering).
pub fn pipeline_status_to_ci_string(p: Option<&PipelineStatus>) -> String {
    match p {
        Some(p) => pipeline_to_ci_status(&p.state, p.result.as_deref()),
        None => "no checks".to_string(),
    }
}

/// Map Bitbucket PR participants to the same `review_status` vocabulary the
/// GitHub path emits: `approved` | `changes_requested` | `pending` | `no reviews`.
///
/// - Any participant with `state == "changes_requested"` → `changes_requested`.
/// - Else any participant with `approved == true` (or `state == "approved"`) → `approved`.
/// - Else if there are any reviewer-role participants → `pending`.
/// - Else → `no reviews`.
pub fn participants_to_review_status(participants: &[Participant]) -> String {
    if participants
        .iter()
        .any(|p| p.state.as_deref() == Some("changes_requested"))
    {
        return "changes_requested".to_string();
    }
    if participants
        .iter()
        .any(|p| p.approved || p.state.as_deref() == Some("approved"))
    {
        return "approved".to_string();
    }
    if participants
        .iter()
        .any(|p| p.role.as_deref() == Some("REVIEWER"))
    {
        return "pending".to_string();
    }
    "no reviews".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- pipeline_to_ci_status -------------------------------------------------

    #[test]
    fn pipeline_completed_successful_is_passing() {
        assert_eq!(
            pipeline_to_ci_status("COMPLETED", Some("SUCCESSFUL")),
            "passing"
        );
    }

    #[test]
    fn pipeline_completed_failed_is_failing() {
        assert_eq!(
            pipeline_to_ci_status("COMPLETED", Some("FAILED")),
            "failing"
        );
    }

    #[test]
    fn pipeline_completed_error_is_failing() {
        assert_eq!(pipeline_to_ci_status("COMPLETED", Some("ERROR")), "failing");
    }

    #[test]
    fn pipeline_completed_expired_is_failing() {
        assert_eq!(
            pipeline_to_ci_status("COMPLETED", Some("EXPIRED")),
            "failing"
        );
    }

    #[test]
    fn pipeline_in_progress_is_pending() {
        assert_eq!(pipeline_to_ci_status("IN_PROGRESS", None), "pending");
    }

    #[test]
    fn pipeline_pending_is_pending() {
        assert_eq!(pipeline_to_ci_status("PENDING", None), "pending");
    }

    #[test]
    fn pipeline_halted_is_pending() {
        assert_eq!(pipeline_to_ci_status("HALTED", None), "pending");
    }

    #[test]
    fn pipeline_unknown_state_is_unknown() {
        assert_eq!(pipeline_to_ci_status("WAT", None), "unknown");
    }

    #[test]
    fn pipeline_state_is_case_insensitive() {
        assert_eq!(
            pipeline_to_ci_status("completed", Some("successful")),
            "passing"
        );
    }

    #[test]
    fn pipeline_status_to_ci_string_none_is_no_checks() {
        assert_eq!(pipeline_status_to_ci_string(None), "no checks");
    }

    #[test]
    fn pipeline_status_to_ci_string_passes_through() {
        let p = PipelineStatus {
            name: "x".into(),
            state: "COMPLETED".into(),
            result: Some("SUCCESSFUL".into()),
            url: None,
        };
        assert_eq!(pipeline_status_to_ci_string(Some(&p)), "passing");
    }

    // -- participants_to_review_status -----------------------------------------

    fn p(state: Option<&str>, approved: bool, role: Option<&str>) -> Participant {
        Participant {
            state: state.map(|s| s.to_string()),
            approved,
            role: role.map(|s| s.to_string()),
        }
    }

    #[test]
    fn empty_participants_is_no_reviews() {
        assert_eq!(participants_to_review_status(&[]), "no reviews");
    }

    #[test]
    fn changes_requested_dominates() {
        let parts = vec![
            p(Some("approved"), true, Some("REVIEWER")),
            p(Some("changes_requested"), false, Some("REVIEWER")),
        ];
        assert_eq!(participants_to_review_status(&parts), "changes_requested");
    }

    #[test]
    fn approved_state() {
        let parts = vec![p(Some("approved"), true, Some("REVIEWER"))];
        assert_eq!(participants_to_review_status(&parts), "approved");
    }

    #[test]
    fn approved_via_boolean_only() {
        let parts = vec![p(None, true, Some("REVIEWER"))];
        assert_eq!(participants_to_review_status(&parts), "approved");
    }

    #[test]
    fn reviewer_no_action_is_pending() {
        let parts = vec![p(None, false, Some("REVIEWER"))];
        assert_eq!(participants_to_review_status(&parts), "pending");
    }

    #[test]
    fn only_non_reviewer_participants_is_no_reviews() {
        // PR author / commenter shows up as PARTICIPANT with no review state.
        let parts = vec![p(None, false, Some("PARTICIPANT"))];
        assert_eq!(participants_to_review_status(&parts), "no reviews");
    }

    // -- remote URL parsing (existing behavior, sanity-check) ------------------

    #[test]
    fn parse_https_remote() {
        let r = parse_bitbucket_remote("https://bitbucket.org/myws/myrepo.git").unwrap();
        assert_eq!(r.workspace, "myws");
        assert_eq!(r.repo_slug, "myrepo");
    }

    #[test]
    fn parse_ssh_remote() {
        let r = parse_bitbucket_remote("git@bitbucket.org:myws/myrepo.git").unwrap();
        assert_eq!(r.workspace, "myws");
        assert_eq!(r.repo_slug, "myrepo");
    }

    #[test]
    fn is_bitbucket_remote_detects_url() {
        assert!(is_bitbucket_remote("git@bitbucket.org:foo/bar.git"));
        assert!(!is_bitbucket_remote("git@github.com:foo/bar.git"));
    }
}
