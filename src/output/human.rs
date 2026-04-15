use colored::Colorize;
use tabled::settings::Style;
use tabled::{Table, Tabled};

use crate::config::ParsecConfig;
use crate::conflict::FileConflict;
use crate::oplog::OpEntry;
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
    // Shell integration hint
    eprintln!(
        "\n  {} cd $(parsec switch {})",
        "Tip:".bold().cyan(),
        workspace.ticket
    );
}

pub fn print_adopt(workspace: &Workspace) {
    let msg = format!(
        "Adopted branch '{}' as {} at {}",
        workspace.branch,
        workspace.ticket.bold(),
        workspace.path.display()
    );
    println!("{}", msg.green());
    if let Some(title) = &workspace.ticket_title {
        println!("  {}", title.dimmed());
    }
    // Shell integration hint
    eprintln!(
        "\n  {} cd $(parsec switch {})",
        "Tip:".bold().cyan(),
        workspace.ticket
    );
}

pub fn print_list(workspaces: &[Workspace]) {
    if workspaces.is_empty() {
        println!("{}", "No active workspaces.".dimmed());
        return;
    }
    let rows: Vec<WorkspaceRow> = workspaces.iter().map(workspace_to_row).collect();
    let table = Table::new(rows).with(Style::modern()).to_string();
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
    println!("{}", format!("Shipped {}!", result.ticket).green().bold());
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
    let table = Table::new(rows).with(Style::modern()).to_string();
    println!("{}", table);
}

pub fn print_switch(workspace: &Workspace) {
    // Intentionally plain — designed for `cd $(parsec switch X)`
    print!("{}", workspace.path.display());
}

pub fn print_config_init() {
    println!("{}", "Configuration saved!".green().bold());
}

pub fn print_log(entries: &[&OpEntry]) {
    if entries.is_empty() {
        println!("{}", "No operations recorded.".dimmed());
        return;
    }

    #[derive(Tabled)]
    struct LogRow {
        #[tabled(rename = "#")]
        id: u64,
        #[tabled(rename = "Op")]
        op: String,
        #[tabled(rename = "Ticket")]
        ticket: String,
        #[tabled(rename = "Detail")]
        detail: String,
        #[tabled(rename = "Time")]
        time: String,
    }

    let rows: Vec<LogRow> = entries
        .iter()
        .rev()
        .map(|e| LogRow {
            id: e.id,
            op: match &e.op {
                crate::oplog::OpKind::Start => "start".green().to_string(),
                crate::oplog::OpKind::Adopt => "adopt".cyan().to_string(),
                crate::oplog::OpKind::Ship => "ship".yellow().to_string(),
                crate::oplog::OpKind::Clean => "clean".red().to_string(),
                crate::oplog::OpKind::Undo => "undo".magenta().to_string(),
            },
            ticket: e.ticket.clone().unwrap_or_else(|| "-".to_string()),
            detail: e.detail.clone(),
            time: e.timestamp.format("%Y-%m-%d %H:%M").to_string(),
        })
        .collect();

    let table = Table::new(rows).with(Style::modern()).to_string();
    println!("{}", table);
}

pub fn print_undo(entry: &OpEntry) {
    let ticket_str = entry.ticket.as_deref().unwrap_or("?");
    let msg = format!("Undid {} for {}", entry.op, ticket_str);
    println!("{}", msg.green().bold());

    match &entry.op {
        crate::oplog::OpKind::Start | crate::oplog::OpKind::Adopt => {
            println!("  {}", "Worktree removed.".dimmed());
        }
        crate::oplog::OpKind::Ship | crate::oplog::OpKind::Clean => {
            if let Some(info) = &entry.undo_info {
                if let Some(path) = &info.path {
                    println!("  {} {}", "Restored at:".bold(), path.display());
                }
            }
            println!("  {}", "Workspace restored.".dimmed());
        }
        _ => {}
    }
}

pub fn print_undo_preview(entry: &OpEntry) {
    let ticket_str = entry.ticket.as_deref().unwrap_or("?");
    println!(
        "{}",
        format!("Would undo: {} {}", entry.op, ticket_str)
            .yellow()
            .bold()
    );

    match &entry.op {
        crate::oplog::OpKind::Start | crate::oplog::OpKind::Adopt => {
            if let Some(info) = &entry.undo_info {
                if let Some(path) = &info.path {
                    println!("  Would remove worktree at {}", path.display());
                }
                if let Some(branch) = &info.branch {
                    println!("  Would delete branch '{}'", branch);
                }
            }
        }
        crate::oplog::OpKind::Ship | crate::oplog::OpKind::Clean => {
            if let Some(info) = &entry.undo_info {
                if let Some(branch) = &info.branch {
                    println!("  Would restore worktree for branch '{}'", branch);
                }
            }
        }
        _ => {
            println!("  {}", "This operation cannot be undone.".red());
        }
    }
}

pub fn print_sync(synced: &[String], failed: &[(String, String)], strategy: &str) {
    if !synced.is_empty() {
        println!(
            "{} {} {} worktree(s):",
            "✓".green(),
            strategy.bold(),
            synced.len()
        );
        for ticket in synced {
            println!("  - {}", ticket);
        }
    }
    if !failed.is_empty() {
        println!(
            "{} Failed to {} {} worktree(s):",
            "✗".red(),
            strategy,
            failed.len()
        );
        for (ticket, reason) in failed {
            println!("  - {}: {}", ticket, reason.red());
        }
    }
    if synced.is_empty() && failed.is_empty() {
        println!("Nothing to sync.");
    }
}

pub fn print_pr_status(statuses: &[(String, crate::github::PrStatus)]) {
    use tabled::{Table, Tabled};

    #[derive(Tabled)]
    struct Row {
        #[tabled(rename = "Ticket")]
        ticket: String,
        #[tabled(rename = "PR")]
        pr: String,
        #[tabled(rename = "State")]
        state: String,
        #[tabled(rename = "CI")]
        ci: String,
        #[tabled(rename = "Reviews")]
        reviews: String,
    }

    let rows: Vec<Row> = statuses
        .iter()
        .map(|(ticket, s)| {
            let ci = match s.ci_status.as_str() {
                "success" => "✓ passed".green().to_string(),
                "failure" | "error" => "✗ failed".red().to_string(),
                "pending" => "● pending".yellow().to_string(),
                _ => s.ci_status.clone(),
            };
            let reviews = match s.review_status.as_str() {
                "approved" => "✓ approved".green().to_string(),
                "changes_requested" => "✗ changes requested".red().to_string(),
                "pending" => "● pending".yellow().to_string(),
                _ => s.review_status.clone(),
            };
            let state = match s.state.as_str() {
                "open" => "open".green().to_string(),
                "closed" => "closed".red().to_string(),
                "merged" => "merged".cyan().to_string(),
                _ => s.state.clone(),
            };
            Row {
                ticket: ticket.clone(),
                pr: format!("#{}", s.number),
                state,
                ci,
                reviews,
            }
        })
        .collect();

    if rows.is_empty() {
        println!("No shipped PRs found.");
        return;
    }

    let table = Table::new(rows)
        .with(tabled::settings::Style::modern())
        .to_string();
    println!("{table}");
}

pub fn print_merge(
    ticket: &str,
    pr_number: u64,
    result: &crate::github::MergeResult,
    method: &str,
) {
    println!(
        "{}",
        format!("Merged PR #{} for {}!", pr_number, ticket)
            .green()
            .bold()
    );
    println!("  {} {}", "Method:".bold(), method);
    println!("  {} {}", "SHA:".bold(), result.sha.dimmed());
    if !result.message.is_empty() {
        println!("  {} {}", "Message:".bold(), result.message);
    }
}

pub fn print_ci_status(statuses: &[(String, crate::github::CiStatus)]) {
    use tabled::{Table, Tabled};

    for (ticket, ci) in statuses {
        println!(
            "{} {} (PR #{}, {})",
            "CI for".bold(),
            ticket.bold().cyan(),
            ci.pr_number,
            ci.head_sha[..7].dimmed()
        );

        if ci.checks.is_empty() {
            println!("  {}", "No checks found.".dimmed());
            println!();
            continue;
        }

        #[derive(Tabled)]
        struct CheckRow {
            #[tabled(rename = "Check")]
            name: String,
            #[tabled(rename = "Status")]
            status: String,
            #[tabled(rename = "Duration")]
            duration: String,
        }

        let rows: Vec<CheckRow> = ci
            .checks
            .iter()
            .map(|c| {
                let status = match (c.status.as_str(), c.conclusion.as_deref()) {
                    (_, Some("success")) => "✓ passed".green().to_string(),
                    (_, Some("failure")) => "✗ failed".red().to_string(),
                    (_, Some("skipped")) => "- skipped".dimmed().to_string(),
                    ("in_progress", _) => "● running".yellow().to_string(),
                    ("queued", _) => "○ queued".dimmed().to_string(),
                    _ => c.status.clone(),
                };
                let duration = match (&c.started_at, &c.completed_at) {
                    (Some(start), Some(end)) => {
                        if let (Ok(s), Ok(e)) = (
                            chrono::DateTime::parse_from_rfc3339(start),
                            chrono::DateTime::parse_from_rfc3339(end),
                        ) {
                            let secs = (e - s).num_seconds();
                            if secs >= 60 {
                                format!("{}m {}s", secs / 60, secs % 60)
                            } else {
                                format!("{}s", secs)
                            }
                        } else {
                            "-".to_string()
                        }
                    }
                    (Some(_), None) => "running...".to_string(),
                    _ => "-".to_string(),
                };
                CheckRow {
                    name: c.name.clone(),
                    status,
                    duration,
                }
            })
            .collect();

        let table = Table::new(rows)
            .with(tabled::settings::Style::modern())
            .to_string();
        println!("{table}");

        // Summary line
        let passed = ci
            .checks
            .iter()
            .filter(|c| c.conclusion.as_deref() == Some("success"))
            .count();
        let failed = ci
            .checks
            .iter()
            .filter(|c| c.conclusion.as_deref() == Some("failure"))
            .count();
        let running = ci
            .checks
            .iter()
            .filter(|c| c.status == "in_progress")
            .count();
        let queued = ci.checks.iter().filter(|c| c.status == "queued").count();
        let total = ci.checks.len();

        let mut parts = Vec::new();
        if passed > 0 {
            parts.push(format!("{passed} passed").green().to_string());
        }
        if failed > 0 {
            parts.push(format!("{failed} failed").red().to_string());
        }
        if running > 0 {
            parts.push(format!("{running} running").yellow().to_string());
        }
        if queued > 0 {
            parts.push(format!("{queued} queued"));
        }

        let overall_icon = match ci.overall.as_str() {
            "passing" => "✓".green().to_string(),
            "failing" => "✗".red().to_string(),
            _ => "●".yellow().to_string(),
        };

        println!(
            "{} CI: {}/{} — {}",
            overall_icon,
            passed,
            total,
            parts.join(", ")
        );
        println!();
    }
}

pub fn print_diff_names(files: &[String], ticket: &str) {
    println!(
        "{} {} ({} files):",
        "Changed files for".bold(),
        ticket.bold().cyan(),
        files.len()
    );
    for f in files {
        println!("  {}", f);
    }
}

pub fn print_diff_stat(stat: &str, ticket: &str) {
    println!("{} {}:", "Diff stat for".bold(), ticket.bold().cyan());
    print!("{}", stat);
}

pub fn print_config_show(config: &ParsecConfig) {
    println!("{}", "[workspace]".bold());
    println!("  layout          = {}", config.workspace.layout);
    println!("  base_dir        = {}", config.workspace.base_dir);
    println!("  branch_prefix   = {}", config.workspace.branch_prefix);
    println!();
    println!("{}", "[tracker]".bold());
    println!("  provider       = {}", config.tracker.provider);
    if let Some(jira) = &config.tracker.jira {
        println!("  jira.base_url  = {}", jira.base_url);
        if let Some(email) = &jira.email {
            println!("  jira.email     = {}", email);
        }
    }
    if let Some(gitlab) = &config.tracker.gitlab {
        println!("  gitlab.base_url = {}", gitlab.base_url);
    }
    println!();
    println!("{}", "[ship]".bold());
    println!("  auto_pr         = {}", config.ship.auto_pr);
    println!("  auto_cleanup    = {}", config.ship.auto_cleanup);
    println!("  draft           = {}", config.ship.draft);
    if !config.hooks.post_create.is_empty() {
        println!();
        println!("{}", "[hooks]".bold());
        for cmd in &config.hooks.post_create {
            println!("  post_create     = {}", cmd);
        }
    }
}
