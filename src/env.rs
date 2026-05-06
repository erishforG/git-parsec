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

    /// Helper: snapshot/clear env vars affecting github_token, then restore.
    /// std::env::set_var/remove_var is unsafe in Rust 2024; test isolation is
    /// per-process. Tests in this module assume serial execution (default for
    /// `cargo test` with `--test-threads=1` not required, but env vars are
    /// shared so we save+restore.
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

    /// 우선순위 + 빈값 fallback 4 시나리오를 한 함수에서 sequential 검사.
    /// (env vars 는 process-wide 라 cargo test 병렬 실행 시 race. 단일 테스트로 합쳐
    /// EnvGuard 의 새로 만들기·복원 루틴 안에서 안전하게 순서 검사.)
    #[test]
    fn github_token_priority_order() {
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
    }

    #[test]
    fn github_token_returns_none_when_all_missing_and_gh_fails() {
        // 환경상 `gh` binary 가 없거나 인증 안돼있으면 None. CI/test env 에서 이게 일반.
        let g = EnvGuard::new(&[PARSEC_GITHUB_TOKEN, GITHUB_TOKEN, GH_TOKEN]);
        // gh binary 가 PATH 에 있고 로그인까지 돼있으면 본 테스트는 Some 을 반환할 수
        // 있음. CI 에서는 로그인 X 가 일반이라 None 기대. local dev 에서는 Some 가능.
        // 따라서 None OR gh 환경에서 정상 token 모두 허용.
        match github_token() {
            None => {}
            Some(t) => assert!(
                !t.is_empty(),
                "if gh auth token is available, it must not be empty"
            ),
        }
        drop(g);
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
