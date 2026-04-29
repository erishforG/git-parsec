//! Lightweight execution log for observability.
//!
//! Each parsec command invocation is recorded as an `ExecEntry` with a unique
//! execution ID, timing, and optional step-level detail. Entries are stored as
//! newline-delimited JSON (JSONL) in `.parsec/execlog.jsonl`.

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

/// A single phase within a command execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecStep {
    pub phase: String,
    pub status: String,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// A complete command execution record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecEntry {
    pub execution_id: String,
    pub command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ticket: Option<String>,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    pub duration_ms: u64,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub steps: Vec<ExecStep>,
}

// ---------------------------------------------------------------------------
// Thread-local accumulators
// ---------------------------------------------------------------------------

thread_local! {
    static CURRENT_STEPS: RefCell<Vec<ExecStep>> = const { RefCell::new(Vec::new()) };
    static CURRENT_TICKET: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Record a step in the current execution context.
pub fn record_step(phase: &str, status: &str, duration_ms: u64, detail: Option<String>) {
    CURRENT_STEPS.with(|steps| {
        steps.borrow_mut().push(ExecStep {
            phase: phase.to_string(),
            status: status.to_string(),
            duration_ms,
            detail,
        });
    });
}

/// Set the ticket for the current execution (called from commands).
pub fn set_ticket(ticket: &str) {
    CURRENT_TICKET.with(|t| *t.borrow_mut() = Some(ticket.to_string()));
}

/// Take all accumulated steps (clears the accumulator).
pub fn take_steps() -> Vec<ExecStep> {
    CURRENT_STEPS.with(|steps| std::mem::take(&mut *steps.borrow_mut()))
}

/// Take the ticket set during execution.
pub fn take_ticket() -> Option<String> {
    CURRENT_TICKET.with(|t| t.borrow_mut().take())
}

/// Generate a new execution ID (UUID v4).
pub fn new_execution_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

// ---------------------------------------------------------------------------
// Persistence (JSONL)
// ---------------------------------------------------------------------------

fn execlog_path(repo_root: &Path) -> PathBuf {
    repo_root.join(".parsec").join("execlog.jsonl")
}

/// Append an execution entry to the JSONL log.
pub fn append(repo_root: &Path, entry: &ExecEntry) -> Result<()> {
    let path = execlog_path(repo_root);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let line = serde_json::to_string(entry)?;
    let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
    writeln!(file, "{}", line)?;
    Ok(())
}

/// Load all execution entries from the JSONL log.
#[allow(dead_code)]
pub fn load(repo_root: &Path) -> Result<Vec<ExecEntry>> {
    let path = execlog_path(repo_root);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let contents = fs::read_to_string(&path)?;
    Ok(contents
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect())
}

/// Read raw JSONL content for export.
pub fn read_raw(repo_root: &Path) -> Result<String> {
    let path = execlog_path(repo_root);
    if !path.exists() {
        return Ok(String::new());
    }
    Ok(fs::read_to_string(&path)?)
}
