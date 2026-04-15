use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io;
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
    #[serde(default)]
    pub parent_ticket: Option<String>,
}

// ---------------------------------------------------------------------------
// ShipResult
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShipResult {
    pub ticket: String,
    pub branch: String,
    pub base_branch: String,
    pub ticket_title: Option<String>,
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

    /// Acquire a lock file. Returns the lock file path for cleanup.
    fn acquire_lock(repo_root: &Path) -> Result<PathBuf> {
        let lock_path = Self::state_path(repo_root).with_extension("lock");

        let mut attempts = 0u32;
        loop {
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&lock_path)
            {
                Ok(mut f) => {
                    use std::io::Write;
                    // Write PID for debugging
                    let _ = writeln!(f, "{}", std::process::id());
                    return Ok(lock_path);
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                    attempts += 1;
                    if attempts > 50 {
                        // Check if lock is stale (older than 30 seconds)
                        if let Ok(meta) = fs::metadata(&lock_path) {
                            if let Ok(modified) = meta.modified() {
                                if modified.elapsed().unwrap_or_default()
                                    > std::time::Duration::from_secs(30)
                                {
                                    let _ = fs::remove_file(&lock_path);
                                    continue;
                                }
                            }
                        }
                        bail!(
                            "Could not acquire state lock after {} attempts. \
                             Remove {} manually if stale.",
                            attempts,
                            lock_path.display()
                        );
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                Err(e) => {
                    return Err(anyhow::anyhow!("Failed to create lock file: {}", e));
                }
            }
        }
    }

    /// Release a previously acquired lock file.
    fn release_lock(lock_path: &Path) {
        let _ = fs::remove_file(lock_path);
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
    ///
    /// Uses a lock file to prevent concurrent writes and an atomic rename so
    /// readers never see a partially-written file.
    pub fn save(&self, repo_root: &Path) -> Result<()> {
        let path = Self::state_path(repo_root);

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create state directory: {}", parent.display())
            })?;
        }

        let lock_path = Self::acquire_lock(repo_root)?;

        let result = (|| {
            let contents =
                serde_json::to_string_pretty(self).context("failed to serialize state to JSON")?;

            // Write to a temp file alongside the real state file, then rename.
            let tmp_path = path.with_extension("tmp");
            fs::write(&tmp_path, &contents).with_context(|| {
                format!("failed to write temp state file: {}", tmp_path.display())
            })?;
            fs::rename(&tmp_path, &path).with_context(|| {
                format!("failed to rename temp state file to: {}", path.display())
            })?;

            Ok(())
        })();

        Self::release_lock(&lock_path);
        result
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
