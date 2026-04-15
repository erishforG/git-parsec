use serde::Serialize;
use serde_json::json;

use crate::config::ParsecConfig;
use crate::conflict::FileConflict;
use crate::oplog::OpEntry;
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

pub fn print_list(workspaces: &[Workspace]) {
    emit(&workspaces);
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
