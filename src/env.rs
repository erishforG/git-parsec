//! Centralized environment variable definitions for parsec.
//!
//! All env var names and token resolution logic live here so that
//! adding or renaming a variable only requires touching one file.

// ---------------------------------------------------------------------------
// Jira
// ---------------------------------------------------------------------------

pub const PARSEC_JIRA_TOKEN: &str = "PARSEC_JIRA_TOKEN";
pub const JIRA_PAT: &str = "JIRA_PAT";
pub const JIRA_BASE_URL: &str = "JIRA_BASE_URL";
pub const PARSEC_JIRA_PROJECT: &str = "PARSEC_JIRA_PROJECT";
pub const PARSEC_JIRA_BOARD_ID: &str = "PARSEC_JIRA_BOARD_ID";
pub const PARSEC_JIRA_ASSIGNEE: &str = "PARSEC_JIRA_ASSIGNEE";

// ---------------------------------------------------------------------------
// GitHub
// ---------------------------------------------------------------------------

pub const PARSEC_GITHUB_TOKEN: &str = "PARSEC_GITHUB_TOKEN";
pub const GITHUB_TOKEN: &str = "GITHUB_TOKEN";
pub const GH_TOKEN: &str = "GH_TOKEN";

// ---------------------------------------------------------------------------
// GitLab
// ---------------------------------------------------------------------------

pub const PARSEC_GITLAB_TOKEN: &str = "PARSEC_GITLAB_TOKEN";
pub const GITLAB_TOKEN: &str = "GITLAB_TOKEN";

// ---------------------------------------------------------------------------
// Token resolvers
// ---------------------------------------------------------------------------

/// Resolve Jira API token. Priority: PARSEC_JIRA_TOKEN > JIRA_PAT > config token
pub fn jira_token(config_token: Option<&str>) -> Option<String> {
    for var in [PARSEC_JIRA_TOKEN, JIRA_PAT] {
        if let Ok(token) = std::env::var(var) {
            if !token.is_empty() {
                return Some(token);
            }
        }
    }
    config_token
        .filter(|t| !t.is_empty())
        .map(|t| t.to_string())
}

/// Resolve GitHub token. Priority: PARSEC_GITHUB_TOKEN > GITHUB_TOKEN > GH_TOKEN
pub fn github_token() -> Option<String> {
    for var in [PARSEC_GITHUB_TOKEN, GITHUB_TOKEN, GH_TOKEN] {
        if let Ok(token) = std::env::var(var) {
            if !token.is_empty() {
                return Some(token);
            }
        }
    }
    None
}

/// Resolve GitLab token. Priority: PARSEC_GITLAB_TOKEN > GITLAB_TOKEN
pub fn gitlab_token() -> Option<String> {
    for var in [PARSEC_GITLAB_TOKEN, GITLAB_TOKEN] {
        if let Ok(token) = std::env::var(var) {
            if !token.is_empty() {
                return Some(token);
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Agent mode
// ---------------------------------------------------------------------------

pub const PARSEC_AGENT: &str = "PARSEC_AGENT";

/// Check if agent mode is active (via PARSEC_AGENT env var).
/// In agent mode: JSON output is forced, interactive prompts are skipped.
pub fn is_agent() -> bool {
    std::env::var(PARSEC_AGENT)
        .map(|v| v == "1" || v == "true")
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Offline mode
// ---------------------------------------------------------------------------

pub const PARSEC_OFFLINE: &str = "PARSEC_OFFLINE";

/// Check if offline mode is active (via --offline flag or PARSEC_OFFLINE env var).
pub fn is_offline() -> bool {
    std::env::var(PARSEC_OFFLINE)
        .map(|v| v == "1" || v == "true")
        .unwrap_or(false)
}
