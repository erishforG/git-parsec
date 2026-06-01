mod human;
mod json;

use crate::config::ParsecConfig;
use crate::conflict::FileConflict;
use crate::oplog::OpEntry;
use crate::tracker::jira::{InboxTicket, SprintInfo};
use crate::tracker::Ticket as TrackerTicket;
use crate::worktree::{ShipResult, Workspace};

/// Extended metadata for a workspace gathered for `parsec list --full`.
pub struct WorkspaceFullInfo {
    pub workspace: Workspace,
    pub unpushed: Option<u32>,
    pub ahead: Option<u32>,
    pub behind: Option<u32>,
    pub last_commit_msg: Option<String>,
    pub last_commit_age: Option<String>,
}

/// A ticket annotated with parsec-specific indicators for board display.
pub struct BoardTicketDisplay {
    pub key: String,
    pub summary: String,
    pub assignee: Option<String>,
    pub has_worktree: bool,
    pub has_pr: bool,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mode {
    Human,
    Json,
    Quiet,
}

/// A single diagnostic check result for `parsec doctor`.
pub struct DoctorCheck {
    pub name: String,
    pub ok: bool,
    pub detail: String,
    pub fix: Option<String>,
}

/// One health record per worktree, produced by `parsec health`.
pub struct HealthRecord {
    /// Ticket identifier for the worktree.
    pub ticket: String,
    /// Number of uncommitted files (staged + unstaged).
    pub uncommitted: usize,
    /// Days since the last commit, or `None` when the history is unreadable.
    pub stale_days: Option<i64>,
    /// Threshold above which the worktree is considered stale.
    pub stale_threshold_days: i64,
    /// Whether a `.git/index.lock` file exists (hung git process indicator).
    pub has_lock: bool,
    /// GitHub Actions / CI overall status for the worktree's open PR, if any.
    /// Populated by Phase 2 CI overlay; `None` when no PR or no token.
    pub ci_status: Option<String>,
    /// GitHub PR number linked to this worktree's branch, if any.
    pub pr_number: Option<u64>,
}

/// Generate a dispatch function that routes to json:: and human:: based on Mode.
///
/// Standard form (both Json and Human):
///   dispatch_output!(fn_name, arg1: Type1, arg2: Type2, ...);
///
/// Stat form (Human-only, skips Json):
///   dispatch_output!(@human_only fn_name, arg1: Type1, ...);
macro_rules! dispatch_output {
    // Human-only variant (skips Json mode)
    (@human_only $fn:ident $(, $arg:ident : $ty:ty)*) => {
        pub fn $fn($($arg: $ty,)* mode: Mode) {
            match mode {
                Mode::Quiet => {}
                Mode::Json => {} // human-only output
                Mode::Human => human::$fn($($arg),*),
            }
        }
    };
    // Standard variant (routes to both Json and Human)
    ($fn:ident $(, $arg:ident : $ty:ty)*) => {
        pub fn $fn($($arg: $ty,)* mode: Mode) {
            match mode {
                Mode::Quiet => {}
                Mode::Json => json::$fn($($arg),*),
                Mode::Human => human::$fn($($arg),*),
            }
        }
    };
}

dispatch_output!(print_start, workspace: &Workspace);
dispatch_output!(print_adopt, workspace: &Workspace);
dispatch_output!(
    print_list,
    workspaces: &[Workspace],
    pr_map: &std::collections::HashMap<String, (u64, String)>
);
dispatch_output!(print_status, workspaces: &[Workspace], ticket_infos: &[Option<crate::tracker::Ticket>]);
dispatch_output!(print_ship, result: &ShipResult);
dispatch_output!(print_clean, removed: &[Workspace], dry_run: bool);
dispatch_output!(print_conflicts, conflicts: &[FileConflict]);
dispatch_output!(print_switch, workspace: &Workspace);
dispatch_output!(print_config_init);
dispatch_output!(print_log, entries: &[&OpEntry]);
dispatch_output!(print_undo, entry: &OpEntry);
dispatch_output!(print_undo_preview, entry: &OpEntry);
dispatch_output!(
    print_sync,
    synced: &[String],
    skipped: &[(String, u32)],
    failed: &[(String, String)],
    strategy: &str
);
dispatch_output!(print_pr_status, statuses: &[(String, crate::github::PrStatus)]);
dispatch_output!(
    print_merge,
    ticket: &str,
    pr_number: u64,
    result: &crate::github::MergeResult,
    method: &str
);
dispatch_output!(print_ci_status, statuses: &[(String, crate::github::CiStatus)]);
dispatch_output!(print_stack, workspaces: &[Workspace]);
dispatch_output!(print_config_show, config: &ParsecConfig);
dispatch_output!(print_diff_names, files: &[String], ticket: &str);
dispatch_output!(@human_only print_diff_stat, stat: &str, ticket: &str);
dispatch_output!(
    print_board,
    sprint: Option<&SprintInfo>,
    columns: &[(String, Vec<BoardTicketDisplay>)]
);
dispatch_output!(print_ticket, ticket: &TrackerTicket);
dispatch_output!(print_comment, ticket_id: &str);
dispatch_output!(print_inbox, tickets: &[InboxTicket]);
dispatch_output!(print_doctor, checks: &[DoctorCheck]);
dispatch_output!(print_health, records: &[HealthRecord]);
dispatch_output!(
    print_list_full,
    infos: &[WorkspaceFullInfo],
    pr_map: &std::collections::HashMap<String, (u64, String)>
);
dispatch_output!(print_create, ticket_id: &str, title: &str, url: &str);

pub fn print_diff_full_json(files: &[(String, String)], ticket: &str) {
    json::print_diff_full(files, ticket);
}

pub fn print_rename(old_ticket: &str, new_ticket: &str, workspace: &Workspace, mode: Mode) {
    match mode {
        Mode::Quiet => {}
        Mode::Json => json::print_rename(old_ticket, new_ticket, workspace),
        Mode::Human => human::print_rename(old_ticket, new_ticket, workspace),
    }
}
