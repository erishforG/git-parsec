//! `parsec` panic hook — opt-in crash report collection (#298).
//!
//! # Phase 1
//! Registers a custom panic hook that:
//! 1. Prints a user-friendly crash banner to **stderr** (always).
//! 2. When `enabled = true` (opt-in), writes a structured JSON report to
//!    `~/.cache/parsec/crash-<timestamp>.json` so users can share it
//!    with the maintainers.
//!
//! No data is **transmitted** automatically.  The user must opt in via config
//! and manually share the file.  See `docs/crash-report.md` for the full
//! privacy policy.
//!
//! # Report Contents
//! - `parsec_version` — crate version from `Cargo.toml`
//! - `timestamp`      — ISO-8601 UTC timestamp of the crash
//! - `os`             — `<family>/<os>` (e.g. `"unix/macos"`)
//! - `shell`          — value of `$SHELL` (sanitised, may be absent)
//! - `command_args`   — `argv[1..]` (first arg only, not flags, for privacy)
//! - `panic_message`  — the panic payload as a string (may be truncated)
//! - `panic_location` — `"file:line"` where the panic occurred
//!
//! The full back-trace is **not** collected by default; set the
//! `RUST_BACKTRACE=1` environment variable for a local stack trace.

use std::panic;
use std::sync::atomic::{AtomicBool, Ordering};

static REPORT_ENABLED: AtomicBool = AtomicBool::new(false);

const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Call this once at startup, before any `tokio` threads are spawned.
///
/// When `enabled` is `true` the hook will also write a JSON crash report to
/// the OS cache directory (`~/.cache/parsec/` on Linux/macOS).
pub fn setup(enabled: bool) {
    REPORT_ENABLED.store(enabled, Ordering::SeqCst);

    panic::set_hook(Box::new(|info| {
        // ── Always: human-readable crash banner ────────────────────────────
        let location = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "<unknown>".to_string());

        let message = format_panic_message(info);

        eprintln!();
        eprintln!("╔══════════════════════════════════════════════════════╗");
        eprintln!("║  parsec crashed — sorry about that!                 ║");
        eprintln!("╚══════════════════════════════════════════════════════╝");
        eprintln!();
        eprintln!("  version  : parsec {CURRENT_VERSION}");
        eprintln!("  location : {location}");
        eprintln!("  message  : {message}");
        eprintln!();
        eprintln!("  For a stack trace, re-run with RUST_BACKTRACE=1.");

        // ── Opt-in: write JSON report ───────────────────────────────────────
        if REPORT_ENABLED.load(Ordering::SeqCst) {
            let report = build_report(&location, &message);
            match save_report(&report) {
                Ok(path) => {
                    eprintln!("  Crash report saved to: {path}");
                    eprintln!(
                        "  To report this bug, open:\n  \
                         https://github.com/erishforG/git-parsec/issues/new"
                    );
                }
                Err(e) => {
                    eprintln!("  (crash report write failed: {e})");
                }
            }
        } else {
            eprintln!(
                "  Tip: enable crash reports with `[crash_report] enabled = true` in\n  \
                 your parsec config to help diagnose issues.  See docs/crash-report.md."
            );
        }

        eprintln!();
    }));
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Extract the panic payload as a trimmed, ASCII-safe string (≤ 512 chars).
fn format_panic_message(info: &panic::PanicHookInfo<'_>) -> String {
    let raw = if let Some(s) = info.payload().downcast_ref::<&str>() {
        s.to_string()
    } else if let Some(s) = info.payload().downcast_ref::<String>() {
        s.clone()
    } else {
        "<non-string panic payload>".to_string()
    };
    // Truncate so the crash banner stays readable on narrow terminals.
    let truncated = raw.chars().take(512).collect::<String>();
    if truncated.len() < raw.len() {
        format!("{truncated}…")
    } else {
        truncated
    }
}

/// Collect the crash report as a JSON string.
fn build_report(location: &str, message: &str) -> String {
    let timestamp = {
        // Use SystemTime → RFC-3339-ish without chrono (already a dep but
        // keep this helper self-contained and allocation-light).
        use std::time::{SystemTime, UNIX_EPOCH};
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        // Minimal ISO-8601: "YYYY-MM-DDTHH:MM:SSZ" via integer maths.
        let s = secs;
        let (y, mo, d, h, mi, se) = unix_to_utc_fields(s);
        format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{se:02}Z")
    };

    let os_family = std::env::consts::FAMILY; // "unix" | "windows"
    let os_name = std::env::consts::OS; // "macos" | "linux" | "windows" …

    let shell = std::env::var("SHELL")
        .ok()
        .map(|s| {
            // Keep only the base name for privacy (e.g. "/usr/bin/zsh" → "zsh").
            std::path::Path::new(&s)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string()
        })
        .unwrap_or_else(|| "unknown".to_string());

    // Capture argv[1] only (sub-command name) — no user-supplied arguments.
    let subcommand = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "<none>".to_string());

    // Escape double-quotes for the naive JSON builder below.
    let esc = |s: &str| s.replace('\\', r"\\").replace('"', r#"\""#);

    format!(
        r#"{{
  "parsec_version": "{ver}",
  "timestamp": "{ts}",
  "os": "{fam}/{os}",
  "shell": "{shell}",
  "subcommand": "{sub}",
  "panic_location": "{loc}",
  "panic_message": "{msg}"
}}"#,
        ver = esc(CURRENT_VERSION),
        ts = esc(&timestamp),
        fam = esc(os_family),
        os = esc(os_name),
        shell = esc(&shell),
        sub = esc(&subcommand),
        loc = esc(location),
        msg = esc(message),
    )
}

/// Persist the report JSON to the OS cache dir and return the file path.
fn save_report(json: &str) -> std::io::Result<String> {
    use std::time::{SystemTime, UNIX_EPOCH};

    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let cache_dir = dirs::cache_dir()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no cache dir"))?
        .join("parsec");

    std::fs::create_dir_all(&cache_dir)?;

    let file_name = format!("crash-{ts}.json");
    let path = cache_dir.join(&file_name);
    std::fs::write(&path, json)?;
    Ok(path.to_string_lossy().into_owned())
}

/// Minimal UTC date/time decomposition from a Unix timestamp.
///
/// Intentionally avoids the `chrono` dep (even though it is available) to
/// keep the panic hook free of code that could itself panic.  The algorithm
/// covers dates from the Unix epoch through ≥ 2100.
fn unix_to_utc_fields(mut secs: u64) -> (u32, u32, u32, u32, u32, u32) {
    let se = (secs % 60) as u32;
    secs /= 60;
    let mi = (secs % 60) as u32;
    secs /= 60;
    let h = (secs % 24) as u32;
    secs /= 24;

    // Days since 1970-01-01 → Gregorian calendar.
    // Algorithm: civil_from_days from https://howardhinnant.github.io/date_algorithms.html
    let z = secs as i64 + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if mo <= 2 { y + 1 } else { y } as u32;

    (y, mo, d, h, mi, se)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_to_utc_known_dates() {
        // 2024-01-15 11:50:45 UTC = 1705319445
        // Verified: python3 -c "import datetime; print(datetime.datetime.utcfromtimestamp(1705319445))"
        let (y, mo, d, h, mi, se) = unix_to_utc_fields(1_705_319_445);
        assert_eq!((y, mo, d), (2024, 1, 15));
        assert_eq!((h, mi, se), (11, 50, 45));
    }

    #[test]
    fn unix_to_utc_epoch() {
        let (y, mo, d, h, mi, se) = unix_to_utc_fields(0);
        assert_eq!((y, mo, d), (1970, 1, 1));
        assert_eq!((h, mi, se), (0, 0, 0));
    }

    #[test]
    fn unix_to_utc_leap_day() {
        // 2024-02-29 00:00:00 UTC = 1709164800
        let (y, mo, d, _h, _mi, _se) = unix_to_utc_fields(1_709_164_800);
        assert_eq!((y, mo, d), (2024, 2, 29));
    }

    #[test]
    fn build_report_contains_required_fields() {
        let json = build_report("src/main.rs:42", "test panic");
        assert!(json.contains("\"parsec_version\""), "version field");
        assert!(json.contains("\"timestamp\""), "timestamp field");
        assert!(json.contains("\"os\""), "os field");
        assert!(json.contains("\"shell\""), "shell field");
        assert!(json.contains("\"panic_location\""), "location field");
        assert!(json.contains("\"panic_message\""), "message field");
        assert!(json.contains("src/main.rs:42"), "location value");
        assert!(json.contains("test panic"), "message value");
    }

    #[test]
    fn build_report_escapes_special_chars() {
        let json = build_report("file.rs:1", r#"something "quoted" and \slashed"#);
        // Must be valid-ish JSON: no raw unescaped double-quotes in values.
        assert!(json.contains(r#"\"quoted\""#), "quotes escaped: {json}");
        assert!(json.contains(r"\\slashed"), "backslash escaped: {json}");
    }

    #[test]
    fn setup_does_not_panic() {
        // Calling setup a second time must not crash.  (The previous hook is
        // replaced; that's expected behaviour for our use-case.)
        setup(false);
        setup(false);
    }
}
