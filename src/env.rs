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
// AI
// ---------------------------------------------------------------------------

pub const PARSEC_AI_API_KEY: &str = "PARSEC_AI_API_KEY";
pub const OPENAI_API_KEY: &str = "OPENAI_API_KEY";
pub const ANTHROPIC_API_KEY: &str = "ANTHROPIC_API_KEY";

/// Resolve AI API key. Priority: PARSEC_AI_API_KEY > provider-specific > config
pub fn ai_api_key(config_key: Option<&str>) -> Option<String> {
    for var in [PARSEC_AI_API_KEY, OPENAI_API_KEY, ANTHROPIC_API_KEY] {
        if let Ok(key) = std::env::var(var) {
            if !key.is_empty() {
                return Some(key);
            }
        }
    }
    config_key.filter(|k| !k.is_empty()).map(|k| k.to_string())
}

// ---------------------------------------------------------------------------
// Bitbucket
// ---------------------------------------------------------------------------

pub const PARSEC_BITBUCKET_TOKEN: &str = "PARSEC_BITBUCKET_TOKEN";
pub const BITBUCKET_TOKEN: &str = "BITBUCKET_TOKEN";

/// Resolve Bitbucket token. Priority: PARSEC_BITBUCKET_TOKEN > BITBUCKET_TOKEN
pub fn bitbucket_token() -> Option<String> {
    for var in [PARSEC_BITBUCKET_TOKEN, BITBUCKET_TOKEN] {
        if let Ok(token) = std::env::var(var) {
            if !token.is_empty() {
                return Some(token);
            }
        }
    }
    None
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
