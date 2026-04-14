use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// WorkspaceStatus
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum WorkspaceStatus {
    Active,
    Shipped,
    Merged,
}

// ---------------------------------------------------------------------------
// Workspace
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    pub ticket: String,
    pub path: PathBuf,
    pub branch: String,
    pub base_branch: String,
    pub created_at: DateTime<Utc>,
    pub ticket_title: Option<String>,
    pub status: WorkspaceStatus,
}

// ---------------------------------------------------------------------------
// ShipResult
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShipResult {
    pub ticket: String,
    pub branch: String,
    pub pr_url: Option<String>,
    pub cleaned_up: bool,
}

// ---------------------------------------------------------------------------
// ParsecState
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ParsecState {
    pub workspaces: HashMap<String, Workspace>,
}

impl ParsecState {
    /// Return the canonical path to the state file.
    pub fn state_path(repo_root: &Path) -> PathBuf {
        repo_root.join(".parsec").join("state.json")
    }

    /// Load state from `{repo_root}/.parsec/state.json`.
    /// Returns an empty state if the file does not exist.
    pub fn load(repo_root: &Path) -> Result<Self> {
        let path = Self::state_path(repo_root);

        if !path.exists() {
            return Ok(Self::default());
        }

        let contents = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read state file: {}", path.display()))?;

        let state: Self = serde_json::from_str(&contents)
            .with_context(|| format!("failed to parse state file: {}", path.display()))?;

        Ok(state)
    }

    /// Persist state to `{repo_root}/.parsec/state.json`, creating directories as needed.
    pub fn save(&self, repo_root: &Path) -> Result<()> {
        let path = Self::state_path(repo_root);

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("failed to create state directory: {}", parent.display())
            })?;
        }

        let contents =
            serde_json::to_string_pretty(self).context("failed to serialize state to JSON")?;

        std::fs::write(&path, contents)
            .with_context(|| format!("failed to write state file: {}", path.display()))?;

        Ok(())
    }

    /// Insert a workspace, keyed by its ticket identifier.
    pub fn add_workspace(&mut self, workspace: Workspace) {
        self.workspaces.insert(workspace.ticket.clone(), workspace);
    }

    /// Remove a workspace by ticket, returning the removed entry if it existed.
    pub fn remove_workspace(&mut self, ticket: &str) -> Option<Workspace> {
        self.workspaces.remove(ticket)
    }

    /// Look up a workspace by ticket.
    pub fn get_workspace(&self, ticket: &str) -> Option<&Workspace> {
        self.workspaces.get(ticket)
    }
}
