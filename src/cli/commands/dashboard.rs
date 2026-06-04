//! `parsec dashboard` (alias `dash`) — interactive TUI dashboard (#248).
//!
//! Built on `ratatui` + `crossterm`, the dashboard renders three panes in a
//! single terminal screen:
//!
//! | Pane          | Contents                                                |
//! |---------------|---------------------------------------------------------|
//! | Worktrees     | List of every active worktree (ticket · branch · status)|
//! | CI Status     | Per-worktree CI summary (`PR #N · ✓ / ✗ / ●`)           |
//! | PRs           | Table view: PR · title · state · review · CI            |
//!
//! Keys: `q` / `Esc` to quit, `r` to force-refresh, `?` to toggle help, and
//! `↑/↓` to move the selection in the worktrees pane.
//!
//! Data is loaded once on entry and refreshed in the background every
//! `refresh_secs` seconds via a `tokio::task`. The draw loop never blocks on
//! network I/O — when a refresh is in flight the previous snapshot stays on
//! screen. When `--no-overlay` is passed, no GitHub calls are made and PR/CI
//! columns show `–` as a placeholder.
//!
//! Terminal state (alternate screen + raw mode) is restored by an RAII guard
//! that runs even on panic, so an unexpected error never leaves the user with
//! a corrupted terminal.

use std::io::{self, Stdout};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{DateTime, Local, Utc};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Cell, Clear, List, ListItem, ListState, Paragraph, Row, Table,
};
use ratatui::Terminal;
use tokio::sync::{mpsc, Mutex};

use crate::config::ParsecConfig;
use crate::git;
use crate::github::GitHubClient;
use crate::worktree::{Workspace, WorktreeManager};

/// Compact PR + CI overlay attached to a worktree row.
#[derive(Debug, Clone)]
struct PrOverlay {
    number: u64,
    title: String,
    state: String,
    ci_status: String,
    review_status: String,
}

/// One row in the worktrees pane (and the index into pr-map for cross-references).
#[derive(Debug, Clone)]
struct DashboardRow {
    ticket: String,
    ticket_title: Option<String>,
    branch: String,
    status: String,
    pr: Option<PrOverlay>,
}

/// Snapshot of all dashboard state — atomically swapped on each refresh.
#[derive(Debug, Clone, Default)]
struct DashboardSnapshot {
    rows: Vec<DashboardRow>,
    last_update: Option<DateTime<Utc>>,
    last_error: Option<String>,
    /// Whether overlay fetching is active. `false` when `--no-overlay` was set
    /// or no GitHub token was available.
    overlay_enabled: bool,
}

/// Messages from background refresh task to the UI loop.
///
/// The payload is intentionally a unit — the UI re-reads the shared snapshot
/// from the `Arc<Mutex<...>>` after each tick, so we only need a wake-up
/// signal rather than the snapshot itself.
enum RefreshMessage {
    /// A new snapshot has been written; wake the UI loop so it redraws.
    Tick,
}

/// Entry point for the `parsec dashboard` subcommand.
///
/// Opens an interactive terminal UI showing worktrees, CI, and PR status.
/// Runs until the user quits (`q` / `Esc`). Terminal state is always
/// restored on exit, even on panic.
pub async fn dashboard(repo: &Path, refresh_secs: u64, no_overlay: bool) -> Result<()> {
    // Build initial snapshot synchronously so the first frame has data.
    let initial = collect_snapshot(repo, no_overlay).await;

    // Shared snapshot — UI reads it, background task writes it.
    let snapshot = Arc::new(Mutex::new(initial));

    // Channel to nudge the UI loop on each refresh + on manual `r` press.
    let (tx, mut rx) = mpsc::channel::<RefreshMessage>(8);

    // Background refresh task.
    let bg_repo = repo.to_path_buf();
    let bg_snapshot = Arc::clone(&snapshot);
    let bg_tx = tx.clone();
    let refresh_secs = refresh_secs.max(1);
    let bg_handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(refresh_secs));
        // Skip the immediate tick — we already loaded data synchronously.
        interval.tick().await;
        loop {
            interval.tick().await;
            let snap = collect_snapshot(&bg_repo, no_overlay).await;
            {
                let mut guard = bg_snapshot.lock().await;
                *guard = snap;
            }
            if bg_tx.send(RefreshMessage::Tick).await.is_err() {
                break; // UI exited.
            }
        }
    });

    // ----- Set up terminal (with RAII restore on Drop) -----
    let mut guard = TerminalGuard::new()?;
    let res = run_ui(
        &mut guard.terminal,
        snapshot,
        &mut rx,
        repo.to_path_buf(),
        no_overlay,
        tx,
    )
    .await;
    drop(guard);

    bg_handle.abort();
    res
}

/// Drive the UI event loop. Returns when the user quits or an unrecoverable
/// error occurs (the `TerminalGuard` restores state on drop either way).
async fn run_ui(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    snapshot: Arc<Mutex<DashboardSnapshot>>,
    rx: &mut mpsc::Receiver<RefreshMessage>,
    repo: PathBuf,
    no_overlay: bool,
    tx: mpsc::Sender<RefreshMessage>,
) -> Result<()> {
    let mut list_state = ListState::default();
    list_state.select(Some(0));
    let mut show_help = false;

    loop {
        // Draw current frame from the latest snapshot.
        {
            let snap = snapshot.lock().await.clone();
            terminal
                .draw(|f| render(f, &snap, &mut list_state, show_help))
                .context("failed to draw frame")?;
        }

        // Wait for either a key event or a refresh message — whichever fires first.
        tokio::select! {
            biased;
            msg = rx.recv() => {
                if msg.is_none() {
                    break; // Channel closed — bail.
                }
                // Snapshot already updated by background task; just redraw.
            }
            ev = tokio::task::spawn_blocking(|| -> io::Result<Option<Event>> {
                if event::poll(Duration::from_millis(250))? {
                    Ok(Some(event::read()?))
                } else {
                    Ok(None)
                }
            }) => {
                match ev {
                    Ok(Ok(Some(Event::Key(key)))) if key.kind == KeyEventKind::Press => {
                        // Ctrl-C — quit.
                        if key.modifiers.contains(KeyModifiers::CONTROL)
                            && matches!(key.code, KeyCode::Char('c'))
                        {
                            break;
                        }
                        match key.code {
                            KeyCode::Char('q') | KeyCode::Esc => break,
                            KeyCode::Char('?') | KeyCode::F(1) => show_help = !show_help,
                            KeyCode::Char('r') => {
                                // Force a fresh snapshot in the background.
                                let bg_repo = repo.clone();
                                let bg_snapshot = Arc::clone(&snapshot);
                                let bg_tx = tx.clone();
                                tokio::spawn(async move {
                                    let snap = collect_snapshot(&bg_repo, no_overlay).await;
                                    {
                                        let mut g = bg_snapshot.lock().await;
                                        *g = snap;
                                    }
                                    let _ = bg_tx.send(RefreshMessage::Tick).await;
                                });
                            }
                            KeyCode::Down | KeyCode::Char('j') => {
                                let snap = snapshot.lock().await;
                                let len = snap.rows.len();
                                if len > 0 {
                                    let i = list_state.selected().unwrap_or(0);
                                    let next = (i + 1).min(len.saturating_sub(1));
                                    list_state.select(Some(next));
                                }
                            }
                            KeyCode::Up | KeyCode::Char('k') => {
                                let i = list_state.selected().unwrap_or(0);
                                list_state.select(Some(i.saturating_sub(1)));
                            }
                            _ => {}
                        }
                    }
                    Ok(Ok(_)) => {} // Non-key event or no event — ignore.
                    Ok(Err(e)) => return Err(anyhow::anyhow!("terminal poll error: {}", e)),
                    Err(e) => return Err(anyhow::anyhow!("event task join error: {}", e)),
                }
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Snapshot collection
// ---------------------------------------------------------------------------

/// Build a complete dashboard snapshot: list worktrees, then (optionally)
/// overlay PR/CI status from GitHub. Errors during overlay are logged into
/// `last_error` but do not fail the function.
async fn collect_snapshot(repo: &Path, no_overlay: bool) -> DashboardSnapshot {
    let mut snap = DashboardSnapshot {
        rows: Vec::new(),
        last_update: Some(Utc::now()),
        last_error: None,
        overlay_enabled: false,
    };

    let config = match ParsecConfig::load() {
        Ok(c) => c,
        Err(e) => {
            snap.last_error = Some(format!("config load failed: {e}"));
            return snap;
        }
    };

    let manager = match WorktreeManager::new(repo, &config) {
        Ok(m) => m,
        Err(e) => {
            snap.last_error = Some(format!("worktree manager init failed: {e}"));
            return snap;
        }
    };

    let workspaces = match manager.list() {
        Ok(w) => w,
        Err(e) => {
            snap.last_error = Some(format!("worktree list failed: {e}"));
            return snap;
        }
    };

    snap.rows = workspaces.iter().map(workspace_to_row).collect();

    if no_overlay {
        return snap;
    }

    let remote_url = git::run_output(repo, &["remote", "get-url", "origin"])
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let client = match GitHubClient::new(&remote_url, &config) {
        Ok(Some(c)) => c,
        Ok(None) => return snap, // no token — placeholders stay
        Err(e) => {
            snap.last_error = Some(format!("github client init failed: {e}"));
            return snap;
        }
    };
    snap.overlay_enabled = true;

    // Best-effort per-worktree overlay. A single failure logs into `last_error`
    // but never aborts the whole refresh.
    for row in &mut snap.rows {
        match fetch_overlay_for_branch(&client, &row.branch).await {
            Ok(Some(o)) => row.pr = Some(o),
            Ok(None) => {}
            Err(e) => {
                snap.last_error = Some(format!("{}: {e}", row.ticket));
            }
        }
    }

    snap
}

/// Map a [`Workspace`] to a UI-ready row.
fn workspace_to_row(ws: &Workspace) -> DashboardRow {
    DashboardRow {
        ticket: ws.ticket.clone(),
        ticket_title: ws.ticket_title.clone(),
        branch: ws.branch.clone(),
        status: format!("{:?}", ws.status).to_lowercase(),
        pr: None,
    }
}

/// Look up a single PR + CI snapshot for `branch`, or `None` if no open PR.
async fn fetch_overlay_for_branch(
    client: &GitHubClient,
    branch: &str,
) -> Result<Option<PrOverlay>> {
    let pr_num = match client.find_pr_by_branch(branch).await? {
        Some(n) => n,
        None => return Ok(None),
    };
    let status = client.get_pr_status(pr_num).await?;
    Ok(Some(PrOverlay {
        number: status.number,
        title: status.title,
        state: status.state,
        ci_status: status.ci_status,
        review_status: status.review_status,
    }))
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Top-level draw function — splits the frame into three panes plus a status
/// bar and renders each one from the current snapshot.
fn render(
    f: &mut ratatui::Frame,
    snap: &DashboardSnapshot,
    list_state: &mut ListState,
    show_help: bool,
) {
    let area = f.area();

    // Outer layout: top row (2 panes) + bottom pane + status bar.
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(45),
            Constraint::Percentage(50),
            Constraint::Length(1),
        ])
        .split(area);

    let top = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(outer[0]);

    render_worktrees_pane(f, top[0], snap, list_state);
    render_ci_pane(f, top[1], snap);
    render_prs_pane(f, outer[1], snap);
    render_status_bar(f, outer[2], snap);

    if show_help {
        render_help_overlay(f, area);
    }
}

/// Left-top pane: list of every active worktree.
fn render_worktrees_pane(
    f: &mut ratatui::Frame,
    area: Rect,
    snap: &DashboardSnapshot,
    list_state: &mut ListState,
) {
    let items: Vec<ListItem> = snap
        .rows
        .iter()
        .map(|row| {
            let dot = match row.pr.as_ref().map(|p| p.ci_status.as_str()) {
                Some("success") => Span::styled("●", Style::default().fg(Color::Green)),
                Some("failure") => Span::styled("●", Style::default().fg(Color::Red)),
                Some("pending") => Span::styled("●", Style::default().fg(Color::Yellow)),
                _ => Span::styled("●", Style::default().fg(Color::DarkGray)),
            };
            let title = row.ticket_title.as_deref().unwrap_or(row.branch.as_str());
            ListItem::new(Line::from(vec![
                dot,
                Span::raw(" "),
                Span::styled(
                    row.ticket.clone(),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::raw(truncate(title, area.width.saturating_sub(20) as usize)),
                Span::raw("  "),
                Span::styled(
                    format!("[{}]", row.status),
                    Style::default().fg(Color::DarkGray),
                ),
            ]))
        })
        .collect();

    let title = format!("Worktrees ({}) ", snap.rows.len());
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(
            Style::default()
                .bg(Color::Blue)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");

    f.render_stateful_widget(list, area, list_state);
}

/// Right-top pane: per-worktree CI summary (`PR #N · ✓/✗/●`).
fn render_ci_pane(f: &mut ratatui::Frame, area: Rect, snap: &DashboardSnapshot) {
    let lines: Vec<Line> = if snap.rows.is_empty() {
        vec![Line::from(Span::styled(
            "no active worktrees",
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        snap.rows
            .iter()
            .map(|row| match &row.pr {
                Some(pr) => {
                    let (symbol, color) = ci_symbol(&pr.ci_status);
                    Line::from(vec![
                        Span::raw(format!("PR #{:<4} ", pr.number)),
                        Span::styled(symbol, Style::default().fg(color)),
                        Span::raw(" "),
                        Span::raw(pr.ci_status.clone()),
                        Span::raw("  "),
                        Span::styled(
                            truncate(&row.ticket, 12),
                            Style::default().fg(Color::DarkGray),
                        ),
                    ])
                }
                None => Line::from(vec![
                    Span::styled("–      ", Style::default().fg(Color::DarkGray)),
                    Span::raw("no PR  "),
                    Span::styled(
                        truncate(&row.ticket, 12),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]),
            })
            .collect()
    };

    let title = if snap.overlay_enabled {
        "CI Status ".to_string()
    } else {
        "CI Status (no overlay) ".to_string()
    };
    let paragraph =
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(title));
    f.render_widget(paragraph, area);
}

/// Bottom pane: full PR table — PR · title · state · review · CI.
fn render_prs_pane(f: &mut ratatui::Frame, area: Rect, snap: &DashboardSnapshot) {
    let header = Row::new(vec!["PR", "Title", "State", "Review", "CI"]).style(
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    );

    let rows: Vec<Row> = snap
        .rows
        .iter()
        .filter_map(|row| {
            let pr = row.pr.as_ref()?;
            let (symbol, color) = ci_symbol(&pr.ci_status);
            Some(Row::new(vec![
                Cell::from(format!("#{}", pr.number)),
                Cell::from(truncate(&pr.title, 60)),
                Cell::from(pr.state.clone()),
                Cell::from(pr.review_status.clone()),
                Cell::from(Span::styled(
                    format!("{symbol} {}", pr.ci_status),
                    Style::default().fg(color),
                )),
            ]))
        })
        .collect();

    let widths = [
        Constraint::Length(6),
        Constraint::Percentage(45),
        Constraint::Length(10),
        Constraint::Length(20),
        Constraint::Length(14),
    ];
    let title = if rows.is_empty() && !snap.overlay_enabled {
        "PRs (overlay disabled — use parsec dashboard without --no-overlay)"
    } else if rows.is_empty() {
        "PRs (no open PRs for active worktrees)"
    } else {
        "PRs"
    };
    let table = Table::new(rows, widths)
        .header(header)
        .block(Block::default().borders(Borders::ALL).title(title));
    f.render_widget(table, area);
}

/// Bottom status bar: keys + last refresh time.
fn render_status_bar(f: &mut ratatui::Frame, area: Rect, snap: &DashboardSnapshot) {
    let last = snap
        .last_update
        .map(|t| t.with_timezone(&Local).format("%H:%M:%S").to_string())
        .unwrap_or_else(|| "—".to_string());

    let mut spans = vec![
        Span::styled(" q ", Style::default().fg(Color::Black).bg(Color::Gray)),
        Span::raw(" quit  "),
        Span::styled(" r ", Style::default().fg(Color::Black).bg(Color::Gray)),
        Span::raw(" refresh  "),
        Span::styled(" ? ", Style::default().fg(Color::Black).bg(Color::Gray)),
        Span::raw(" help  "),
        Span::styled(
            format!("last update {last}"),
            Style::default().fg(Color::DarkGray),
        ),
    ];
    if let Some(err) = &snap.last_error {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            format!("⚠ {}", truncate(err, 60)),
            Style::default().fg(Color::Red),
        ));
    }
    let bar = Paragraph::new(Line::from(spans));
    f.render_widget(bar, area);
}

/// Centered help overlay shown when the user presses `?`.
fn render_help_overlay(f: &mut ratatui::Frame, area: Rect) {
    let popup_area = centered_rect(60, 40, area);
    f.render_widget(Clear, popup_area);
    let body = vec![
        Line::from(Span::styled(
            "parsec dashboard — keyboard shortcuts",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from("  q / Esc       quit"),
        Line::from("  r             force refresh now"),
        Line::from("  ↑ / ↓ / j / k move selection"),
        Line::from("  ? / F1        toggle this help"),
        Line::from(""),
        Line::from(Span::styled(
            "press ? again to dismiss",
            Style::default().fg(Color::DarkGray),
        )),
    ];
    let p = Paragraph::new(body).block(Block::default().borders(Borders::ALL).title("Help"));
    f.render_widget(p, popup_area);
}

/// Compute a centered `Rect` covering `percent_x` × `percent_y` of `r`.
fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

/// Map a CI status string to (symbol, color) for compact display.
fn ci_symbol(status: &str) -> (&'static str, Color) {
    match status {
        "success" => ("✓", Color::Green),
        "failure" => ("✗", Color::Red),
        "pending" => ("●", Color::Yellow),
        _ => ("–", Color::DarkGray),
    }
}

/// Truncate a string at `max` characters (display-width approximation).
fn truncate(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

// ---------------------------------------------------------------------------
// Terminal lifecycle (RAII)
// ---------------------------------------------------------------------------

/// RAII guard for the alternate screen + raw mode pair. The destructor runs on
/// panic as well as normal exit, so an unexpected error never leaves the
/// user's terminal in a corrupted state.
struct TerminalGuard {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl TerminalGuard {
    fn new() -> Result<Self> {
        enable_raw_mode().context("failed to enable raw mode")?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen).context("failed to enter alternate screen")?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend).context("failed to create ratatui terminal")?;
        Ok(Self { terminal })
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        // Best-effort restore — we can't return errors from Drop.
        let _ = disable_raw_mode();
        let _ = execute!(self.terminal.backend_mut(), LeaveAlternateScreen);
        let _ = self.terminal.show_cursor();
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_short_string_unchanged() {
        assert_eq!(truncate("abc", 10), "abc");
    }

    #[test]
    fn truncate_long_string_adds_ellipsis() {
        let out = truncate("abcdefghij", 5);
        assert_eq!(out.chars().count(), 5);
        assert!(out.ends_with('…'));
    }

    #[test]
    fn truncate_zero_returns_empty() {
        assert_eq!(truncate("anything", 0), "");
    }

    #[test]
    fn ci_symbol_known_states() {
        assert_eq!(ci_symbol("success").0, "✓");
        assert_eq!(ci_symbol("failure").0, "✗");
        assert_eq!(ci_symbol("pending").0, "●");
        assert_eq!(ci_symbol("unknown").0, "–");
    }

    #[test]
    fn workspace_to_row_preserves_fields() {
        use chrono::Utc;
        let ws = Workspace {
            ticket: "CL-42".to_string(),
            path: std::path::PathBuf::from("/tmp/x"),
            branch: "feat/cl-42".to_string(),
            base_branch: "develop".to_string(),
            created_at: Utc::now(),
            ticket_title: Some("Fix login bug".to_string()),
            status: crate::worktree::WorkspaceStatus::Active,
            parent_ticket: None,
        };
        let row = workspace_to_row(&ws);
        assert_eq!(row.ticket, "CL-42");
        assert_eq!(row.branch, "feat/cl-42");
        assert_eq!(row.status, "active");
        assert!(row.pr.is_none());
    }

    #[test]
    fn snapshot_default_has_no_rows() {
        let snap = DashboardSnapshot::default();
        assert!(snap.rows.is_empty());
        assert!(snap.last_update.is_none());
        assert!(!snap.overlay_enabled);
    }
}
