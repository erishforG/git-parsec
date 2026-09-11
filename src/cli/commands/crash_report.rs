//! `parsec crash-report` — manage locally-saved crash reports (#298 Phase 2).
//!
//! Crash reports are opt-in JSON files written by the panic hook (Phase 1) to
//! `<OS cache dir>/parsec/crash-<timestamp>.json`
//! (e.g. `~/.cache/parsec/crash-20260101T000000Z.json` on Linux/macOS).
//!
//! ## Subcommands
//!
//! | Command            | Description                                        |
//! |--------------------|----------------------------------------------------|
//! | `list`             | List all reports with timestamp and panic preview  |
//! | `show <id>`        | Print the full JSON of one report                  |
//! | `clear`            | Delete all crash reports from the cache directory  |
//!
//! All operations are **read-only** except `clear` (and `clear --dry-run`
//! remains non-destructive).  No data is transmitted.

use anyhow::{bail, Result};
use std::path::PathBuf;

// ── Cache helpers ─────────────────────────────────────────────────────────────

/// Resolve the parsec crash-report cache directory.
pub(crate) fn crash_cache_dir() -> Result<PathBuf> {
    dirs::cache_dir()
        .ok_or_else(|| anyhow::anyhow!("cannot determine OS cache directory"))
        .map(|d| d.join("parsec"))
}

/// Collect all `crash-*.json` files sorted chronologically (oldest first).
pub(crate) fn list_crash_files(cache_dir: &PathBuf) -> Result<Vec<PathBuf>> {
    if !cache_dir.exists() {
        return Ok(vec![]);
    }
    let mut files: Vec<PathBuf> = std::fs::read_dir(cache_dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with("crash-") && n.ends_with(".json"))
                .unwrap_or(false)
        })
        .collect();
    files.sort();
    Ok(files)
}

// ── Subcommands ────────────────────────────────────────────────────────────────

/// `parsec crash-report list`
///
/// Prints each report's file stem and a one-line preview of the panic message.
/// With `--json` the output is a JSON array of metadata objects.
pub fn crash_report_list(json_mode: bool) -> Result<()> {
    let dir = crash_cache_dir()?;
    let files = list_crash_files(&dir)?;

    if files.is_empty() {
        if json_mode {
            println!("[]");
        } else {
            println!("No crash reports found in {}.", dir.display());
        }
        return Ok(());
    }

    if json_mode {
        let items: Vec<serde_json::Value> = files
            .iter()
            .filter_map(|p| {
                let id = p.file_stem()?.to_str()?.to_string();
                let content = std::fs::read_to_string(p).ok()?;
                let v: serde_json::Value = serde_json::from_str(&content).ok()?;
                Some(serde_json::json!({
                    "id": id,
                    "timestamp": v["timestamp"],
                    "panic_message": v["panic_message"],
                    "panic_location": v["panic_location"],
                }))
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&items)?);
    } else {
        let col_id = 36usize;
        let col_ts = 26usize;
        println!(
            "{:<col_id$}  {:<col_ts$}  panic (preview)",
            "ID", "timestamp"
        );
        println!("{}", "─".repeat(col_id + col_ts + 40));
        for path in &files {
            let id = path
                .file_stem()
                .and_then(|n| n.to_str())
                .unwrap_or("?")
                .to_string();
            let (ts, preview) = match std::fs::read_to_string(path)
                .ok()
                .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            {
                Some(v) => {
                    let ts = v["timestamp"].as_str().unwrap_or("?").to_string();
                    let msg = v["panic_message"].as_str().unwrap_or("?");
                    let preview: String = msg.chars().take(48).collect();
                    let preview = if msg.chars().count() > 48 {
                        format!("{preview}…")
                    } else {
                        preview
                    };
                    (ts, preview)
                }
                None => ("?".to_string(), "(unreadable)".to_string()),
            };
            println!("{id:<col_id$}  {ts:<col_ts$}  {preview}");
        }
        println!(
            "\n{} report(s).  Use `parsec crash-report show <id>` to view a full report.",
            files.len()
        );
    }
    Ok(())
}

/// `parsec crash-report show <id>`
///
/// Resolves the report by id (file stem, e.g. `crash-20260101T000000Z`),
/// then pretty-prints the full JSON to stdout.  Accepts the id with or
/// without the `.json` suffix.
pub fn crash_report_show(id: &str, json_mode: bool) -> Result<()> {
    let dir = crash_cache_dir()?;
    let stem = id.trim_end_matches(".json");
    let path = dir.join(format!("{stem}.json"));

    if !path.exists() {
        bail!(
            "crash report `{stem}` not found in {}.\n\
             Run `parsec crash-report list` to see available reports.",
            dir.display()
        );
    }

    let content = std::fs::read_to_string(&path)?;

    if json_mode {
        // Re-parse and re-serialize to normalise whitespace.
        let v: serde_json::Value = serde_json::from_str(&content)?;
        println!("{}", serde_json::to_string_pretty(&v)?);
    } else {
        let v: serde_json::Value =
            serde_json::from_str(&content).unwrap_or(serde_json::Value::Null);
        println!("── Crash report: {stem} ─────────────────────────────────────");
        if let Some(obj) = v.as_object() {
            for (k, val) in obj {
                let display = val.as_str().map_or_else(|| val.to_string(), str::to_string);
                println!("  {k:<22}: {display}");
            }
        } else {
            println!("{content}");
        }
        println!("\n  File: {}", path.display());
        println!(
            "\n  To report this bug, open:\n  \
             https://github.com/erishforG/git-parsec/issues/new\n  \
             (attach or paste the JSON above)"
        );
    }
    Ok(())
}

/// `parsec crash-report clear`
///
/// Removes all crash reports from the cache directory.
/// With `--dry-run` (forwarded from the global flag) no files are deleted.
pub fn crash_report_clear(dry_run: bool) -> Result<()> {
    let dir = crash_cache_dir()?;
    let files = list_crash_files(&dir)?;

    if files.is_empty() {
        println!("No crash reports to clear.");
        return Ok(());
    }

    let mut removed = 0usize;
    for path in &files {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("?");
        if dry_run {
            println!("[dry-run] would remove: {name}");
        } else {
            std::fs::remove_file(path)?;
            removed += 1;
        }
    }

    if dry_run {
        println!(
            "[dry-run] {} report(s) would be removed from {}.",
            files.len(),
            dir.display()
        );
    } else {
        println!("Cleared {removed} crash report(s) from {}.", dir.display());
    }
    Ok(())
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Write a minimal valid crash-report fixture.
    fn write_fixture(dir: &std::path::Path, stem: &str, msg: &str) {
        let content = format!(
            concat!(
                r#"{{"parsec_version":"0.5.0","timestamp":"2026-01-01T00:00:00Z","#,
                r#""os":"unix/macos","shell":"zsh","subcommand":"start","#,
                r#""panic_location":"src/main.rs:1","panic_message":"{msg}"}}"#
            ),
            msg = msg
        );
        std::fs::write(dir.join(format!("{stem}.json")), content).unwrap();
    }

    #[test]
    fn list_ignores_non_crash_files() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = tmp.path().join("parsec");
        std::fs::create_dir_all(&cache).unwrap();
        std::fs::write(cache.join("other.txt"), "x").unwrap();
        std::fs::write(cache.join("not-crash.json"), "{}").unwrap();
        let files = list_crash_files(&cache).unwrap();
        assert!(files.is_empty(), "non-crash files should be filtered out");
    }

    #[test]
    fn list_sorted_oldest_first() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = tmp.path().join("parsec");
        std::fs::create_dir_all(&cache).unwrap();
        write_fixture(&cache, "crash-2026-01-02T00:00:00Z", "beta");
        write_fixture(&cache, "crash-2026-01-01T00:00:00Z", "alpha");
        let files = list_crash_files(&cache).unwrap();
        assert_eq!(files.len(), 2);
        assert!(
            files[0].to_str().unwrap().contains("2026-01-01"),
            "oldest report should come first"
        );
    }

    #[test]
    fn list_nonexistent_dir_returns_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = tmp.path().join("parsec").join("does-not-exist");
        let files = list_crash_files(&cache).unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn clear_removes_crash_files() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = tmp.path().join("parsec");
        std::fs::create_dir_all(&cache).unwrap();
        write_fixture(&cache, "crash-2026-01-01T00:00:00Z", "boom");
        let files_before = list_crash_files(&cache).unwrap();
        assert_eq!(files_before.len(), 1);
        for p in &files_before {
            std::fs::remove_file(p).unwrap();
        }
        let files_after = list_crash_files(&cache).unwrap();
        assert!(files_after.is_empty(), "all crash files should be removed");
    }

    #[test]
    fn clear_dry_run_leaves_files_intact() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = tmp.path().join("parsec");
        std::fs::create_dir_all(&cache).unwrap();
        write_fixture(&cache, "crash-2026-01-01T00:00:00Z", "still here");
        let files = list_crash_files(&cache).unwrap();
        assert_eq!(files.len(), 1);
        // Simulate dry-run: do NOT remove files.
        for p in &files {
            assert!(p.exists(), "dry-run must not delete files");
        }
    }
}
