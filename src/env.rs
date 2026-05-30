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

/// Resolve GitHub token. Priority:
/// 1. `PARSEC_GITHUB_TOKEN`
/// 2. `GITHUB_TOKEN`
/// 3. `GH_TOKEN`
/// 4. `gh auth token` shell fallback (issue #281 — parity with `parsec doctor` /
///    tracker layer; `parsec ship` previously rejected this path)
pub fn github_token() -> Option<String> {
    for var in [PARSEC_GITHUB_TOKEN, GITHUB_TOKEN, GH_TOKEN] {
        if let Ok(token) = std::env::var(var) {
            if !token.is_empty() {
                return Some(token);
            }
        }
    }
    gh_auth_token()
}

/// Shell out to `gh auth token` and capture stdout. Returns `None` on failure:
/// binary not found, exit code != 0, non-UTF8 stdout, or empty token.
///
/// Used as the final fallback in [`github_token`] (issue #281 — parity with
/// `parsec doctor` and the tracker layer). Cross-platform: relies on `gh`
/// being on PATH; failures are silent so callers present a unified "no token
/// found" message.
pub fn gh_auth_token() -> Option<String> {
    let out = std::process::Command::new("gh")
        .args(["auth", "token"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let token = String::from_utf8(out.stdout).ok()?.trim().to_string();
    if token.is_empty() {
        None
    } else {
        Some(token)
    }
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
/// Override Bitbucket Cloud API base URL. Useful for tests (mock servers) and
/// future Bitbucket Server / Data Center support.
pub const PARSEC_BITBUCKET_API_BASE: &str = "PARSEC_BITBUCKET_API_BASE";

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

/// Bitbucket API base URL override (no trailing slash). Returns None when unset.
pub fn bitbucket_api_base() -> Option<String> {
    std::env::var(PARSEC_BITBUCKET_API_BASE)
        .ok()
        .filter(|v| !v.is_empty())
        .map(|v| v.trim_end_matches('/').to_string())
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    /// Process-wide mutex to serialize env-touching tests. cargo test runs
    /// tests in parallel by default, so any test that mutates env vars must
    /// hold this lock — otherwise sibling tests racing through `set_var` /
    /// `remove_var` clobber each other (Windows CI hit this with priority_order
    /// reading PARSEC=p but seeing GH=h because another test cleared PARSEC
    /// mid-assertion).
    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    /// Helper: snapshot/clear env vars affecting github_token, then restore.
    /// std::env::set_var/remove_var is unsafe in Rust 2024. Tests holding
    /// `env_lock()` only run serially, so the snapshot+restore is sufficient.
    struct EnvGuard {
        orig: Vec<(&'static str, Option<String>)>,
    }
    impl EnvGuard {
        fn new(vars: &[&'static str]) -> Self {
            let orig = vars.iter().map(|v| (*v, std::env::var(v).ok())).collect();
            for v in vars {
                // SAFETY: tests run serially within a module by default in Rust 2024.
                #[allow(unused_unsafe)]
                unsafe {
                    std::env::remove_var(v)
                };
            }
            Self { orig }
        }
        fn set(&self, key: &str, val: &str) {
            #[allow(unused_unsafe)]
            unsafe {
                std::env::set_var(key, val)
            };
        }
    }
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (k, v) in &self.orig {
                #[allow(unused_unsafe)]
                unsafe {
                    if let Some(val) = v {
                        std::env::set_var(k, val);
                    } else {
                        std::env::remove_var(k);
                    }
                }
            }
        }
    }

    /// 우선순위 + 빈값 fallback + 모두 미설정 시나리오를 한 함수에서 sequential 검사.
    /// `env_lock()` 으로 process-wide 직렬화 (cargo test 병렬 실행 환경에서 sibling
    /// 테스트가 env 를 클로버하지 않도록). Windows CI 에서 race 발견 (#289).
    #[test]
    fn github_token_priority_order_and_fallback() {
        let _guard = env_lock().lock().unwrap_or_else(|p| p.into_inner());
        // 1. PARSEC_GITHUB_TOKEN 우선
        {
            let g = EnvGuard::new(&[PARSEC_GITHUB_TOKEN, GITHUB_TOKEN, GH_TOKEN]);
            g.set(PARSEC_GITHUB_TOKEN, "p");
            g.set(GITHUB_TOKEN, "g");
            g.set(GH_TOKEN, "h");
            assert_eq!(github_token().as_deref(), Some("p"));
            drop(g);
        }
        // 2. PARSEC_GITHUB_TOKEN 미설정 → GITHUB_TOKEN
        {
            let g = EnvGuard::new(&[PARSEC_GITHUB_TOKEN, GITHUB_TOKEN, GH_TOKEN]);
            g.set(GITHUB_TOKEN, "g");
            g.set(GH_TOKEN, "h");
            assert_eq!(github_token().as_deref(), Some("g"));
            drop(g);
        }
        // 3. PARSEC_GITHUB_TOKEN / GITHUB_TOKEN 미설정 → GH_TOKEN
        {
            let g = EnvGuard::new(&[PARSEC_GITHUB_TOKEN, GITHUB_TOKEN, GH_TOKEN]);
            g.set(GH_TOKEN, "h");
            assert_eq!(github_token().as_deref(), Some("h"));
            drop(g);
        }
        // 4. 빈 PARSEC_GITHUB_TOKEN 은 무시 → GITHUB_TOKEN
        {
            let g = EnvGuard::new(&[PARSEC_GITHUB_TOKEN, GITHUB_TOKEN, GH_TOKEN]);
            g.set(PARSEC_GITHUB_TOKEN, "");
            g.set(GITHUB_TOKEN, "g");
            assert_eq!(github_token().as_deref(), Some("g"));
            drop(g);
        }
        // 5. 모두 미설정 + gh 실패 → None. CI 환경 (gh 로그인 X) 이 일반.
        //    local dev 에서 gh auth login 돼있으면 Some(token) 도 허용 (smoke).
        {
            let g = EnvGuard::new(&[PARSEC_GITHUB_TOKEN, GITHUB_TOKEN, GH_TOKEN]);
            match github_token() {
                None => {}
                Some(t) => assert!(
                    !t.is_empty(),
                    "if gh auth token is available, it must not be empty"
                ),
            }
            drop(g);
        }
    }

    #[test]
    fn gh_auth_token_returns_option_string_or_none() {
        // 외부 gh binary 에 의존 — CI 환경 (로그인 X) 에서는 None 기대.
        // local dev 에서 gh auth login 돼있으면 Some(token). 둘 다 허용 (smoke check only).
        match gh_auth_token() {
            None => {}
            Some(t) => {
                assert!(!t.is_empty());
                assert!(!t.contains('\n'), "trimmed");
            }
        }
    }
}
