//! `parsec self-update` — check for a newer release and print upgrade instructions.
//!
//! # Phase 1 (this module)
//! Compares the running version against the latest GitHub release and prints
//! an upgrade command when a newer version is available.  No binary download
//! or in-place replacement is performed; that is deferred to Phase 2.

use anyhow::Result;
use serde::Deserialize;

const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const GITHUB_REPO: &str = "erishforG/git-parsec";
/// HTTP timeout for the GitHub releases API call (seconds).
const CHECK_TIMEOUT_SECS: u64 = 8;

#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
    html_url: String,
    body: Option<String>,
}

/// Compare two semver strings of the form `"X.Y.Z"` or `"vX.Y.Z"`.
///
/// Each component is compared numerically so `"0.10.0"` sorts after `"0.9.0"`,
/// unlike a plain lexicographic comparison.
fn cmp_semver(a: &str, b: &str) -> std::cmp::Ordering {
    let parse = |s: &str| -> (u64, u64, u64) {
        let s = s.trim_start_matches('v');
        let mut it = s.splitn(3, '.').map(|p| p.parse::<u64>().unwrap_or(0));
        (
            it.next().unwrap_or(0),
            it.next().unwrap_or(0),
            it.next().unwrap_or(0),
        )
    };
    parse(a).cmp(&parse(b))
}

/// Fetch the latest release metadata from the GitHub releases API.
async fn fetch_latest_release() -> anyhow::Result<GitHubRelease> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(CHECK_TIMEOUT_SECS))
        .user_agent(format!("parsec/{CURRENT_VERSION}"))
        .build()?;
    let url = format!("https://api.github.com/repos/{GITHUB_REPO}/releases/latest");
    let release = client
        .get(&url)
        .send()
        .await?
        .error_for_status()?
        .json::<GitHubRelease>()
        .await?;
    Ok(release)
}

/// `parsec self-update` entry point.
///
/// When `offline` is `true` (either `--offline` flag or config), the network
/// call is skipped and only the current version is printed.
pub async fn self_update(offline: bool) -> Result<()> {
    let current = CURRENT_VERSION;

    if offline {
        println!("parsec {current}");
        println!("note: version check skipped (--offline)");
        return Ok(());
    }

    use std::io::Write as _;
    print!("parsec {current}  →  checking for updates… ");
    let _ = std::io::stdout().flush();

    match fetch_latest_release().await {
        Err(e) => {
            println!("(network unavailable: {e:#})");
            println!("Current version: parsec {current}");
            println!("See https://github.com/{GITHUB_REPO}/releases for the latest release.");
        }
        Ok(release) => {
            let latest_tag = &release.tag_name;
            let latest = latest_tag.trim_start_matches('v');
            match cmp_semver(latest, current) {
                std::cmp::Ordering::Greater => {
                    println!("update available!\n");
                    println!("  {current}  →  {latest}");
                    println!("  {}", release.html_url);
                    // Show a brief excerpt of the release notes (up to 4 lines).
                    if let Some(notes) = &release.body {
                        let preview: String = notes.lines().take(4).collect::<Vec<_>>().join("\n");
                        if !preview.trim().is_empty() {
                            println!("\n  Release notes (preview):");
                            for line in preview.lines() {
                                println!("    {line}");
                            }
                        }
                    }
                    println!("\nTo upgrade:");
                    println!(
                        "  cargo install --git https://github.com/{GITHUB_REPO} \
                         --bin parsec --force"
                    );
                    println!(
                        "\nnote: automated binary replacement is planned for Phase 2 \
                         (see issue #296)."
                    );
                }
                std::cmp::Ordering::Equal => {
                    println!("✓  already up to date ({current})");
                }
                std::cmp::Ordering::Less => {
                    // The user is running a dev build ahead of the published release.
                    println!(
                        "✓  {latest} is the latest published release \
                         (you are ahead — development build)"
                    );
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::cmp_semver;
    use std::cmp::Ordering;

    #[test]
    fn newer_patch() {
        assert_eq!(cmp_semver("0.5.1", "0.5.0"), Ordering::Greater);
    }

    #[test]
    fn newer_minor_double_digit() {
        // Numeric comparison: "0.10.0" > "0.9.0"; lexicographic would fail.
        assert_eq!(cmp_semver("0.10.0", "0.9.0"), Ordering::Greater);
    }

    #[test]
    fn newer_major() {
        assert_eq!(cmp_semver("1.0.0", "0.5.0"), Ordering::Greater);
    }

    #[test]
    fn equal_plain() {
        assert_eq!(cmp_semver("0.5.0", "0.5.0"), Ordering::Equal);
    }

    #[test]
    fn v_prefix_stripped() {
        assert_eq!(cmp_semver("v1.2.3", "1.2.3"), Ordering::Equal);
        assert_eq!(cmp_semver("v2.0.0", "v1.9.9"), Ordering::Greater);
    }

    #[test]
    fn older() {
        assert_eq!(cmp_semver("0.4.0", "0.5.0"), Ordering::Less);
    }

    #[test]
    fn multi_digit_major() {
        assert_eq!(cmp_semver("10.0.0", "9.99.99"), Ordering::Greater);
    }
}
