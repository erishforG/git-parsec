use std::path::Path;

use anyhow::Result;

use crate::git;
use crate::output::{self, Mode};

pub async fn doctor(repo: &Path, mode: Mode) -> Result<()> {
    use output::DoctorCheck;
    use std::process::Command as StdCommand;

    let mut checks: Vec<DoctorCheck> = Vec::new();

    // ------------------------------------------------------------------
    // 1. git version and worktree support (requires >= 2.15)
    // ------------------------------------------------------------------
    {
        let git_out = StdCommand::new("git").arg("--version").output();
        match git_out {
            Err(_) => {
                checks.push(DoctorCheck {
                    name: "git_version".to_string(),
                    ok: false,
                    detail: "git not found".to_string(),
                    fix: Some("Install git: https://git-scm.com/downloads".to_string()),
                });
            }
            Ok(out) => {
                let version_str = String::from_utf8_lossy(&out.stdout).trim().to_string();
                // Parse "git version X.Y.Z"
                let ver_nums: Option<(u32, u32)> = version_str
                    .split_whitespace()
                    .find(|s| s.contains('.'))
                    .and_then(|v| {
                        let parts: Vec<&str> = v.split('.').collect();
                        let major = parts.first().and_then(|s| s.parse::<u32>().ok())?;
                        let minor = parts.get(1).and_then(|s| s.parse::<u32>().ok())?;
                        Some((major, minor))
                    });
                let ok = match ver_nums {
                    Some((major, minor)) => major > 2 || (major == 2 && minor >= 15),
                    None => false,
                };
                checks.push(DoctorCheck {
                    name: "git_version".to_string(),
                    ok,
                    detail: if ok {
                        format!("{} (worktree support ok)", version_str)
                    } else {
                        format!("{} (need >= 2.15 for worktree support)", version_str)
                    },
                    fix: if ok {
                        None
                    } else {
                        Some("Upgrade git to 2.15 or later".to_string())
                    },
                });
            }
        }
    }

    // ------------------------------------------------------------------
    // 2. Config file
    // ------------------------------------------------------------------
    {
        let config_path = crate::config::ParsecConfig::config_path();
        let exists = config_path.exists();
        checks.push(DoctorCheck {
            name: "config_file".to_string(),
            ok: exists,
            detail: if exists {
                format!("config file found at {}", config_path.display())
            } else {
                format!("config file not found at {}", config_path.display())
            },
            fix: if exists {
                None
            } else {
                Some("Run `parsec config init` to create the config file".to_string())
            },
        });
    }

    // ------------------------------------------------------------------
    // 3. Token configuration
    // ------------------------------------------------------------------
    {
        let config_result = crate::config::ParsecConfig::load();
        let github_token_found = match &config_result {
            Ok(cfg) => {
                let from_config = cfg.github.values().any(|h| h.token.is_some());
                let from_env = std::env::var("GITHUB_TOKEN").is_ok();
                let from_gh = StdCommand::new("gh")
                    .args(["auth", "token"])
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false);
                if from_config {
                    Some("config file")
                } else if from_env {
                    Some("GITHUB_TOKEN env var")
                } else if from_gh {
                    Some("gh auth token")
                } else {
                    None
                }
            }
            Err(_) => {
                let from_env = std::env::var("GITHUB_TOKEN").is_ok();
                let from_gh = StdCommand::new("gh")
                    .args(["auth", "token"])
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false);
                if from_env {
                    Some("GITHUB_TOKEN env var")
                } else if from_gh {
                    Some("gh auth token")
                } else {
                    None
                }
            }
        };

        match github_token_found {
            Some(source) => {
                let host = config_result
                    .as_ref()
                    .ok()
                    .and_then(|cfg| cfg.github.keys().next().cloned())
                    .unwrap_or_else(|| "github.com".to_string());
                checks.push(DoctorCheck {
                    name: "github_token".to_string(),
                    ok: true,
                    detail: format!("GitHub token configured ({host}) via {source}"),
                    fix: None,
                });
            }
            None => {
                checks.push(DoctorCheck {
                    name: "github_token".to_string(),
                    ok: false,
                    detail: "GitHub token not found".to_string(),
                    fix: Some(
                        "Set GITHUB_TOKEN, run `gh auth login`, or add token to config via `parsec config init`".to_string(),
                    ),
                });
            }
        }

        // Jira token check (only if Jira is configured)
        let jira_configured = config_result
            .as_ref()
            .map(|cfg| cfg.tracker.provider == crate::config::TrackerProvider::Jira)
            .unwrap_or(false);
        if jira_configured {
            let config_token = config_result
                .as_ref()
                .ok()
                .and_then(|cfg| cfg.tracker.jira.as_ref())
                .and_then(|j| j.token.as_deref());
            let token_found = crate::env::jira_token(config_token).is_some();
            let source = if std::env::var(crate::env::PARSEC_JIRA_TOKEN).is_ok() {
                "PARSEC_JIRA_TOKEN env var"
            } else if std::env::var(crate::env::JIRA_PAT).is_ok() {
                "JIRA_PAT env var"
            } else if config_token.is_some() {
                "config file"
            } else {
                "not found"
            };
            checks.push(DoctorCheck {
                name: "jira_token".to_string(),
                ok: token_found,
                detail: if token_found {
                    format!("Jira token configured ({})", source)
                } else {
                    "Jira token not found — set PARSEC_JIRA_TOKEN, JIRA_PAT, or add token to [tracker.jira] in config".to_string()
                },
                fix: if token_found {
                    None
                } else {
                    Some("Set PARSEC_JIRA_TOKEN env var or add token to config file".to_string())
                },
            });
        }
    }

    // ------------------------------------------------------------------
    // 4. Tracker connectivity
    // ------------------------------------------------------------------
    {
        let config_result = crate::config::ParsecConfig::load();
        if let Ok(cfg) = &config_result {
            match cfg.tracker.provider {
                crate::config::TrackerProvider::Jira => {
                    if let Some(jira) = &cfg.tracker.jira {
                        let url =
                            format!("{}/rest/api/2/myself", jira.base_url.trim_end_matches('/'));
                        let config_token =
                            cfg.tracker.jira.as_ref().and_then(|j| j.token.as_deref());
                        let token = crate::env::jira_token(config_token).unwrap_or_default();
                        let email = jira.email.clone().unwrap_or_default();
                        let reachable = {
                            let client = reqwest::Client::builder()
                                .timeout(std::time::Duration::from_secs(5))
                                .build()
                                .unwrap_or_default();
                            if token.is_empty() || email.is_empty() {
                                // Try unauthenticated; 200 or 401 both mean the
                                // server is reachable.
                                client
                                    .get(&url)
                                    .send()
                                    .await
                                    .map(|r| {
                                        let s = r.status().as_u16();
                                        s == 200 || s == 401
                                    })
                                    .unwrap_or(false)
                            } else {
                                // Authenticated check — credentials stay out of
                                // the process list.
                                client
                                    .get(&url)
                                    .basic_auth(&email, Some(&token))
                                    .send()
                                    .await
                                    .map(|r| r.status().as_u16() == 200)
                                    .unwrap_or(false)
                            }
                        };
                        checks.push(DoctorCheck {
                            name: "tracker_connectivity".to_string(),
                            ok: reachable,
                            detail: if reachable {
                                format!("Jira API reachable ({})", jira.base_url)
                            } else {
                                format!("Jira API unreachable ({})", jira.base_url)
                            },
                            fix: if reachable {
                                None
                            } else {
                                Some(format!("Check network and Jira URL: {}", jira.base_url))
                            },
                        });
                    }
                }
                crate::config::TrackerProvider::Github => {
                    // Derive API base URL from the configured GitHub host
                    // (supports GitHub Enterprise).
                    let gh_host = cfg
                        .github
                        .keys()
                        .next()
                        .map(String::as_str)
                        .unwrap_or("github.com");
                    let api_base = if gh_host == "github.com" {
                        "https://api.github.com".to_string()
                    } else {
                        format!("https://{}/api/v3", gh_host)
                    };
                    let reachable = {
                        let client = reqwest::Client::builder()
                            .timeout(std::time::Duration::from_secs(5))
                            .build()
                            .unwrap_or_default();
                        client
                            .get(&api_base)
                            .send()
                            .await
                            .map(|r| {
                                let s = r.status().as_u16();
                                s > 0 && s != 000
                            })
                            .unwrap_or(false)
                    };
                    checks.push(DoctorCheck {
                        name: "tracker_connectivity".to_string(),
                        ok: reachable,
                        detail: if reachable {
                            format!("GitHub API reachable ({})", gh_host)
                        } else {
                            format!("GitHub API unreachable ({})", gh_host)
                        },
                        fix: if reachable {
                            None
                        } else {
                            Some(format!("Check network connectivity to {}", gh_host))
                        },
                    });
                }
                _ => {}
            }
        }
    }

    // ------------------------------------------------------------------
    // 5. Shell integration installed
    // ------------------------------------------------------------------
    {
        let home = std::env::var("HOME").unwrap_or_default();
        let shell = std::env::var("SHELL").unwrap_or_default();

        let shell_files: Vec<std::path::PathBuf> = if shell.contains("zsh") {
            vec![std::path::PathBuf::from(format!("{}/.zshrc", home))]
        } else if shell.contains("bash") {
            vec![
                std::path::PathBuf::from(format!("{}/.bashrc", home)),
                std::path::PathBuf::from(format!("{}/.bash_profile", home)),
            ]
        } else {
            vec![
                std::path::PathBuf::from(format!("{}/.zshrc", home)),
                std::path::PathBuf::from(format!("{}/.bashrc", home)),
            ]
        };

        let shell_name = if shell.contains("zsh") { "zsh" } else { "bash" };
        let init_pattern = "parsec init";
        let found = shell_files.iter().any(|f| {
            std::fs::read_to_string(f)
                .map(|contents| contents.contains(init_pattern))
                .unwrap_or(false)
        });

        checks.push(DoctorCheck {
            name: "shell_integration".to_string(),
            ok: found,
            detail: if found {
                format!("shell integration installed ({})", shell_name)
            } else {
                "shell integration not found in shell config".to_string()
            },
            fix: if found {
                None
            } else {
                Some(format!(
                    r#"Add to ~/.{shell_name}rc:  eval "$(parsec init {shell_name})""#
                ))
            },
        });
    }

    // ------------------------------------------------------------------
    // 6. Tab completions configured
    // ------------------------------------------------------------------
    {
        let home = std::env::var("HOME").unwrap_or_default();
        let shell = std::env::var("SHELL").unwrap_or_default();
        let shell_name = if shell.contains("zsh") { "zsh" } else { "bash" };

        let shell_files: Vec<std::path::PathBuf> = if shell.contains("zsh") {
            vec![std::path::PathBuf::from(format!("{}/.zshrc", home))]
        } else {
            vec![
                std::path::PathBuf::from(format!("{}/.bashrc", home)),
                std::path::PathBuf::from(format!("{}/.bash_profile", home)),
            ]
        };

        let completions_pattern = "parsec config completions";
        let found = shell_files.iter().any(|f| {
            std::fs::read_to_string(f)
                .map(|contents| contents.contains(completions_pattern))
                .unwrap_or(false)
        });

        checks.push(DoctorCheck {
            name: "tab_completions".to_string(),
            ok: found,
            detail: if found {
                "tab completions configured".to_string()
            } else {
                "tab completions not configured".to_string()
            },
            fix: if found {
                None
            } else {
                Some(format!(
                    r#"Add to ~/.{shell_name}rc:  eval "$(parsec config completions {shell_name})""#
                ))
            },
        });
    }

    // ------------------------------------------------------------------
    // 7. Remote access
    // ------------------------------------------------------------------
    {
        let remote_url = git::run_output(repo, &["remote", "get-url", "origin"]);
        match remote_url {
            Err(_) => {
                checks.push(DoctorCheck {
                    name: "remote_access".to_string(),
                    ok: false,
                    detail: "no remote 'origin' configured".to_string(),
                    fix: Some("Run `git remote add origin <url>` to add a remote".to_string()),
                });
            }
            Ok(url) => {
                let ls_remote = git::run_output(repo, &["ls-remote", "--heads", "origin"]);
                let ok = ls_remote.is_ok();
                checks.push(DoctorCheck {
                    name: "remote_access".to_string(),
                    ok,
                    detail: if ok {
                        format!("remote origin accessible ({})", url)
                    } else {
                        format!("remote origin not accessible ({})", url)
                    },
                    fix: if ok {
                        None
                    } else {
                        Some("Check network access and credentials for the remote".to_string())
                    },
                });
            }
        }
    }

    output::print_doctor(&checks, mode);
    Ok(())
}
