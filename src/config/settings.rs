use anyhow::{Context, Result};
use dialoguer::{Confirm, Input, Select};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Default value helpers required by serde
// ---------------------------------------------------------------------------

fn default_base_dir() -> String {
    ".parsec/workspaces".to_string()
}

fn default_branch_prefix() -> String {
    "feature/".to_string()
}

fn default_provider() -> TrackerProvider {
    TrackerProvider::None
}

fn default_true() -> bool {
    true
}

// ---------------------------------------------------------------------------
// TrackerProvider
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum TrackerProvider {
    Jira,
    Github,
    Gitlab,
    #[default]
    None,
}

impl std::fmt::Display for TrackerProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TrackerProvider::Jira => write!(f, "jira"),
            TrackerProvider::Github => write!(f, "github"),
            TrackerProvider::Gitlab => write!(f, "gitlab"),
            TrackerProvider::None => write!(f, "none"),
        }
    }
}

// ---------------------------------------------------------------------------
// WorktreeLayout
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum WorktreeLayout {
    #[default]
    Sibling, // ../repo.ticket/ (worktrunk-style, default)
    Internal, // .parsec/workspaces/ticket/ (inside repo)
}

fn default_layout() -> WorktreeLayout {
    WorktreeLayout::Sibling
}

impl std::fmt::Display for WorktreeLayout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WorktreeLayout::Sibling => write!(f, "sibling"),
            WorktreeLayout::Internal => write!(f, "internal"),
        }
    }
}

// ---------------------------------------------------------------------------
// WorkspaceConfig
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    #[serde(default = "default_layout")]
    pub layout: WorktreeLayout,
    #[serde(default = "default_base_dir")]
    pub base_dir: String, // only used for Internal layout
    #[serde(default = "default_branch_prefix")]
    pub branch_prefix: String,
    /// Default base branch for worktree creation (e.g. "develop")
    #[serde(default)]
    pub default_base: Option<String>,
}

impl Default for WorkspaceConfig {
    fn default() -> Self {
        Self {
            layout: default_layout(),
            base_dir: default_base_dir(),
            branch_prefix: default_branch_prefix(),
            default_base: None,
        }
    }
}

// ---------------------------------------------------------------------------
// JiraConfig
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JiraConfig {
    pub base_url: String,
    pub email: Option<String>,
    pub project: Option<String>,
    pub board_id: Option<u64>,
    pub assignee: Option<String>,
}

// ---------------------------------------------------------------------------
// GitlabConfig
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitlabConfig {
    pub base_url: String,
}

// ---------------------------------------------------------------------------
// TrackerConfig
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackerConfig {
    #[serde(default = "default_provider")]
    pub provider: TrackerProvider,
    #[serde(default)]
    pub jira: Option<JiraConfig>,
    #[serde(default)]
    pub gitlab: Option<GitlabConfig>,
    #[serde(default)]
    pub auto_transition: Option<AutoTransitionConfig>,
    /// When true, auto-post PR link as comment on the ticket during `parsec ship`
    #[serde(default)]
    pub comment_on_ship: bool,
}

impl Default for TrackerConfig {
    fn default() -> Self {
        Self {
            provider: default_provider(),
            jira: None,
            gitlab: None,
            auto_transition: None,
            comment_on_ship: false,
        }
    }
}

// ---------------------------------------------------------------------------
// ShipConfig
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShipConfig {
    #[serde(default = "default_true")]
    pub auto_pr: bool,
    #[serde(default = "default_true")]
    pub auto_cleanup: bool,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub default_base: Option<String>,
}

impl Default for ShipConfig {
    fn default() -> Self {
        Self {
            auto_pr: true,
            auto_cleanup: true,
            draft: false,
            default_base: None,
        }
    }
}

// ---------------------------------------------------------------------------
// HooksConfig
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HooksConfig {
    /// Commands to run after creating a worktree (in the worktree directory)
    #[serde(default)]
    pub post_create: Vec<String>,
}

// ---------------------------------------------------------------------------
// AutoTransitionConfig
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AutoTransitionConfig {
    /// Target status name when `parsec start` is run (e.g. "In Progress")
    #[serde(default)]
    pub on_start: Option<String>,
    /// Target status name when `parsec ship` is run (e.g. "In Review")
    #[serde(default)]
    pub on_ship: Option<String>,
    /// Target status name when `parsec merge` is run (e.g. "Done")
    #[serde(default)]
    pub on_merge: Option<String>,
}

// ---------------------------------------------------------------------------
// GithubHostConfig
// ---------------------------------------------------------------------------

/// Per-host GitHub configuration (e.g. token for github.com or a GHE instance).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GithubHostConfig {
    /// Personal access token for this host.
    pub token: Option<String>,
}

// ---------------------------------------------------------------------------
// RepoTrackerOverride / RepoOverrideConfig
// ---------------------------------------------------------------------------

/// Tracker overrides that can be set per-repo.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RepoTrackerOverride {
    pub provider: Option<TrackerProvider>,
    #[serde(default)]
    pub jira: Option<JiraConfig>,
    #[serde(default)]
    pub gitlab: Option<GitlabConfig>,
}

/// Per-repo configuration overrides.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RepoOverrideConfig {
    #[serde(default)]
    pub tracker: Option<RepoTrackerOverride>,
}

// ---------------------------------------------------------------------------
// ParsecConfig
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ParsecConfig {
    #[serde(default)]
    pub workspace: WorkspaceConfig,
    #[serde(default)]
    pub tracker: TrackerConfig,
    #[serde(default)]
    pub ship: ShipConfig,
    #[serde(default)]
    pub hooks: HooksConfig,
    /// Per-host GitHub tokens. Keys are hostnames like "github.com" or
    /// "github.example.com". Serializes as `[github."hostname"]` in TOML.
    #[serde(default)]
    pub github: HashMap<String, GithubHostConfig>,
    /// Per-repo configuration overrides. Keys are "owner/repo" strings.
    /// Serializes as `[repos."owner/repo"]` in TOML.
    #[serde(default)]
    pub repos: HashMap<String, RepoOverrideConfig>,
}

impl ParsecConfig {
    /// Return the canonical path to the config file.
    pub fn config_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| {
                dirs::home_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join(".config")
            })
            .join("parsec")
            .join("config.toml")
    }

    /// Load the config from disk. Returns `Default` if the file does not exist.
    pub fn load() -> Result<Self> {
        let path = Self::config_path();

        if !path.exists() {
            return Ok(Self::default());
        }

        let contents = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read config file: {}", path.display()))?;

        let config: Self = toml::from_str(&contents)
            .with_context(|| format!("Failed to parse config file: {}", path.display()))?;

        Ok(config)
    }

    /// Persist the config to disk, creating parent directories as needed.
    pub fn save(&self) -> Result<()> {
        let path = Self::config_path();

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("Failed to create config directory: {}", parent.display())
            })?;
        }

        let contents =
            toml::to_string_pretty(self).context("Failed to serialize config to TOML")?;

        std::fs::write(&path, contents)
            .with_context(|| format!("Failed to write config file: {}", path.display()))?;

        Ok(())
    }

    /// Apply per-repo tracker overrides for the repository at `repo_root`.
    ///
    /// Runs `git remote get-url origin`, parses `owner/repo` from the URL,
    /// and merges any matching `[repos."owner/repo".tracker]` settings into
    /// `self.tracker`. Silently ignores errors (no remote, no match, etc.).
    pub fn resolve_for_repo(&mut self, repo_root: &Path) {
        // Get the origin remote URL; silently skip if unavailable.
        let remote_url = match std::process::Command::new("git")
            .args(["remote", "get-url", "origin"])
            .current_dir(repo_root)
            .output()
        {
            Ok(out) if out.status.success() => String::from_utf8(out.stdout).unwrap_or_default(),
            _ => return,
        };
        let remote_url = remote_url.trim();

        // Parse owner/repo from the URL.
        let parsed = crate::github::parse_github_remote(remote_url);
        let remote = match parsed {
            Some(r) => r,
            None => return,
        };
        let key = format!("{}/{}", remote.owner, remote.repo);

        // Look up the per-repo override.
        let repo_cfg = match self.repos.get(&key) {
            Some(r) => r.clone(),
            None => return,
        };

        let tracker_override = match repo_cfg.tracker {
            Some(t) => t,
            None => return,
        };

        // Apply overrides.
        if let Some(provider) = tracker_override.provider {
            self.tracker.provider = provider;
        }
        if let Some(jira) = tracker_override.jira {
            self.tracker.jira = Some(jira);
        }
        if let Some(gitlab) = tracker_override.gitlab {
            self.tracker.gitlab = Some(gitlab);
        }
    }

    /// Interactively prompt the user to configure parsec and return the resulting config.
    pub fn init_interactive() -> Result<Self> {
        let mut config = Self::default();

        // ---- Tracker provider ------------------------------------------------
        let provider_options = &["None", "Jira", "GitHub", "GitLab"];
        let provider_idx = Select::new()
            .with_prompt("Issue tracker provider")
            .items(provider_options)
            .default(0)
            .interact()
            .context("Failed to read tracker provider selection")?;

        config.tracker.provider = match provider_idx {
            1 => TrackerProvider::Jira,
            2 => TrackerProvider::Github,
            3 => TrackerProvider::Gitlab,
            _ => TrackerProvider::None,
        };

        // ---- Jira-specific options -------------------------------------------
        if config.tracker.provider == TrackerProvider::Jira {
            let base_url: String = Input::new()
                .with_prompt("Jira base URL (e.g. https://yourorg.atlassian.net)")
                .interact_text()
                .context("Failed to read Jira base URL")?;

            let email_input: String = Input::new()
                .with_prompt("Jira email (leave blank to skip)")
                .allow_empty(true)
                .interact_text()
                .context("Failed to read Jira email")?;

            config.tracker.jira = Some(JiraConfig {
                base_url,
                email: if email_input.is_empty() {
                    None
                } else {
                    Some(email_input)
                },
                project: None,
                board_id: None,
                assignee: None,
            });
        }

        // ---- GitLab-specific options -----------------------------------------
        if config.tracker.provider == TrackerProvider::Gitlab {
            let base_url: String = Input::new()
                .with_prompt("GitLab base URL (e.g. https://gitlab.com)")
                .default("https://gitlab.com".to_string())
                .interact_text()
                .context("Failed to read GitLab base URL")?;

            config.tracker.gitlab = Some(GitlabConfig { base_url });
        }

        // ---- Worktree layout -------------------------------------------------
        let layout_options = &[
            "Sibling (recommended - worktrees next to repo)",
            "Internal (worktrees inside .parsec/)",
        ];
        let layout_idx = Select::new()
            .with_prompt("Worktree layout")
            .items(layout_options)
            .default(0)
            .interact()
            .context("Failed to read layout selection")?;
        config.workspace.layout = match layout_idx {
            1 => WorktreeLayout::Internal,
            _ => WorktreeLayout::Sibling,
        };

        // ---- Branch prefix ---------------------------------------------------
        let branch_prefix: String = Input::new()
            .with_prompt("Branch prefix for new worktrees")
            .default("feature/".to_string())
            .interact_text()
            .context("Failed to read branch prefix")?;

        config.workspace.branch_prefix = branch_prefix;

        // ---- Ship options ----------------------------------------------------
        config.ship.auto_pr = Confirm::new()
            .with_prompt("Automatically open a PR when shipping?")
            .default(true)
            .interact()
            .context("Failed to read auto PR preference")?;

        config.ship.draft = Confirm::new()
            .with_prompt("Create PRs as drafts by default?")
            .default(false)
            .interact()
            .context("Failed to read draft PR preference")?;

        Ok(config)
    }
}
