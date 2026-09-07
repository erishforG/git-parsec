//! `parsec self-update` — check for a newer release and print upgrade instructions.
//!
//! # Phase 1 (this module)
//! Compares the running version against the latest GitHub release and prints
//! an upgrade command when a newer version is available.  No binary download
//! or in-place replacement is performed; that is deferred to Phase 2.

use anyhow::Result;
use serde::{Deserialize, Serialize};

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

// ── Startup version check (Phase 2) ────────────────────────────────────────

/// Cache filename stored in the OS cache directory (e.g. `~/.cache` on Linux).
const VERSION_CACHE_FILENAME: &str = ".parsec-version-check";
/// Minimum seconds between live GitHub API checks (24 h).
const VERSION_CHECK_THROTTLE_SECS: u64 = 86_400;
/// Network timeout for a startup background check (2 s — must feel instant).
const STARTUP_CHECK_TIMEOUT_SECS: u64 = 2;

/// Persisted state for the startup version-check throttle.
#[derive(Serialize, Deserialize, Default)]
struct VersionCheckCache {
    /// Unix epoch seconds of the last check attempt (successful or not).
    last_checked_secs: u64,
    /// Latest release tag returned by GitHub (e.g. `"v0.5.1"`).
    latest_tag: Option<String>,
}

fn version_cache_path() -> Option<std::path::PathBuf> {
    dirs::cache_dir().map(|d| d.join(VERSION_CACHE_FILENAME))
}

fn load_version_cache() -> VersionCheckCache {
    version_cache_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_version_cache(cache: &VersionCheckCache) {
    if let Some(path) = version_cache_path() {
        if let Ok(json) = serde_json::to_string(cache) {
            let _ = std::fs::write(path, json);
        }
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Returns `true` when `latest_tag` represents a version newer than the
/// running binary.  Extracted as a pure function for testability.
fn should_show_update_hint(latest_tag: Option<&str>) -> bool {
    latest_tag
        .map(|tag| cmp_semver(tag.trim_start_matches('v'), CURRENT_VERSION))
        .map(|ord| ord == std::cmp::Ordering::Greater)
        .unwrap_or(false)
}

fn print_update_hint(latest: &str) {
    eprintln!(
        "\n  ✦ parsec {latest} available (current: {CURRENT_VERSION})\
         \n    Run `parsec self-update` for upgrade instructions.\n"
    );
}

/// Print a one-line update hint to **stderr** if a newer release is available.
///
/// Throttles the live GitHub API call to at most once every 24 hours by
/// caching the result in [`version_cache_path()`].  Always a no-op in
/// offline mode; never panics.
///
/// Should be called after the main command has finished so it does not
/// interleave with command output.  Skipped in `--json` / `--quiet` mode
/// and for the `parsec self-update` command itself (see `src/cli/mod.rs`).
pub async fn startup_version_hint(offline: bool) {
    if offline {
        return;
    }

    let mut cache = load_version_cache();
    let now = now_secs();
    let age_secs = now.saturating_sub(cache.last_checked_secs);

    if age_secs >= VERSION_CHECK_THROTTLE_SECS {
        // Cache is stale — attempt a quick live refresh.
        let client = match reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(STARTUP_CHECK_TIMEOUT_SECS))
            .user_agent(format!("parsec/{CURRENT_VERSION}"))
            .build()
        {
            Ok(c) => c,
            Err(_) => return,
        };
        let url = format!("https://api.github.com/repos/{GITHUB_REPO}/releases/latest");
        match client.get(&url).send().await {
            Ok(resp) => {
                if let Ok(release) = resp.json::<GitHubRelease>().await {
                    let tag = release.tag_name.clone();
                    cache = VersionCheckCache {
                        last_checked_secs: now,
                        latest_tag: Some(tag.clone()),
                    };
                    save_version_cache(&cache);
                    if should_show_update_hint(Some(&tag)) {
                        print_update_hint(tag.trim_start_matches('v'));
                    }
                }
            }
            Err(_) => {
                // Network unavailable — bump timestamp to avoid hammering
                // the API on every run, but keep any cached latest_tag.
                cache.last_checked_secs = now;
                save_version_cache(&cache);
            }
        }
    } else if let Some(ref tag) = cache.latest_tag.clone() {
        // Use the cached result without a network call.
        if should_show_update_hint(Some(tag)) {
            print_update_hint(tag.trim_start_matches('v'));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        cmp_semver, now_secs, should_show_update_hint, VersionCheckCache, CURRENT_VERSION,
        VERSION_CHECK_THROTTLE_SECS,
    };
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

    // ── Phase 2: startup version-check helpers ──────────────────────────

    #[test]
    fn version_cache_serde_round_trip() {
        let cache = VersionCheckCache {
            last_checked_secs: 1_700_000_000,
            latest_tag: Some("v0.5.1".into()),
        };
        let json = serde_json::to_string(&cache).expect("serialize");
        let parsed: VersionCheckCache = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.last_checked_secs, 1_700_000_000);
        assert_eq!(parsed.latest_tag.as_deref(), Some("v0.5.1"));
    }

    #[test]
    fn version_cache_default_is_stale() {
        let cache = VersionCheckCache::default();
        let age = now_secs().saturating_sub(cache.last_checked_secs);
        assert!(
            age >= VERSION_CHECK_THROTTLE_SECS,
            "default cache should be stale"
        );
    }

    #[test]
    fn should_show_hint_newer_version() {
        // A tag strictly newer than the running CURRENT_VERSION should trigger a hint.
        // We use a version guaranteed to be newer than any cargo package version.
        assert!(should_show_update_hint(Some("v999.0.0")));
    }

    #[test]
    fn should_show_hint_older_version() {
        assert!(!should_show_update_hint(Some("v0.0.1")));
    }

    #[test]
    fn should_show_hint_none() {
        assert!(!should_show_update_hint(None));
    }

    #[test]
    fn should_show_hint_equal_version() {
        // Equal to current — no hint.
        assert!(!should_show_update_hint(Some(CURRENT_VERSION)));
    }
}
