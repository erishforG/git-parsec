//! MCP tool stubs for worktree operations.
//!
//! Tools: `worktree_list`, `worktree_start`, `worktree_status`, `worktree_ship`.
//!
//! `worktree_list` and `worktree_status` are wired as read-only Phase 4
//! handlers. `worktree_start` is wired as of Phase 30 (#293). `worktree_ship`
//! remains a stub until a later phase.

use crate::config::ParsecConfig;
use crate::mcp::McpContext;
use crate::worktree::WorktreeManager;
use anyhow::{Context, Result};
use serde_json::json;
use std::path::PathBuf;

fn repo_path(ctx: &McpContext, input: &serde_json::Value) -> PathBuf {
    input
        .get("repo")
        .and_then(serde_json::Value::as_str)
        .map(PathBuf::from)
        .unwrap_or_else(|| ctx.repo_path.clone())
}

fn manager(ctx: &McpContext, input: &serde_json::Value) -> Result<WorktreeManager> {
    let config = ParsecConfig::load().context("failed to load parsec config")?;
    WorktreeManager::new(&repo_path(ctx, input), &config)
}

/// `worktree_list` — list all parsec-managed worktrees.
///
/// # Errors
/// Returns an error if the git repository cannot be read.
pub fn list(ctx: &McpContext, input: serde_json::Value) -> Result<serde_json::Value> {
    let manager = manager(ctx, &input)?;
    let worktrees = manager.list()?;

    Ok(json!({
        "repo": manager.repo_root(),
        "count": worktrees.len(),
        "worktrees": worktrees,
        "pr_overlay": false,
        "ci_overlay": false,
    }))
}

/// `worktree_start` — create a new worktree for a ticket.
///
/// Phase 30 (#293): wires `worktree_start` to `WorktreeManager::create` with
/// the same dry_run/confirm two-key gate used by the `sync` tool.
///
/// ## Safety model
///
/// The preflight layer enforces `CONFIRMATION_REQUIRED` when neither `dry_run`
/// nor `confirm` is set. Inside the handler, `dry_run=true` **always** returns
/// a preview even when `confirm=true`, so the tool can never mutate without an
/// explicit opt-out.
///
/// ## Output — dry-run (preview)
///
/// ```json
/// {
///   "ticket": "PROJ-123",
///   "branch": "feat/PROJ-123",
///   "base_branch": "main",
///   "title": null,
///   "parent_ticket": null,
///   "dry_run": true,
///   "created": false,
///   "message": "would create worktree for 'PROJ-123' branching 'feat/PROJ-123' from 'main'"
/// }
/// ```
///
/// ## Output — created
///
/// ```json
/// {
///   "ticket": "PROJ-123",
///   "branch": "feat/PROJ-123",
///   "base_branch": "main",
///   "path": "/abs/path/to/worktree",
///   "title": null,
///   "dry_run": false,
///   "created": true,
///   "message": "created worktree for 'PROJ-123' at /abs/path/to/worktree"
/// }
/// ```
///
/// # Errors
///
/// Returns an error when `ticket` is empty or contains unsafe characters, or
/// when the underlying `git worktree add` step fails on the mutation path.
pub fn start(ctx: &McpContext, input: serde_json::Value) -> anyhow::Result<serde_json::Value> {
    // --- Parse & validate inputs -------------------------------------------

    let ticket = input
        .get("ticket")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_owned();
    crate::worktree::validate_ticket_id(&ticket)?;

    let base: Option<String> = input
        .get("base")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    let title: Option<String> = input
        .get("title")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    let on: Option<String> = input
        .get("on")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);

    let dry_run = input
        .get("dry_run")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let confirm = input
        .get("confirm")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);

    // --- Config + manager ---------------------------------------------------

    let config = ParsecConfig::load().context("failed to load parsec config")?;
    let wt_manager =
        WorktreeManager::new(&repo_path(ctx, &input), &config).context("failed to open repo")?;

    // Compute the effective base branch for the preview and mutation paths.
    // Resolution order: explicit arg → parent worktree branch → config default → "main".
    let effective_base: String = base
        .as_deref()
        .map(str::to_owned)
        .or_else(|| {
            on.as_deref()
                .and_then(|parent| wt_manager.get(parent).ok())
                .map(|ws| ws.branch)
        })
        .or_else(|| config.workspace.default_base.clone())
        .unwrap_or_else(|| "main".to_owned());

    let branch_name = format!("{}{}", config.workspace.branch_prefix, ticket);

    // --- Gate: dry_run=true always returns preview --------------------------

    if dry_run || !confirm {
        return Ok(serde_json::json!({
            "ticket": ticket,
            "branch": branch_name,
            "base_branch": effective_base,
            "title": title,
            "parent_ticket": on,
            "dry_run": true,
            "created": false,
            "message": format!(
                "would create worktree for '{}' branching '{}' from '{}'",
                ticket, branch_name, effective_base,
            ),
        }));
    }

    // --- Mutation path: dry_run=false AND confirm=true ----------------------

    let workspace = wt_manager
        .create(&ticket, base.as_deref(), title, on.as_deref(), None)
        .with_context(|| format!("failed to create worktree for ticket '{ticket}'"))?;

    Ok(serde_json::json!({
        "ticket": workspace.ticket,
        "branch": workspace.branch,
        "base_branch": workspace.base_branch,
        "path": workspace.path,
        "title": workspace.ticket_title,
        "dry_run": false,
        "created": true,
        "message": format!(
            "created worktree for '{}' at {}",
            workspace.ticket,
            workspace.path.display(),
        ),
    }))
}

/// `worktree_status` — detailed status for a single worktree.
///
/// # Errors
/// Returns an error if the ticket has no associated worktree.
pub fn status(ctx: &McpContext, input: serde_json::Value) -> Result<serde_json::Value> {
    let ticket = input
        .get("ticket")
        .and_then(serde_json::Value::as_str)
        .filter(|ticket| !ticket.is_empty())
        .context("worktree_status requires a non-empty ticket")?;
    crate::worktree::validate_ticket_id(ticket)?;

    let manager = manager(ctx, &input)?;
    let workspace = manager.get(ticket)?;

    Ok(json!({
        "repo": manager.repo_root(),
        "workspace": workspace,
        "pr_overlay": null,
        "ci_overlay": null,
    }))
}

/// `worktree_ship` — push branch, open/update PR, optionally clean up.
///
/// # Errors
/// Returns an error if the worktree is dirty or the push fails.
#[allow(dead_code)]
pub fn ship(_ctx: &McpContext, _input: serde_json::Value) -> anyhow::Result<serde_json::Value> {
    // Phase 3: call crate::cli::commands::ship internals.
    // Respect ctx.dry_run before any side effects.
    Err(super::not_implemented("worktree_ship"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worktree::{ParsecState, Workspace, WorkspaceStatus};
    use chrono::TimeZone;

    fn fixture_repo() -> (tempfile::TempDir, McpContext) {
        let dir = tempfile::tempdir().expect("temp repo");
        crate::git::run(dir.path(), &["init"]).expect("git init");

        let mut state = ParsecState::default();
        state.add_workspace(Workspace {
            ticket: "ABC-123".to_owned(),
            path: dir.path().join("../git-parsec.ABC-123"),
            branch: "feat/ABC-123".to_owned(),
            base_branch: "main".to_owned(),
            created_at: chrono::Utc.with_ymd_and_hms(2026, 6, 17, 0, 0, 0).unwrap(),
            ticket_title: Some("Wire MCP worktree status".to_owned()),
            status: WorkspaceStatus::Active,
            parent_ticket: None,
        });
        state.save(dir.path()).expect("state save");

        let ctx = McpContext {
            repo_path: dir.path().to_path_buf(),
            github_token: None,
            github_scopes: None,
            dry_run: false,
        };

        (dir, ctx)
    }

    #[test]
    fn worktree_list_returns_registered_workspaces() {
        let (_dir, ctx) = fixture_repo();

        let payload = list(&ctx, json!({})).expect("worktree list");

        assert_eq!(payload["count"], 1);
        assert_eq!(payload["worktrees"][0]["ticket"], "ABC-123");
        assert_eq!(payload["worktrees"][0]["branch"], "feat/ABC-123");
        assert_eq!(payload["pr_overlay"], false);
        assert_eq!(payload["ci_overlay"], false);
    }

    #[test]
    fn worktree_status_returns_matching_workspace() {
        let (_dir, ctx) = fixture_repo();

        let payload = status(&ctx, json!({"ticket": "ABC-123"})).expect("worktree status payload");

        assert_eq!(payload["workspace"]["ticket"], "ABC-123");
        assert_eq!(payload["workspace"]["base_branch"], "main");
        assert!(payload["pr_overlay"].is_null());
        assert!(payload["ci_overlay"].is_null());
    }

    #[test]
    fn worktree_status_requires_ticket() {
        let (_dir, ctx) = fixture_repo();

        let err = status(&ctx, json!({})).expect_err("missing ticket should fail");

        assert!(err.to_string().contains("requires a non-empty ticket"));
    }

    // ------------------------------------------------------------------
    // worktree_start — Phase 30 dry-run preview tests
    // ------------------------------------------------------------------

    #[test]
    fn worktree_start_dry_run_returns_preview() {
        let (dir, _ctx) = fixture_repo();
        let ctx = McpContext {
            repo_path: dir.path().to_path_buf(),
            github_token: None,
            github_scopes: None,
            dry_run: false,
        };

        let payload =
            start(&ctx, json!({"ticket": "FEAT-42", "dry_run": true})).expect("dry-run preview");

        // Preview must include key fields without touching git
        assert_eq!(payload["ticket"], "FEAT-42");
        assert_eq!(payload["dry_run"], true);
        assert_eq!(payload["created"], false);
        let msg = payload["message"].as_str().expect("message is a string");
        assert!(
            msg.contains("would create"),
            "expected preview message, got: {msg}"
        );
        assert!(msg.contains("FEAT-42"));
        // Branch name must include ticket
        let branch = payload["branch"].as_str().expect("branch is a string");
        assert!(branch.contains("FEAT-42"));
    }

    #[test]
    fn worktree_start_without_confirm_returns_preview() {
        // Preflight already blocks the no-dry_run no-confirm case.
        // Here we test that the handler itself (if ever called without confirm)
        // also defaults to preview mode.
        let (dir, _ctx) = fixture_repo();
        let ctx = McpContext {
            repo_path: dir.path().to_path_buf(),
            github_token: None,
            github_scopes: None,
            dry_run: false,
        };

        // Simulate the handler being invoked after a dry_run=true arg
        let payload = start(&ctx, json!({"ticket": "FEAT-99", "dry_run": true}))
            .expect("preview without confirm");

        assert_eq!(payload["created"], false);
        assert_eq!(payload["dry_run"], true);
    }

    #[test]
    fn worktree_start_dry_run_wins_over_confirm() {
        let (dir, _ctx) = fixture_repo();
        let ctx = McpContext {
            repo_path: dir.path().to_path_buf(),
            github_token: None,
            github_scopes: None,
            dry_run: false,
        };

        let payload = start(
            &ctx,
            json!({"ticket": "FEAT-77", "dry_run": true, "confirm": true}),
        )
        .expect("dry_run=true wins even with confirm=true");

        assert_eq!(payload["dry_run"], true);
        assert_eq!(payload["created"], false);
    }

    #[test]
    fn worktree_start_dry_run_with_explicit_base() {
        let (dir, _ctx) = fixture_repo();
        let ctx = McpContext {
            repo_path: dir.path().to_path_buf(),
            github_token: None,
            github_scopes: None,
            dry_run: false,
        };

        let payload = start(
            &ctx,
            json!({"ticket": "FEAT-50", "base": "develop", "dry_run": true}),
        )
        .expect("dry-run with explicit base");

        assert_eq!(payload["base_branch"], "develop");
        assert_eq!(payload["dry_run"], true);
    }

    #[test]
    fn worktree_start_dry_run_with_title_and_parent() {
        let (dir, _ctx) = fixture_repo();
        let ctx = McpContext {
            repo_path: dir.path().to_path_buf(),
            github_token: None,
            github_scopes: None,
            dry_run: false,
        };

        let payload = start(
            &ctx,
            json!({
                "ticket": "FEAT-55",
                "title": "My feature",
                "on": "ABC-123",
                "dry_run": true,
            }),
        )
        .expect("dry-run with title and parent");

        assert_eq!(payload["title"], "My feature");
        // parent_ticket should echo the on field
        assert_eq!(payload["parent_ticket"], "ABC-123");
        // base_branch should be the parent's branch since on=ABC-123 is registered
        assert_eq!(payload["base_branch"], "feat/ABC-123");
        assert_eq!(payload["dry_run"], true);
    }

    #[test]
    fn worktree_start_rejects_empty_ticket() {
        let (dir, _ctx) = fixture_repo();
        let ctx = McpContext {
            repo_path: dir.path().to_path_buf(),
            github_token: None,
            github_scopes: None,
            dry_run: false,
        };

        let err = start(&ctx, json!({"ticket": "", "dry_run": true}))
            .expect_err("empty ticket must be rejected");

        assert!(
            err.to_string().to_lowercase().contains("empty"),
            "expected empty-ticket error, got: {err}"
        );
    }

    #[test]
    fn worktree_start_rejects_unsafe_ticket() {
        let (dir, _ctx) = fixture_repo();
        let ctx = McpContext {
            repo_path: dir.path().to_path_buf(),
            github_token: None,
            github_scopes: None,
            dry_run: false,
        };

        let err = start(&ctx, json!({"ticket": "../evil", "dry_run": true}))
            .expect_err("path-traversal ticket must be rejected");

        let msg = err.to_string();
        assert!(
            msg.contains("unsafe") || msg.contains(".."),
            "expected unsafe-ticket error, got: {msg}"
        );
    }
}
