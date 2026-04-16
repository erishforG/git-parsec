mod human;
mod json;

use crate::config::ParsecConfig;
use crate::conflict::FileConflict;
use crate::oplog::OpEntry;
use crate::tracker::jira::SprintInfo;
use crate::tracker::Ticket as TrackerTicket;
use crate::worktree::{ShipResult, Workspace};

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

pub fn print_start(workspace: &Workspace, mode: Mode) {
    match mode {
        Mode::Quiet => {}
        Mode::Json => json::print_start(workspace),
        Mode::Human => human::print_start(workspace),
    }
}

pub fn print_adopt(workspace: &Workspace, mode: Mode) {
    match mode {
        Mode::Quiet => {}
        Mode::Json => json::print_adopt(workspace),
        Mode::Human => human::print_adopt(workspace),
    }
}

pub fn print_list(
    workspaces: &[Workspace],
    pr_map: &std::collections::HashMap<String, (u64, String)>,
    mode: Mode,
) {
    match mode {
        Mode::Quiet => {}
        Mode::Json => json::print_list(workspaces, pr_map),
        Mode::Human => human::print_list(workspaces, pr_map),
    }
}

pub fn print_status(workspaces: &[Workspace], mode: Mode) {
    match mode {
        Mode::Quiet => {}
        Mode::Json => json::print_status(workspaces),
        Mode::Human => human::print_status(workspaces),
    }
}

pub fn print_ship(result: &ShipResult, mode: Mode) {
    match mode {
        Mode::Quiet => {}
        Mode::Json => json::print_ship(result),
        Mode::Human => human::print_ship(result),
    }
}

pub fn print_clean(removed: &[Workspace], dry_run: bool, mode: Mode) {
    match mode {
        Mode::Quiet => {}
        Mode::Json => json::print_clean(removed, dry_run),
        Mode::Human => human::print_clean(removed, dry_run),
    }
}

pub fn print_conflicts(conflicts: &[FileConflict], mode: Mode) {
    match mode {
        Mode::Quiet => {}
        Mode::Json => json::print_conflicts(conflicts),
        Mode::Human => human::print_conflicts(conflicts),
    }
}

pub fn print_switch(workspace: &Workspace, mode: Mode) {
    match mode {
        Mode::Quiet => {}
        Mode::Json => json::print_switch(workspace),
        Mode::Human => human::print_switch(workspace),
    }
}

pub fn print_config_init(mode: Mode) {
    match mode {
        Mode::Quiet => {}
        Mode::Json => json::print_config_init(),
        Mode::Human => human::print_config_init(),
    }
}

pub fn print_log(entries: &[&OpEntry], mode: Mode) {
    match mode {
        Mode::Quiet => {}
        Mode::Json => json::print_log(entries),
        Mode::Human => human::print_log(entries),
    }
}

pub fn print_undo(entry: &OpEntry, mode: Mode) {
    match mode {
        Mode::Quiet => {}
        Mode::Json => json::print_undo(entry),
        Mode::Human => human::print_undo(entry),
    }
}

pub fn print_undo_preview(entry: &OpEntry, mode: Mode) {
    match mode {
        Mode::Quiet => {}
        Mode::Json => json::print_undo_preview(entry),
        Mode::Human => human::print_undo_preview(entry),
    }
}

pub fn print_sync(synced: &[String], failed: &[(String, String)], strategy: &str, mode: Mode) {
    match mode {
        Mode::Quiet => {}
        Mode::Json => json::print_sync(synced, failed, strategy),
        Mode::Human => human::print_sync(synced, failed, strategy),
    }
}

pub fn print_pr_status(statuses: &[(String, crate::github::PrStatus)], mode: Mode) {
    match mode {
        Mode::Quiet => {}
        Mode::Json => json::print_pr_status(statuses),
        Mode::Human => human::print_pr_status(statuses),
    }
}

pub fn print_merge(
    ticket: &str,
    pr_number: u64,
    result: &crate::github::MergeResult,
    method: &str,
    mode: Mode,
) {
    match mode {
        Mode::Quiet => {}
        Mode::Json => json::print_merge(ticket, pr_number, result, method),
        Mode::Human => human::print_merge(ticket, pr_number, result, method),
    }
}

pub fn print_ci_status(statuses: &[(String, crate::github::CiStatus)], mode: Mode) {
    match mode {
        Mode::Quiet => {}
        Mode::Json => json::print_ci_status(statuses),
        Mode::Human => human::print_ci_status(statuses),
    }
}

pub fn print_stack(workspaces: &[Workspace], mode: Mode) {
    match mode {
        Mode::Quiet => {}
        Mode::Json => json::print_stack(workspaces),
        Mode::Human => human::print_stack(workspaces),
    }
}

pub fn print_config_show(config: &ParsecConfig, mode: Mode) {
    match mode {
        Mode::Quiet => {}
        Mode::Json => json::print_config_show(config),
        Mode::Human => human::print_config_show(config),
    }
}

pub fn print_diff_names(files: &[String], ticket: &str, mode: Mode) {
    match mode {
        Mode::Quiet => {}
        Mode::Json => json::print_diff_names(files, ticket),
        Mode::Human => human::print_diff_names(files, ticket),
    }
}

pub fn print_diff_stat(stat: &str, ticket: &str, mode: Mode) {
    match mode {
        Mode::Quiet => {}
        Mode::Json => {} // stat is human-only
        Mode::Human => human::print_diff_stat(stat, ticket),
    }
}

pub fn print_diff_full_json(files: &[(String, String)], ticket: &str) {
    json::print_diff_full(files, ticket);
}

pub fn print_board(
    sprint: Option<&SprintInfo>,
    columns: &[(String, Vec<BoardTicketDisplay>)],
    mode: Mode,
) {
    match mode {
        Mode::Quiet => {}
        Mode::Json => json::print_board(sprint, columns),
        Mode::Human => human::print_board(sprint, columns),
    }
}

pub fn print_ticket(ticket: &TrackerTicket, mode: Mode) {
    match mode {
        Mode::Quiet => {}
        Mode::Json => json::print_ticket(ticket),
        Mode::Human => human::print_ticket(ticket),
    }
}

pub fn print_comment(ticket_id: &str, mode: Mode) {
    match mode {
        Mode::Quiet => {}
        Mode::Json => json::print_comment(ticket_id),
        Mode::Human => human::print_comment(ticket_id),
    }
}
