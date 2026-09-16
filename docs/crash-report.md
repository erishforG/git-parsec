# Crash Report Privacy Policy

`parsec` includes an **opt-in** crash report feature that saves a local JSON
file when an unexpected panic occurs.  No data is ever sent automatically.

## What is collected

When `[crash_report] enabled = true` and a panic happens, parsec writes a
`crash-<timestamp>.json` file under the platform cache directory:

| Platform | Directory |
|---|---|
| Linux | `$XDG_CACHE_HOME/parsec/`, or `~/.cache/parsec/` when unset |
| macOS | `~/Library/Caches/parsec/` |
| Windows | `%LOCALAPPDATA%\parsec\` |

The exact directory is selected by [`dirs::cache_dir()`](https://docs.rs/dirs/latest/dirs/fn.cache_dir.html).
Each report contains:

| Field | Example | Notes |
|---|---|---|
| `parsec_version` | `"0.5.0"` | Binary version |
| `timestamp` | `"2026-09-10T09:00:00Z"` | UTC ISO-8601 |
| `os` | `"unix/macos"` | Family + OS name |
| `shell` | `"zsh"` | `$SHELL` basename only |
| `subcommand` | `"ship"` | `argv[1]` only, no flags or ticket IDs |
| `panic_location` | `"src/cli/commands/ship.rs:42"` | Source file + line |
| `panic_message` | `"attempt to subtract with overflow"` | Panic payload (≤ 512 chars) |

### What is NOT collected

- Your ticket IDs, branch names, commit messages, or file contents
- Your GitHub token, Jira token, or any credentials
- Your full command-line arguments or environment variables
- Network-level information (IP address, hostname)
- A stack trace (use `RUST_BACKTRACE=1` locally for that)

## Opt-in

Add to `~/.config/parsec/config.toml`:

```toml
[crash_report]
enabled = true
```

The default is `enabled = false` — **nothing is saved unless you opt in**.

## How to share a report

If you experience a crash and want to help:

1. Run `parsec crash-report list` to find the report ID
2. Review it with `parsec crash-report show <id>` (it is plain JSON)
3. Open a GitHub issue: <https://github.com/erishforG/git-parsec/issues/new>
4. Paste or attach the file

You are never required to share a crash report.

## Retention

Reports stay in the platform cache directory until you remove them. Preview a
cleanup first, then delete all saved reports with:

```sh
parsec --dry-run crash-report clear
parsec crash-report clear
```
