use colored::Colorize;
use tabled::settings::Style;
use tabled::{Table, Tabled};

use crate::config::ParsecConfig;
use crate::conflict::FileConflict;
use crate::worktree::{ShipResult, Workspace, WorkspaceStatus};

// ---------------------------------------------------------------------------
// Table row types
// ---------------------------------------------------------------------------

#[derive(Tabled)]
struct WorkspaceRow {
    #[tabled(rename = "Ticket")]
    ticket: String,
    #[tabled(rename = "Branch")]
    branch: String,
    #[tabled(rename = "Status")]
    status: String,
    #[tabled(rename = "Created")]
    created: String,
    #[tabled(rename = "Path")]
    path: String,
}

#[derive(Tabled)]
struct ConflictRow {
    #[tabled(rename = "File")]
    file: String,
    #[tabled(rename = "Worktrees")]
    worktrees: String,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn status_label(status: &WorkspaceStatus) -> String {
    match status {
        WorkspaceStatus::Active => "active".green().to_string(),
        WorkspaceStatus::Shipped => "shipped".yellow().to_string(),
        WorkspaceStatus::Merged => "merged".blue().to_string(),
    }
}

fn workspace_to_row(ws: &Workspace) -> WorkspaceRow {
    WorkspaceRow {
        ticket: ws.ticket.clone(),
        branch: ws.branch.clone(),
        status: status_label(&ws.status),
        created: ws.created_at.format("%Y-%m-%d %H:%M").to_string(),
        path: ws.path.display().to_string(),
    }
}

// ---------------------------------------------------------------------------
// Public print functions
// ---------------------------------------------------------------------------

pub fn print_start(workspace: &Workspace) {
    let msg = format!(
        "Created workspace for {} at {}",
        workspace.ticket.bold(),
        workspace.path.display()
    );
    println!("{}", msg.green());
    if let Some(title) = &workspace.ticket_title {
        println!("  {}", title.dimmed());
    }
}

pub fn print_list(workspaces: &[Workspace]) {
    if workspaces.is_empty() {
        println!("{}", "No active workspaces.".dimmed());
        return;
    }
    let rows: Vec<WorkspaceRow> = workspaces.iter().map(workspace_to_row).collect();
    let table = Table::new(rows).with(Style::rounded()).to_string();
    println!("{}", table);
}

pub fn print_status(workspaces: &[Workspace]) {
    if workspaces.is_empty() {
        println!("{}", "No workspaces found.".dimmed());
        return;
    }
    for ws in workspaces {
        println!("{}", "─".repeat(50).dimmed());
        println!("  {} {}", "Ticket:".bold(), ws.ticket);
        if let Some(title) = &ws.ticket_title {
            println!("  {} {}", "Title:".bold(), title);
        }
        println!("  {} {}", "Branch:".bold(), ws.branch);
        println!("  {} {}", "Base:".bold(), ws.base_branch);
        println!("  {} {}", "Status:".bold(), status_label(&ws.status));
        println!(
            "  {} {}",
            "Created:".bold(),
            ws.created_at.format("%Y-%m-%d %H:%M UTC")
        );
        println!("  {} {}", "Path:".bold(), ws.path.display());
    }
    println!("{}", "─".repeat(50).dimmed());
}

pub fn print_ship(result: &ShipResult) {
    println!(
        "{}",
        format!("Shipped {}!", result.ticket).green().bold()
    );
    if let Some(url) = &result.pr_url {
        println!("  {} {}", "PR:".bold(), url.cyan());
    }
    if result.cleaned_up {
        println!("  {}", "Workspace cleaned up.".dimmed());
    }
}

pub fn print_clean(removed: &[Workspace], dry_run: bool) {
    if removed.is_empty() {
        if dry_run {
            println!("{}", "Nothing to remove.".dimmed());
        } else {
            println!("{}", "No worktrees were removed.".dimmed());
        }
        return;
    }
    let verb = if dry_run { "Would remove" } else { "Removed" };
    println!("{} {} worktree(s):", verb.bold(), removed.len());
    for ws in removed {
        println!("  {} {}", "-".dimmed(), ws.ticket.yellow());
    }
}

pub fn print_conflicts(conflicts: &[FileConflict]) {
    if conflicts.is_empty() {
        println!("{}", "No conflicts detected.".green());
        return;
    }
    println!(
        "{}",
        format!("Found {} conflict(s):", conflicts.len())
            .yellow()
            .bold()
    );
    let rows: Vec<ConflictRow> = conflicts
        .iter()
        .map(|c| ConflictRow {
            file: c.file.clone(),
            worktrees: c.worktrees.join(", "),
        })
        .collect();
    let table = Table::new(rows).with(Style::rounded()).to_string();
    println!("{}", table);
}

pub fn print_switch(workspace: &Workspace) {
    // Intentionally plain — designed for `cd $(parsec switch X)`
    print!("{}", workspace.path.display());
}

pub fn print_config_init() {
    println!("{}", "Configuration saved!".green().bold());
}

pub fn print_config_show(config: &ParsecConfig) {
    println!("{}", "[workspace]".bold());
    println!("  base_dir       = {}", config.workspace.base_dir);
    println!("  branch_prefix  = {}", config.workspace.branch_prefix);
    println!();
    println!("{}", "[tracker]".bold());
    println!("  provider       = {}", config.tracker.provider);
    if let Some(jira) = &config.tracker.jira {
        println!("  jira.base_url  = {}", jira.base_url);
        if let Some(email) = &jira.email {
            println!("  jira.email     = {}", email);
        }
    }
    println!();
    println!("{}", "[ship]".bold());
    println!("  auto_pr        = {}", config.ship.auto_pr);
    println!("  auto_cleanup   = {}", config.ship.auto_cleanup);
    println!("  draft          = {}", config.ship.draft);
}
