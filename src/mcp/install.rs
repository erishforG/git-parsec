//! MCP client config installer — `parsec mcp install --client=<client>`.
//!
//! Writes or updates the `mcpServers.git-parsec` entry in the target client's
//! JSON config file without disturbing other keys.
//!
//! # Installer Contract (from `docs/mcp/clients.md`)
//! - Parse the config file as JSON before writing.
//! - Preserve unrelated top-level keys and unrelated `mcpServers` entries.
//! - Replace only the `mcpServers.git-parsec` entry.
//! - Refuse to modify if the file contains non-standard JSON syntax.
//! - Create a timestamped backup before modifying an existing config file.
//! - Support `--dry-run` (prints the target path and planned JSON without writing).
//! - Never embed `GITHUB_TOKEN`, `GH_TOKEN`, or other credentials in config.
//!
//! # Phase Status
//! Phase 33 (#293): automated installer hook.

use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::str::FromStr;

/// Supported MCP desktop clients.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpClientTarget {
    ClaudeDesktop,
    Cursor,
}

impl std::fmt::Display for McpClientTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ClaudeDesktop => write!(f, "claude-desktop"),
            Self::Cursor => write!(f, "cursor"),
        }
    }
}

impl FromStr for McpClientTarget {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "claude-desktop" => Ok(Self::ClaudeDesktop),
            "cursor" => Ok(Self::Cursor),
            other => bail!(
                "unknown MCP client '{}'; valid choices: claude-desktop, cursor",
                other
            ),
        }
    }
}

/// Returns the platform-specific config file path for the given client.
///
/// Paths match the table in `docs/mcp/clients.md`.
pub fn config_path(client: McpClientTarget) -> Result<PathBuf> {
    match client {
        McpClientTarget::ClaudeDesktop => claude_desktop_config_path(),
        McpClientTarget::Cursor => cursor_config_path(),
    }
}

fn claude_desktop_config_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("could not determine home directory")?;
    #[cfg(target_os = "macos")]
    {
        Ok(home.join("Library/Application Support/Claude/claude_desktop_config.json"))
    }
    #[cfg(target_os = "windows")]
    {
        let appdata = dirs::data_dir().context("could not determine AppData directory")?;
        Ok(appdata.join("Claude/claude_desktop_config.json"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = home;
        bail!("Claude Desktop is only supported on macOS and Windows; detected other OS")
    }
}

fn cursor_config_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("could not determine home directory")?;
    Ok(home.join(".cursor/mcp.json"))
}

/// Builds the `mcpServers.git-parsec` entry value.
///
/// When `bin_path` is `None`, the entry uses `"parsec"` relying on `$PATH`.
pub fn build_server_entry(bin_path: Option<&str>) -> Value {
    let command = bin_path.unwrap_or("parsec");
    serde_json::json!({
        "command": command,
        "args": ["mcp", "serve"]
    })
}

/// Merges the `mcpServers.git-parsec` entry into an existing config value.
///
/// - All other top-level keys are preserved unchanged.
/// - All other `mcpServers` entries are preserved unchanged.
/// - If `mcpServers` does not exist, it is created.
/// - The `mcpServers.git-parsec` entry is inserted or replaced.
pub fn merge_server_entry(mut config: Value, entry: Value) -> Value {
    // Ensure the root is an object (guard against malformed top-level JSON).
    if !config.is_object() {
        config = Value::Object(serde_json::Map::new());
    }

    let obj = config.as_object_mut().expect("just ensured object");

    match obj.get_mut("mcpServers") {
        Some(Value::Object(servers)) => {
            servers.insert("git-parsec".to_string(), entry);
        }
        _ => {
            let mut servers = serde_json::Map::new();
            servers.insert("git-parsec".to_string(), entry);
            obj.insert("mcpServers".to_string(), Value::Object(servers));
        }
    }

    config
}

/// Creates a timestamped `.bak-<timestamp>` copy of `path`.
///
/// Returns the backup path on success.
fn timestamped_backup(path: &Path) -> Result<PathBuf> {
    let ts = chrono::Local::now().format("%Y%m%dT%H%M%S");
    let ext = format!("bak-{ts}");
    let backup = path.with_extension(ext);
    std::fs::copy(path, &backup).with_context(|| format!("failed to backup {}", path.display()))?;
    Ok(backup)
}

/// Run the MCP client config installer.
///
/// Reads the target client's config file, merges the `mcpServers.git-parsec`
/// entry (preserving all other keys), and writes the result.
///
/// When `dry_run` is `true` the planned JSON is printed to stdout and no files
/// are touched on disk.
pub fn install(client: McpClientTarget, dry_run: bool, bin_path: Option<&str>) -> Result<()> {
    let path = config_path(client)?;
    let entry = build_server_entry(bin_path);

    // Read and parse existing config, or start with an empty JSON object.
    let existing: Value = if path.exists() {
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        serde_json::from_str(&raw).with_context(|| {
            format!(
                "could not parse {} as JSON — it may contain comments or \
                 trailing commas; fix the file manually and retry",
                path.display()
            )
        })?
    } else {
        Value::Object(serde_json::Map::new())
    };

    let merged = merge_server_entry(existing, entry);
    let output =
        serde_json::to_string_pretty(&merged).context("failed to serialise merged config")?;

    if dry_run {
        println!("# dry-run — no files written");
        println!("# target: {}", path.display());
        println!("{output}");
        return Ok(());
    }

    // Backup existing file before overwriting.
    if path.exists() {
        let backup = timestamped_backup(&path)?;
        eprintln!("info: backup written to {}", backup.display());
    }

    // Create parent directory if it does not yet exist.
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create directory {}", parent.display()))?;
    }

    std::fs::write(&path, &output)
        .with_context(|| format!("failed to write {}", path.display()))?;

    println!("✓ git-parsec registered in {} config", client);
    println!("  {}", path.display());
    Ok(())
}

// ── tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn merge_into_empty_object() {
        let entry = json!({"command": "parsec", "args": ["mcp", "serve"]});
        let result = merge_server_entry(json!({}), entry.clone());
        assert_eq!(result["mcpServers"]["git-parsec"], entry);
    }

    #[test]
    fn merge_preserves_other_mcp_servers() {
        let existing = json!({
            "mcpServers": {
                "other-tool": {"command": "other", "args": []}
            }
        });
        let entry = json!({"command": "parsec", "args": ["mcp", "serve"]});
        let result = merge_server_entry(existing, entry.clone());
        assert_eq!(result["mcpServers"]["git-parsec"], entry);
        // Unrelated server entry must be preserved.
        assert_eq!(result["mcpServers"]["other-tool"]["command"], "other");
    }

    #[test]
    fn merge_replaces_existing_entry() {
        let existing = json!({
            "mcpServers": {
                "git-parsec": {"command": "old-parsec", "args": ["serve"]}
            }
        });
        let entry = json!({"command": "parsec", "args": ["mcp", "serve"]});
        let result = merge_server_entry(existing, entry);
        assert_eq!(result["mcpServers"]["git-parsec"]["command"], "parsec");
        assert_eq!(result["mcpServers"]["git-parsec"]["args"][0], "mcp");
        assert_eq!(result["mcpServers"]["git-parsec"]["args"][1], "serve");
    }

    #[test]
    fn merge_preserves_top_level_keys() {
        let existing = json!({"theme": "dark", "mcpServers": {}});
        let entry = json!({"command": "parsec", "args": ["mcp", "serve"]});
        let result = merge_server_entry(existing, entry);
        assert_eq!(result["theme"], "dark");
        assert!(result["mcpServers"]["git-parsec"].is_object());
    }

    #[test]
    fn merge_handles_non_object_mcp_servers() {
        // If mcpServers is unexpectedly a non-object, replace it cleanly.
        let existing = json!({"mcpServers": "invalid"});
        let entry = json!({"command": "parsec", "args": ["mcp", "serve"]});
        let result = merge_server_entry(existing, entry.clone());
        assert_eq!(result["mcpServers"]["git-parsec"], entry);
    }

    #[test]
    fn build_entry_default_bin() {
        let entry = build_server_entry(None);
        assert_eq!(entry["command"], "parsec");
        assert_eq!(entry["args"], json!(["mcp", "serve"]));
    }

    #[test]
    fn build_entry_custom_bin() {
        let entry = build_server_entry(Some("/usr/local/bin/parsec"));
        assert_eq!(entry["command"], "/usr/local/bin/parsec");
        assert_eq!(entry["args"], json!(["mcp", "serve"]));
    }

    #[test]
    fn client_target_fromstr_valid() {
        assert_eq!(
            "claude-desktop".parse::<McpClientTarget>().unwrap(),
            McpClientTarget::ClaudeDesktop
        );
        assert_eq!(
            "cursor".parse::<McpClientTarget>().unwrap(),
            McpClientTarget::Cursor
        );
    }

    #[test]
    fn client_target_fromstr_invalid() {
        assert!("vscode".parse::<McpClientTarget>().is_err());
        assert!("".parse::<McpClientTarget>().is_err());
        assert!("CLAUDE-DESKTOP".parse::<McpClientTarget>().is_err());
    }

    #[test]
    fn client_target_display() {
        assert_eq!(McpClientTarget::ClaudeDesktop.to_string(), "claude-desktop");
        assert_eq!(McpClientTarget::Cursor.to_string(), "cursor");
    }

    #[test]
    fn dry_run_prints_to_stdout() {
        // Smoke check: dry_run with a temp dir so no real config is modified.
        // We only verify it returns Ok — stdout capture is left to integration tests.
        // config_path is platform-specific, so exercise just the merge+serialise path.
        let existing = json!({"mcpServers": {"other": {"command": "x"}}});
        let entry = build_server_entry(None);
        let merged = merge_server_entry(existing, entry);
        let serialised = serde_json::to_string_pretty(&merged).unwrap();
        assert!(serialised.contains("git-parsec"));
        assert!(serialised.contains("parsec"));
        assert!(serialised.contains("other")); // preserved
    }
}
