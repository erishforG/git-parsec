mod human;
mod json;

use crate::config::ParsecConfig;
use crate::conflict::FileConflict;
use crate::oplog::OpEntry;
use crate::worktree::{ShipResult, Workspace};

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

pub fn print_list(workspaces: &[Workspace], mode: Mode) {
    match mode {
        Mode::Quiet => {}
        Mode::Json => json::print_list(workspaces),
        Mode::Human => human::print_list(workspaces),
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

pub fn print_config_show(config: &ParsecConfig, mode: Mode) {
    match mode {
        Mode::Quiet => {}
        Mode::Json => json::print_config_show(config),
        Mode::Human => human::print_config_show(config),
    }
}
