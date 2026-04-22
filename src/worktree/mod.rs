mod lifecycle;
mod manager;

pub use lifecycle::{ParsecState, ShipResult, Workspace, WorkspaceStatus};
pub use manager::WorktreeManager;

use anyhow::{bail, Result};

/// Validate a ticket identifier for safety.
///
/// Allowed formats:
///   - Jira:   `[A-Z][A-Z0-9_]{1,19}-[1-9][0-9]*`  (e.g. PROJ-123, R2D2-1)
///   - GitHub/GitLab: `#?[1-9][0-9]*`                (e.g. 42, #42)
///   - Generic: alphanumeric with hyphens/underscores (e.g. my-ticket-1)
///
/// Rejects path traversal, shell meta-characters, and git ref-unsafe sequences.
pub fn validate_ticket_id(ticket: &str) -> Result<()> {
    if ticket.is_empty() {
        bail!("Ticket ID cannot be empty.");
    }

    if ticket.len() > 100 {
        bail!("Ticket ID too long (max 100 characters): {}", ticket);
    }

    // Block path traversal
    if ticket.contains("..") || ticket.contains('/') || ticket.contains('\\') {
        bail!("Ticket ID contains unsafe path characters: {}", ticket);
    }

    // Block null bytes and control characters
    if ticket.chars().any(|c| c.is_control()) {
        bail!("Ticket ID contains control characters: {}", ticket);
    }

    // Block whitespace
    if ticket.chars().any(|c| c.is_whitespace()) {
        bail!("Ticket ID contains whitespace: {}", ticket);
    }

    // Block git ref-unsafe characters: ~, ^, :, ?, *, [, @{
    const UNSAFE_CHARS: &[char] = &['~', '^', ':', '?', '*', '[', ' '];
    if let Some(c) = ticket.chars().find(|c| UNSAFE_CHARS.contains(c)) {
        bail!(
            "Ticket ID contains git-unsafe character '{}': {}",
            c,
            ticket
        );
    }
    if ticket.contains("@{") {
        bail!("Ticket ID contains git-unsafe sequence '@{{}}': {}", ticket);
    }

    // Block shell meta-characters
    const SHELL_CHARS: &[char] = &[
        '$', '`', '!', '&', '|', ';', '(', ')', '{', '}', '<', '>', '"', '\'',
    ];
    if let Some(c) = ticket.chars().find(|c| SHELL_CHARS.contains(c)) {
        bail!(
            "Ticket ID contains shell-unsafe character '{}': {}",
            c,
            ticket
        );
    }

    // Must start with alphanumeric or #
    let first = ticket.chars().next().unwrap();
    if !first.is_alphanumeric() && first != '#' {
        bail!(
            "Ticket ID must start with a letter, digit, or '#': {}",
            ticket
        );
    }

    Ok(())
}
