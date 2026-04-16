use serde::Serialize;
use serde_json::json;

use super::BoardTicketDisplay;
use crate::config::ParsecConfig;
use crate::conflict::FileConflict;
use crate::oplog::OpEntry;
use crate::tracker::jira::{InboxTicket, SprintInfo};
use crate::tracker::Ticket as TrackerTicket;
use crate::worktree::{ShipResult, Workspace};

fn emit<T: Serialize>(value: &T) {
    println!(
        "{}",
        serde_json::to_string(value)
            .unwrap_or_else(|e| { json!({ "error": e.to_string() }).to_string() })
    );
}

pub fn print_start(workspace: &Workspace) {
    emit(workspace);
}

pub fn print_adopt(workspace: &Workspace) {
    emit(workspace);
}

pub fn print_list(
    workspaces: &[Workspace],
    pr_map: &std::collections::HashMap<String, (u64, String)>,
) {
    let value: Vec<serde_json::Value> = workspaces
        .iter()
        .map(|ws| {
            let mut obj = serde_json::to_value(ws).unwrap_or(serde_json::json!({}));
            if let Some((num, state)) = pr_map.get(&ws.ticket) {
                obj["pr_number"] = serde_json::json!(num);
                obj["pr_state"] = serde_json::json!(state);
            }
            obj
        })
        .collect();
    emit(&value);
}

pub fn print_status(workspaces: &[Workspace]) {
    emit(&workspaces);
}

pub fn print_ship(result: &ShipResult) {
    emit(result);
}

pub fn print_clean(removed: &[Workspace], dry_run: bool) {
    let value = json!({
        "dry_run": dry_run,
        "removed": removed,
        "count": removed.len(),
    });
    println!("{}", value);
}

pub fn print_conflicts(conflicts: &[FileConflict]) {
    emit(&conflicts);
}

pub fn print_switch(workspace: &Workspace) {
    let value = json!({ "path": workspace.path });
    println!("{}", value);
}

pub fn print_log(entries: &[&OpEntry]) {
    emit(&entries);
}

pub fn print_config_init() {
    let value = json!({ "status": "ok", "message": "Configuration saved" });
    println!("{}", value);
}

pub fn print_config_show(config: &ParsecConfig) {
    emit(config);
}

pub fn print_sync(synced: &[String], failed: &[(String, String)], strategy: &str) {
    let value = json!({
        "action": "sync",
        "strategy": strategy,
        "synced": synced,
        "failed": failed.iter().map(|(t, r)| json!({"ticket": t, "reason": r})).collect::<Vec<_>>(),
    });
    println!("{}", value);
}

pub fn print_pr_status(statuses: &[(String, crate::github::PrStatus)]) {
    let value: Vec<_> = statuses
        .iter()
        .map(|(ticket, s)| {
            json!({
                "ticket": ticket,
                "pr_number": s.number,
                "title": s.title,
                "state": s.state,
                "ci_status": s.ci_status,
                "review_status": s.review_status,
                "mergeable": s.mergeable,
                "url": s.url,
            })
        })
        .collect();
    emit(&value);
}

pub fn print_merge(
    ticket: &str,
    pr_number: u64,
    result: &crate::github::MergeResult,
    method: &str,
) {
    let value = json!({
        "action": "merge",
        "ticket": ticket,
        "pr_number": pr_number,
        "method": method,
        "sha": result.sha,
        "message": result.message,
        "merged": result.merged,
    });
    println!("{}", value);
}

pub fn print_ci_status(statuses: &[(String, crate::github::CiStatus)]) {
    let value: Vec<_> = statuses
        .iter()
        .map(|(ticket, ci)| {
            json!({
                "ticket": ticket,
                "pr_number": ci.pr_number,
                "head_sha": ci.head_sha,
                "overall": ci.overall,
                "checks": ci.checks,
            })
        })
        .collect();
    emit(&value);
}

pub fn print_diff_names(files: &[String], ticket: &str) {
    let value = json!({
        "ticket": ticket,
        "files": files,
        "count": files.len(),
    });
    println!("{}", value);
}

pub fn print_diff_full(files: &[(String, String)], ticket: &str) {
    let value = json!({
        "ticket": ticket,
        "changes": files.iter().map(|(status, file)| json!({"status": status, "file": file})).collect::<Vec<_>>(),
        "count": files.len(),
    });
    println!("{}", value);
}

pub fn print_stack(workspaces: &[Workspace]) {
    let value: Vec<_> = workspaces
        .iter()
        .map(|ws| {
            json!({
                "ticket": ws.ticket,
                "branch": ws.branch,
                "base_branch": ws.base_branch,
                "parent_ticket": ws.parent_ticket,
                "title": ws.ticket_title,
            })
        })
        .collect();
    emit(&value);
}

pub fn print_undo(entry: &OpEntry) {
    let value = json!({
        "action": "undo",
        "undone_op": format!("{}", entry.op),
        "ticket": entry.ticket,
    });
    println!("{}", value);
}

pub fn print_undo_preview(entry: &OpEntry) {
    let value = json!({
        "action": "undo_preview",
        "would_undo": format!("{}", entry.op),
        "ticket": entry.ticket,
        "undo_info": entry.undo_info,
    });
    println!("{}", value);
}

pub fn print_ticket(ticket: &TrackerTicket) {
    emit(ticket);
}

pub fn print_comment(ticket_id: &str) {
    let value = json!({
        "action": "comment",
        "ticket": ticket_id,
        "status": "ok",
    });
    println!("{}", value);
}

pub fn print_inbox(tickets: &[InboxTicket]) {
    emit(&tickets);
}

pub fn print_board(sprint: Option<&SprintInfo>, columns: &[(String, Vec<BoardTicketDisplay>)]) {
    let sprint_json = sprint.map(|s| {
        json!({
            "id": s.id,
            "name": s.name,
            "start": s.start_date,
            "end": s.end_date,
        })
    });

    let total_count: usize = columns.iter().map(|(_, tickets)| tickets.len()).sum();

    let columns_json: serde_json::Map<String, serde_json::Value> = columns
        .iter()
        .map(|(name, tickets)| {
            let ticket_list: Vec<serde_json::Value> = tickets
                .iter()
                .map(|t| {
                    let mut obj = json!({
                        "key": t.key,
                        "summary": t.summary,
                        "status": name,
                    });
                    if let Some(ref assignee) = t.assignee {
                        obj["assignee"] = json!(assignee);
                    }
                    if let Some(ref url) = t.url {
                        obj["url"] = json!(url);
                    }
                    if t.has_worktree {
                        obj["worktree"] = json!(true);
                    }
                    if t.has_pr {
                        obj["pr"] = json!(true);
                    }
                    obj
                })
                .collect();
            (name.clone(), json!(ticket_list))
        })
        .collect();

    let value = json!({
        "sprint": sprint_json,
        "total_count": total_count,
        "columns": columns_json,
    });
    println!("{}", value);
}
