use anyhow::{Context, Result};
use chrono::Utc;
use std::path::{Path, PathBuf};

use super::lifecycle::{ParsecState, ShipResult, Workspace, WorkspaceStatus};
use crate::config::ParsecConfig;
use crate::git;

// ---------------------------------------------------------------------------
// WorktreeManager
// ---------------------------------------------------------------------------

pub struct WorktreeManager {
    repo_root: PathBuf,
    config: ParsecConfig,
}

impl WorktreeManager {
    pub fn new(repo: &Path, config: &ParsecConfig) -> Result<Self> {
        let repo_root = git::get_repo_root(repo)
            .with_context(|| format!("failed to locate git repository root from {:?}", repo))?;

        Ok(Self {
            repo_root,
            config: config.clone(),
        })
    }

    pub fn repo_root(&self) -> &Path {
        &self.repo_root
    }

    // -----------------------------------------------------------------------
    // create
    // -----------------------------------------------------------------------

    pub fn create(
        &self,
        ticket: &str,
        base: Option<&str>,
        ticket_title: Option<String>,
    ) -> Result<Workspace> {
        let base_branch = match base {
            Some(b) => b.to_owned(),
            None => git::get_default_branch(&self.repo_root)
                .context("failed to detect default branch")?,
        };

        let branch = format!("{}{}", self.config.workspace.branch_prefix, ticket);
        let worktree_path = match self.config.workspace.layout {
            crate::config::WorktreeLayout::Sibling => {
                // ../reponame.ticket/
                let repo_name = self
                    .repo_root
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "repo".to_string());
                self.repo_root
                    .parent()
                    .unwrap_or(&self.repo_root)
                    .join(format!("{}.{}", repo_name, ticket))
            }
            crate::config::WorktreeLayout::Internal => self
                .repo_root
                .join(&self.config.workspace.base_dir)
                .join(ticket),
        };

        // Graceful fetch — won't fail if no remote exists
        git::fetch_if_remote(&self.repo_root)?;

        git::worktree_add(&self.repo_root, &worktree_path, &branch, &base_branch).with_context(
            || {
                format!(
                    "failed to create worktree for ticket '{}' at {:?}",
                    ticket, worktree_path
                )
            },
        )?;

        let workspace = Workspace {
            ticket: ticket.to_owned(),
            path: worktree_path.clone(),
            branch,
            base_branch,
            created_at: Utc::now(),
            ticket_title,
            status: WorkspaceStatus::Active,
        };

        let mut state =
            ParsecState::load(&self.repo_root).context("failed to load parsec state")?;
        state.add_workspace(workspace.clone());
        state
            .save(&self.repo_root)
            .context("failed to save parsec state")?;

        // Run post-create hooks
        for hook_cmd in &self.config.hooks.post_create {
            eprintln!("Running post-create hook: {}", hook_cmd);
            let status = std::process::Command::new("sh")
                .args(["-c", hook_cmd])
                .current_dir(&worktree_path)
                .status();
            match status {
                Ok(s) if s.success() => {}
                Ok(s) => eprintln!("warning: hook '{}' exited with {}", hook_cmd, s),
                Err(e) => eprintln!("warning: failed to run hook '{}': {}", hook_cmd, e),
            }
        }

        Ok(workspace)
    }

    // -----------------------------------------------------------------------
    // adopt — import an existing branch into parsec state
    // -----------------------------------------------------------------------

    pub fn adopt(
        &self,
        ticket: &str,
        branch: Option<&str>,
        ticket_title: Option<String>,
    ) -> Result<Workspace> {
        let mut state =
            ParsecState::load(&self.repo_root).context("failed to load parsec state")?;

        // Check if ticket is already managed
        if state.get_workspace(ticket).is_some() {
            anyhow::bail!(
                "ticket '{}' is already managed by parsec. Use `parsec status {}` to see it.",
                ticket,
                ticket
            );
        }

        // Resolve the branch name
        let branch_name = match branch {
            Some(b) => b.to_owned(),
            None => {
                // Default: branch_prefix + ticket
                let candidate = format!("{}{}", self.config.workspace.branch_prefix, ticket);
                // Verify the branch exists
                match git::run_output(
                    &self.repo_root,
                    &[
                        "rev-parse",
                        "--verify",
                        &format!("refs/heads/{}", candidate),
                    ],
                ) {
                    Ok(_) => candidate,
                    Err(_) => {
                        // Try current branch
                        let current = git::run_output(
                            &self.repo_root,
                            &["rev-parse", "--abbrev-ref", "HEAD"],
                        )
                        .context("could not detect branch. Specify one with --branch <name>")?;
                        if current == "HEAD" || current == "main" || current == "master" {
                            anyhow::bail!(
                                "no branch found for ticket '{}'. Specify one with: parsec adopt {} --branch <branch-name>",
                                ticket, ticket
                            );
                        }
                        current
                    }
                }
            }
        };

        // Verify branch exists
        git::run_output(
            &self.repo_root,
            &[
                "rev-parse",
                "--verify",
                &format!("refs/heads/{}", branch_name),
            ],
        )
        .with_context(|| {
            format!(
                "branch '{}' does not exist. Create it first or check the name.",
                branch_name
            )
        })?;

        // Detect base branch
        let base_branch =
            git::get_default_branch(&self.repo_root).unwrap_or_else(|_| "main".to_owned());

        // Check if this branch already has a worktree
        let worktree_path = self.find_worktree_for_branch(&branch_name);

        let path = match worktree_path {
            Some(p) => p,
            None => {
                // No worktree exists — create one
                let wt_path = match self.config.workspace.layout {
                    crate::config::WorktreeLayout::Sibling => {
                        let repo_name = self
                            .repo_root
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_else(|| "repo".to_string());
                        self.repo_root
                            .parent()
                            .unwrap_or(&self.repo_root)
                            .join(format!("{}.{}", repo_name, ticket))
                    }
                    crate::config::WorktreeLayout::Internal => self
                        .repo_root
                        .join(&self.config.workspace.base_dir)
                        .join(ticket),
                };
                git::run(
                    &self.repo_root,
                    &[
                        "worktree",
                        "add",
                        wt_path.to_str().unwrap_or(""),
                        &branch_name,
                    ],
                )
                .with_context(|| {
                    format!(
                        "failed to create worktree for branch '{}' at {:?}",
                        branch_name, wt_path
                    )
                })?;
                wt_path
            }
        };

        let workspace = Workspace {
            ticket: ticket.to_owned(),
            path,
            branch: branch_name,
            base_branch,
            created_at: Utc::now(),
            ticket_title,
            status: WorkspaceStatus::Active,
        };

        state.add_workspace(workspace.clone());
        state
            .save(&self.repo_root)
            .context("failed to save parsec state after adopt")?;

        Ok(workspace)
    }

    /// Find existing worktree path for a given branch name.
    fn find_worktree_for_branch(&self, branch: &str) -> Option<PathBuf> {
        let output = git::run_output(&self.repo_root, &["worktree", "list", "--porcelain"]).ok()?;

        let mut current_path: Option<String> = None;
        for line in output.lines() {
            if let Some(val) = line.strip_prefix("worktree ") {
                current_path = Some(val.to_owned());
            } else if let Some(val) = line.strip_prefix("branch ") {
                let wt_branch = val.strip_prefix("refs/heads/").unwrap_or(val);
                if wt_branch == branch {
                    return current_path.map(PathBuf::from);
                }
            } else if line.is_empty() {
                current_path = None;
            }
        }
        None
    }

    // -----------------------------------------------------------------------
    // list
    // -----------------------------------------------------------------------

    pub fn list(&self) -> Result<Vec<Workspace>> {
        let state = ParsecState::load(&self.repo_root).context("failed to load parsec state")?;

        let mut workspaces: Vec<Workspace> = state.workspaces.into_values().collect();
        workspaces.sort_by_key(|w| w.created_at);
        Ok(workspaces)
    }

    // -----------------------------------------------------------------------
    // get
    // -----------------------------------------------------------------------

    pub fn get(&self, ticket: &str) -> Result<Workspace> {
        let state = ParsecState::load(&self.repo_root).context("failed to load parsec state")?;

        state
            .get_workspace(ticket)
            .cloned()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "no workspace found for ticket '{}'. Run `parsec list` to see active workspaces, or `parsec adopt {}` to import an existing branch.",
                    ticket, ticket
                )
            })
    }

    // -----------------------------------------------------------------------
    // ship (push + cleanup only, PR creation is in commands.rs)
    // -----------------------------------------------------------------------

    pub fn ship(&self, ticket: &str) -> Result<ShipResult> {
        let mut state =
            ParsecState::load(&self.repo_root).context("failed to load parsec state")?;

        let workspace = state
            .get_workspace(ticket)
            .cloned()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "no workspace found for ticket '{}'. Run `parsec list` to see active workspaces, or `parsec adopt {}` to import an existing branch.",
                    ticket, ticket
                )
            })?;

        // Push the branch from the worktree itself so HEAD is correct.
        git::push_branch(&workspace.path, &workspace.branch)
            .with_context(|| format!("failed to push branch '{}'", workspace.branch))?;

        // Optionally clean up the worktree and local branch.
        let cleaned_up = if self.config.ship.auto_cleanup {
            match git::worktree_remove(&self.repo_root, &workspace.path) {
                Ok(()) => {
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
        } else if let Some(ws) = state.workspaces.get_mut(ticket) {
            ws.status = WorkspaceStatus::Shipped;
        }
        state
            .save(&self.repo_root)
            .context("failed to save parsec state after ship")?;

        Ok(ShipResult {
            ticket: ticket.to_owned(),
            branch: workspace.branch,
            base_branch: workspace.base_branch,
            ticket_title: workspace.ticket_title,
            pr_url: None, // Set by commands.rs after async PR creation
            cleaned_up,
        })
    }

    // -----------------------------------------------------------------------
    // clean
    // -----------------------------------------------------------------------

    pub fn clean(&self, all: bool, dry_run: bool) -> Result<Vec<Workspace>> {
        let mut state =
            ParsecState::load(&self.repo_root).context("failed to load parsec state")?;

        let candidates: Vec<Workspace> = state
            .workspaces
            .values()
            .filter(|ws| {
                if all {
                    return true;
                }
                git::is_branch_merged(&self.repo_root, &ws.branch, &ws.base_branch).unwrap_or(false)
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

            state
                .save(&self.repo_root)
                .context("failed to save parsec state after clean")?;
        }

        Ok(candidates)
    }
}
