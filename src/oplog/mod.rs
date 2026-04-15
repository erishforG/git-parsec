use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpKind {
    Start,
    Adopt,
    Ship,
    Clean,
    Undo,
}

impl std::fmt::Display for OpKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpKind::Start => write!(f, "start"),
            OpKind::Adopt => write!(f, "adopt"),
            OpKind::Ship => write!(f, "ship"),
            OpKind::Clean => write!(f, "clean"),
            OpKind::Undo => write!(f, "undo"),
        }
    }
}

/// Information needed to undo an operation (for future parsec undo)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UndoInfo {
    pub branch: Option<String>,
    pub base_branch: Option<String>,
    pub path: Option<PathBuf>,
    pub ticket_title: Option<String>,
}

/// A single operation log entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpEntry {
    pub id: u64,
    pub op: OpKind,
    pub ticket: Option<String>,
    pub detail: String,
    pub timestamp: DateTime<Utc>,
    pub undo_info: Option<UndoInfo>,
}

/// The full operation log
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OpLog {
    pub entries: Vec<OpEntry>,
}

impl OpLog {
    fn log_path(repo_root: &Path) -> PathBuf {
        repo_root.join(".parsec").join("oplog.json")
    }

    pub fn load(repo_root: &Path) -> Result<Self> {
        let path = Self::log_path(repo_root);
        if !path.exists() {
            return Ok(Self::default());
        }
        let contents = fs::read_to_string(&path)
            .with_context(|| format!("failed to read oplog: {}", path.display()))?;
        let log: Self = serde_json::from_str(&contents)
            .with_context(|| format!("failed to parse oplog: {}", path.display()))?;
        Ok(log)
    }

    pub fn save(&self, repo_root: &Path) -> Result<()> {
        let path = Self::log_path(repo_root);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let contents = serde_json::to_string_pretty(self)?;
        fs::write(&path, contents)?;
        Ok(())
    }

    pub fn append(
        &mut self,
        op: OpKind,
        ticket: Option<String>,
        detail: String,
        undo_info: Option<UndoInfo>,
    ) {
        let id = self.entries.last().map(|e| e.id + 1).unwrap_or(1);
        self.entries.push(OpEntry {
            id,
            op,
            ticket,
            detail,
            timestamp: Utc::now(),
            undo_info,
        });
    }

    /// Get entries, optionally filtered by ticket
    pub fn get_entries(&self, ticket: Option<&str>) -> Vec<&OpEntry> {
        self.entries
            .iter()
            .filter(|e| match ticket {
                Some(t) => e.ticket.as_deref() == Some(t),
                None => true,
            })
            .collect()
    }

    /// Get the last entry (for undo)
    pub fn last_entry(&self) -> Option<&OpEntry> {
        self.entries.last()
    }
}

/// Helper to record an operation. Call from commands.rs after each mutating operation.
pub fn record(
    repo_root: &Path,
    op: OpKind,
    ticket: Option<&str>,
    detail: &str,
    undo_info: Option<UndoInfo>,
) -> Result<()> {
    let mut log = OpLog::load(repo_root)?;
    log.append(
        op,
        ticket.map(|s| s.to_owned()),
        detail.to_owned(),
        undo_info,
    );
    log.save(repo_root)?;
    Ok(())
}
