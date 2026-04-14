use anyhow::{bail, Context, Result};
use chrono::Utc;
use std::path::{Path, PathBuf};

use crate::config::ParsecConfig;
use crate::git;
use super::lifecycle::{ParsecState, ShipResult, Workspace, WorkspaceStatus};

// ---------------------------------------------------------------------------
// WorktreeManager
// ---------------------------------------------------------------------------

pub struct WorktreeManager {
    repo_root: PathBuf,
    config: ParsecConfig,
}

impl WorktreeManager {
    /// Construct a new manager rooted at the repository containing `repo`.
    pub fn new(repo: &Path, config: &ParsecConfig) -> Result<Self> {
        let repo_root = git::get_repo_root(repo)
            .with_context(|| format!("failed to locate git repository root from {:?}", repo))?;

        Ok(Self {
            repo_root,
            config: config.clone(),
        })
    }

    // -----------------------------------------------------------------------
    // create
    // -----------------------------------------------------------------------

    /// Create a new worktree for `ticket` branching off `base` (or the
    /// detected default branch when `base` is `None`).
    pub fn create(&self, ticket: &str, base: Option<&str>) -> Result<Workspace> {
        let base_branch = match base {
            Some(b) => b.to_owned(),
            None => git::get_default_branch(&self.repo_root)
                .context("failed to detect default branch")?,
        };

        let branch = format!(
            "{}{}",
            self.config.workspace.branch_prefix, ticket
        );
        let worktree_path = self
            .repo_root
            .join(&self.config.workspace.base_dir)
            .join(ticket);

        git::fetch(&self.repo_root).context("failed to fetch from origin")?;

        git::worktree_add(&self.repo_root, &worktree_path, &branch, &base_branch)
            .with_context(|| {
                format!(
                    "failed to create worktree for ticket '{}' at {:?}",
                    ticket, worktree_path
                )
            })?;

        let workspace = Workspace {
            ticket: ticket.to_owned(),
            path: worktree_path,
            branch,
            base_branch,
            created_at: Utc::now(),
            ticket_title: None,
            status: WorkspaceStatus::Active,
        };

        let mut state = ParsecState::load(&self.repo_root)
            .context("failed to load parsec state")?;
        state.add_workspace(workspace.clone());
        state.save(&self.repo_root)
            .context("failed to save parsec state")?;

        Ok(workspace)
    }

    // -----------------------------------------------------------------------
    // list
    // -----------------------------------------------------------------------

    /// Return all tracked workspaces sorted by creation time (oldest first).
    pub fn list(&self) -> Result<Vec<Workspace>> {
        let state = ParsecState::load(&self.repo_root)
            .context("failed to load parsec state")?;

        let mut workspaces: Vec<Workspace> = state.workspaces.into_values().collect();
        workspaces.sort_by_key(|w| w.created_at);
        Ok(workspaces)
    }

    // -----------------------------------------------------------------------
    // get
    // -----------------------------------------------------------------------

    /// Retrieve a single workspace by ticket, returning an error if not found.
    pub fn get(&self, ticket: &str) -> Result<Workspace> {
        let state = ParsecState::load(&self.repo_root)
            .context("failed to load parsec state")?;

        state
            .get_workspace(ticket)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("no workspace found for ticket '{}'", ticket))
    }

    // -----------------------------------------------------------------------
    // ship
    // -----------------------------------------------------------------------

    /// Push the branch, optionally open a GitHub PR, and optionally clean up
    /// the worktree.
    pub fn ship(&self, ticket: &str, draft: bool, no_pr: bool) -> Result<ShipResult> {
        let mut state = ParsecState::load(&self.repo_root)
            .context("failed to load parsec state")?;

        let workspace = state
            .get_workspace(ticket)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("no workspace found for ticket '{}'", ticket))?;

        // Push the branch from the worktree itself so HEAD is correct.
        git::push_branch(&workspace.path, &workspace.branch)
            .with_context(|| format!("failed to push branch '{}'", workspace.branch))?;

        // Optionally create a GitHub PR.
        let pr_url = if !no_pr && self.config.ship.auto_pr {
            let title = workspace
                .ticket_title
                .clone()
                .unwrap_or_else(|| workspace.ticket.clone());

            create_github_pr(
                &self.repo_root,
                &workspace.branch,
                &workspace.base_branch,
                &title,
                draft || self.config.ship.draft,
            )
            .unwrap_or_else(|e| {
                eprintln!("warning: PR creation skipped: {e}");
                None
            })
        } else {
            None
        };

        // Optionally clean up the worktree and local branch.
        let cleaned_up = if self.config.ship.auto_cleanup {
            match git::worktree_remove(&self.repo_root, &workspace.path) {
                Ok(()) => {
                    // Best-effort branch deletion; ignore errors if already gone.
                    let _ = git::delete_branch(&self.repo_root, &workspace.branch);
                    true
                }
                Err(e) => {
                    eprintln!("warning: failed to remove worktree: {e}");
                    false
                }
            }
        } else {
            false
        };

        // Update persisted state.
        if cleaned_up {
            state.remove_workspace(ticket);
        } else {
            if let Some(ws) = state.workspaces.get_mut(ticket) {
                ws.status = WorkspaceStatus::Shipped;
            }
        }
        state.save(&self.repo_root)
            .context("failed to save parsec state after ship")?;

        Ok(ShipResult {
            ticket: ticket.to_owned(),
            branch: workspace.branch,
            pr_url,
            cleaned_up,
        })
    }

    // -----------------------------------------------------------------------
    // clean
    // -----------------------------------------------------------------------

    /// Remove merged (or all) workspaces.
    ///
    /// When `dry_run` is `true` the list of workspaces that *would* be removed
    /// is returned but no changes are made.
    pub fn clean(&self, all: bool, dry_run: bool) -> Result<Vec<Workspace>> {
        let mut state = ParsecState::load(&self.repo_root)
            .context("failed to load parsec state")?;

        let candidates: Vec<Workspace> = state
            .workspaces
            .values()
            .filter(|ws| {
                if all {
                    return true;
                }
                // Include workspace if the branch is merged into its base.
                git::is_branch_merged(&self.repo_root, &ws.branch, &ws.base_branch)
                    .unwrap_or(false)
            })
            .cloned()
            .collect();

        if !dry_run {
            for ws in &candidates {
                match git::worktree_remove(&self.repo_root, &ws.path) {
                    Ok(()) => {
                        let _ = git::delete_branch(&self.repo_root, &ws.branch);
                    }
                    Err(e) => {
                        eprintln!(
                            "warning: failed to remove worktree for '{}': {e}",
                            ws.ticket
                        );
                    }
                }
                state.remove_workspace(&ws.ticket);
            }

            state.save(&self.repo_root)
                .context("failed to save parsec state after clean")?;
        }

        Ok(candidates)
    }
}

// ---------------------------------------------------------------------------
// GitHub PR creation (synchronous, shells out to curl)
// ---------------------------------------------------------------------------

/// Parse `git@github.com:owner/repo.git` or `https://github.com/owner/repo.git`
/// into `(owner, repo)`.
fn parse_github_remote(url: &str) -> Option<(String, String)> {
    // SSH form: git@github.com:owner/repo.git
    if let Some(rest) = url.strip_prefix("git@github.com:") {
        let rest = rest.trim_end_matches(".git");
        let mut parts = rest.splitn(2, '/');
        let owner = parts.next()?.to_owned();
        let repo = parts.next()?.to_owned();
        return Some((owner, repo));
    }

    // HTTPS form: https://github.com/owner/repo.git  (or .git-less)
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

/// Create a GitHub pull request by shelling out to `curl`.
///
/// Returns `None` silently when `PARSEC_GITHUB_TOKEN` is not set, so callers
/// can treat "no token" as a non-fatal condition.
fn create_github_pr(
    repo_root: &Path,
    branch: &str,
    base: &str,
    title: &str,
    draft: bool,
) -> Result<Option<String>> {
    let token = match std::env::var("PARSEC_GITHUB_TOKEN") {
        Ok(t) if !t.is_empty() => t,
        _ => return Ok(None), // No token — skip silently.
    };

    let remote_url = git::get_remote_url(repo_root)
        .context("failed to get origin remote URL")?;

    let (owner, repo) = parse_github_remote(&remote_url).ok_or_else(|| {
        anyhow::anyhow!(
            "could not parse owner/repo from remote URL: {}",
            remote_url
        )
    })?;

    let api_url = format!(
        "https://api.github.com/repos/{}/{}/pulls",
        owner, repo
    );

    // Build a minimal JSON payload.
    let body = serde_json::json!({
        "title": title,
        "head":  branch,
        "base":  base,
        "draft": draft,
    });
    let body_str = serde_json::to_string(&body).context("failed to serialize PR payload")?;

    let output = std::process::Command::new("curl")
        .args([
            "--silent",
            "--fail",
            "--location",
            "--request",
            "POST",
            "--header",
            "Accept: application/vnd.github+json",
            "--header",
            &format!("Authorization: Bearer {}", token),
            "--header",
            "X-GitHub-Api-Version: 2022-11-28",
            "--header",
            "Content-Type: application/json",
            "--data",
            &body_str,
            &api_url,
        ])
        .output()
        .context("failed to spawn curl for GitHub PR creation")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("curl exited with status {}: {}", output.status, stderr.trim());
    }

    let response_text = String::from_utf8(output.stdout)
        .context("GitHub API response contained non-UTF-8 bytes")?;

    let response: serde_json::Value =
        serde_json::from_str(&response_text).context("failed to parse GitHub API response")?;

    let html_url = response["html_url"]
        .as_str()
        .map(|s| s.to_owned());

    Ok(html_url)
}
