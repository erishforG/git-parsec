//! `parsec test` — run tests inside parsec-managed worktrees (issue #247).
//!
//! Runs a configurable shell command (default: `cargo test`) inside a single
//! worktree or across every active worktree, with optional parallelism and
//! tree-hash result caching.
//!
//! Selection logic (in order):
//! 1. `--all`        → all active worktrees
//! 2. `ticket`       → the named worktree only
//! 3. auto-detect    → the worktree whose path contains `cwd`
//!
//! Caching: when `--cache` is set, the test result for each worktree is keyed
//! by the worktree's `git rev-parse HEAD^{tree}` output and stored under
//! `<repo>/.parsec/test-cache/<tree-hash>.json`. Only successful (`exit 0`)
//! runs are cached.

use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::config::ParsecConfig;
use crate::git;
use crate::output::{self, Mode, TestResult};
use crate::worktree::{Workspace, WorktreeManager};

/// On-disk cache entry for a single worktree+tree-hash combination.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheEntry {
    tree_hash: String,
    exit_code: i32,
    duration_ms: u64,
    stdout_tail: String,
}

/// Run `parsec test` against one or more worktrees.
pub async fn test(
    repo: &Path,
    ticket: Option<&str>,
    all: bool,
    jobs: usize,
    cache: bool,
    command_override: Option<&str>,
    mode: Mode,
) -> Result<()> {
    let config = ParsecConfig::load()?;
    let manager = WorktreeManager::new(repo, &config)?;

    // Effective settings: CLI flags override config values.
    let command = command_override
        .map(|s| s.to_string())
        .unwrap_or_else(|| config.test.command.clone());
    let jobs = if jobs == 0 { config.test.jobs } else { jobs };
    let jobs = jobs.max(1);
    let cache_enabled = cache || config.test.cache;

    // Resolve target workspaces.
    let workspaces = if all {
        let ws = manager.list()?;
        if ws.is_empty() {
            anyhow::bail!("no active workspaces to test");
        }
        ws
    } else if let Some(t) = ticket {
        vec![manager.get(t)?]
    } else {
        let cwd = std::env::current_dir()?;
        let all_ws = manager.list()?;
        let found = all_ws
            .into_iter()
            .find(|w| cwd.starts_with(&w.path))
            .ok_or_else(|| {
                anyhow::anyhow!("not inside a parsec worktree. Specify a ticket or use --all.")
            })?;
        vec![found]
    };

    let cache_dir = manager.repo_root().join(".parsec").join("test-cache");
    if cache_enabled {
        std::fs::create_dir_all(&cache_dir).with_context(|| {
            format!(
                "failed to create test cache directory: {}",
                cache_dir.display()
            )
        })?;
    }

    let results = if jobs > 1 && workspaces.len() > 1 {
        run_parallel(workspaces, command, cache_enabled, &cache_dir, jobs).await
    } else {
        run_sequential(workspaces, command, cache_enabled, &cache_dir).await
    };

    let any_failed = results.iter().any(|r| r.exit_code != 0);
    output::print_test_results(&results, mode);

    if any_failed {
        // Propagate non-zero exit via ParsecError to keep the existing
        // error pipeline consistent. The first failing exit code is used.
        let first_fail = results
            .iter()
            .find(|r| r.exit_code != 0)
            .map(|r| r.exit_code)
            .unwrap_or(1);
        anyhow::bail!("one or more worktree tests failed (exit_code={first_fail})");
    }

    Ok(())
}

/// Run each workspace's command sequentially.
async fn run_sequential(
    workspaces: Vec<Workspace>,
    command: String,
    cache: bool,
    cache_dir: &Path,
) -> Vec<TestResult> {
    let mut out = Vec::with_capacity(workspaces.len());
    for ws in workspaces {
        out.push(run_one(ws, command.clone(), cache, cache_dir.to_path_buf()).await);
    }
    out
}

/// Run workspaces in parallel using a tokio `JoinSet` with a semaphore-bounded
/// concurrency limit equal to `jobs`.
async fn run_parallel(
    workspaces: Vec<Workspace>,
    command: String,
    cache: bool,
    cache_dir: &Path,
    jobs: usize,
) -> Vec<TestResult> {
    let semaphore = std::sync::Arc::new(Semaphore::new(jobs));
    let mut set: JoinSet<TestResult> = JoinSet::new();
    let cache_dir = cache_dir.to_path_buf();
    for ws in workspaces {
        let sem = semaphore.clone();
        let cmd = command.clone();
        let cdir = cache_dir.clone();
        set.spawn(async move {
            let _permit = sem.acquire().await.expect("semaphore closed");
            run_one(ws, cmd, cache, cdir).await
        });
    }
    let mut out = Vec::new();
    while let Some(joined) = set.join_next().await {
        match joined {
            Ok(r) => out.push(r),
            Err(e) => out.push(TestResult {
                ticket: "<unknown>".to_string(),
                exit_code: 1,
                duration_ms: 0,
                from_cache: false,
                stdout_tail: format!("task join error: {e}"),
            }),
        }
    }
    // Stable order by ticket for deterministic output.
    out.sort_by(|a, b| a.ticket.cmp(&b.ticket));
    out
}

/// Run the test command for a single workspace, consulting / updating the
/// cache when enabled. Always returns a [`TestResult`] (never panics).
async fn run_one(ws: Workspace, command: String, cache: bool, cache_dir: PathBuf) -> TestResult {
    let tree_hash = git::run_output(&ws.path, &["rev-parse", "HEAD^{tree}"])
        .map(|s| s.trim().to_string())
        .unwrap_or_default();

    if cache && !tree_hash.is_empty() {
        if let Some(entry) = load_cache(&cache_dir, &tree_hash) {
            return TestResult {
                ticket: ws.ticket,
                exit_code: entry.exit_code,
                duration_ms: entry.duration_ms,
                from_cache: true,
                stdout_tail: entry.stdout_tail,
            };
        }
    }

    let started = Instant::now();
    let ws_path = ws.path.clone();
    let cmd_str = command.clone();
    let exec = tokio::task::spawn_blocking(move || {
        // Cross-platform shell: cmd.exe on Windows (bash on Windows resolves
        // to WSL which may not be installed), sh -c elsewhere.
        let mut c = if cfg!(windows) {
            let mut cmd = std::process::Command::new("cmd");
            cmd.arg("/C");
            cmd
        } else {
            let mut cmd = std::process::Command::new("sh");
            cmd.arg("-c");
            cmd
        };
        c.arg(&cmd_str).current_dir(&ws_path).output()
    })
    .await;
    let duration_ms = started.elapsed().as_millis() as u64;

    let (exit_code, stdout_tail) = match exec {
        Ok(Ok(out)) => {
            let code = out.status.code().unwrap_or(-1);
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            let combined = if stderr.is_empty() {
                stdout
            } else if stdout.is_empty() {
                stderr
            } else {
                format!("{stdout}\n{stderr}")
            };
            (code, tail_lines(&combined, 40))
        }
        Ok(Err(e)) => (-1, format!("failed to spawn command: {e}")),
        Err(e) => (-1, format!("join error: {e}")),
    };

    if cache && exit_code == 0 && !tree_hash.is_empty() {
        let entry = CacheEntry {
            tree_hash: tree_hash.clone(),
            exit_code,
            duration_ms,
            stdout_tail: stdout_tail.clone(),
        };
        let _ = save_cache(&cache_dir, &entry);
    }

    TestResult {
        ticket: ws.ticket,
        exit_code,
        duration_ms,
        from_cache: false,
        stdout_tail,
    }
}

/// Return the last `n` lines of `text`, joined by `\n`.
fn tail_lines(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= n {
        return lines.join("\n");
    }
    lines[lines.len() - n..].join("\n")
}

/// Try to read a cached test result for `tree_hash`. Returns `None` on
/// any I/O or parse error.
fn load_cache(cache_dir: &Path, tree_hash: &str) -> Option<CacheEntry> {
    let path = cache_dir.join(format!("{tree_hash}.json"));
    let bytes = std::fs::read(&path).ok()?;
    serde_json::from_slice::<CacheEntry>(&bytes).ok()
}

/// Persist `entry` to `<cache_dir>/<tree_hash>.json` (best-effort).
fn save_cache(cache_dir: &Path, entry: &CacheEntry) -> Result<()> {
    let path = cache_dir.join(format!("{}.json", entry.tree_hash));
    let bytes = serde_json::to_vec_pretty(entry)?;
    std::fs::write(&path, bytes)
        .with_context(|| format!("failed to write cache file: {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tail_lines_returns_all_when_short() {
        assert_eq!(tail_lines("a\nb\nc", 5), "a\nb\nc");
    }

    #[test]
    fn tail_lines_truncates_when_long() {
        let text = (1..=10)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let tail = tail_lines(&text, 3);
        assert_eq!(tail, "8\n9\n10");
    }
}
