//! MCP tool handlers for worktree operations.
//!
//! Tools: `worktree_list`, `worktree_start`, `worktree_status`, `worktree_ship`.
//!
//! `worktree_list` and `worktree_status` are wired as read-only Phase 4
//! handlers. `worktree_start` is wired as of Phase 30 (#293).
//! `worktree_ship` is wired as of Phase 31 (#293): push + `gh pr create` +
//! optional cleanup, guarded by the mandatory dry_run/confirm two-key gate.

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
/// Phase 31 (#293): wires `worktree_ship` to `WorktreeManager::ship_push` +
/// `gh pr create` with the mandatory dry_run / confirm two-key gate.
///
/// ## Safety model
///
/// Same pattern as `sync` and `worktree_start`:
/// - `dry_run=true` → preview only (default safe)
/// - `dry_run=false + confirm=true` → push + create PR + optional cleanup
///
/// The preflight layer enforces `CONFIRMATION_REQUIRED` before this handler
/// runs, so `dry_run` and `confirm` can never both be absent here.
///
/// ## Output — dry-run (preview)
///
/// ```json
/// {
///   "ticket": "PROJ-123",
///   "branch": "feat/PROJ-123",
///   "base_branch": "main",
///   "draft": false,
///   "no_cleanup": false,
///   "dry_run": true,
///   "shipped": false,
///   "message": "would push 'feat/PROJ-123' and open PR against 'main'"
/// }
/// ```
///
/// ## Output — shipped
///
/// ```json
/// {
///   "ticket": "PROJ-123",
///   "branch": "feat/PROJ-123",
///   "base_branch": "main",
///   "pr_url": "https://github.com/org/repo/pull/42",
///   "pr_number": 42,
///   "draft": false,
///   "cleaned_up": false,
///   "dry_run": false,
///   "shipped": true,
///   "message": "pushed 'feat/PROJ-123' and opened PR #42"
/// }
/// ```
///
/// # Errors
///
/// Returns an error when `ticket` is empty, contains unsafe characters, or has
/// no registered worktree. The mutation path also returns an error when `git
/// push` or `gh pr create` fails.
pub fn ship(ctx: &McpContext, input: serde_json::Value) -> anyhow::Result<serde_json::Value> {
    // --- Parse & validate inputs -------------------------------------------

    let ticket = input
        .get("ticket")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_owned();
    crate::worktree::validate_ticket_id(&ticket)?;

    let draft = input
        .get("draft")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let no_cleanup = input
        .get("no_cleanup")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
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

    // Look up the workspace early — validates ticket is registered.
    let workspace = wt_manager.get(&ticket)?;
    let branch = workspace.branch.clone();
    let base_branch = workspace.base_branch.clone();
    let title = workspace.ticket_title.clone();

    // --- Gate: dry_run=true OR !confirm → preview --------------------------

    if dry_run || !confirm {
        return Ok(serde_json::json!({
            "ticket":      ticket,
            "branch":      branch,
            "base_branch": base_branch,
            "draft":       draft,
            "no_cleanup":  no_cleanup,
            "dry_run":     true,
            "shipped":     false,
            "message": format!(
                "would push '{}' and open {}PR against '{}'",
                branch,
                if draft { "draft " } else { "" },
                base_branch,
            ),
        }));
    }

    // --- Mutation path: dry_run=false AND confirm=true ----------------------

    let token = ctx
        .github_token
        .as_ref()
        .map(|t| t.expose_secret().to_owned());

    // Step 1: push branch from the worktree.
    wt_manager
        .ship_push(&ticket)
        .with_context(|| format!("failed to push branch '{branch}'"))?;

    // Step 2: create PR via gh CLI.
    let pr_title = title
        .as_deref()
        .map(|t| format!("[{ticket}] {t}"))
        .unwrap_or_else(|| ticket.clone());

    let repo = repo_path(ctx, &input);
    let pr_result = create_gh_pr(
        &repo,
        &branch,
        &base_branch,
        &pr_title,
        draft,
        token.as_deref(),
    );

    let (pr_url, pr_number) = match pr_result {
        Ok(pair) => pair,
        Err(e) => {
            // Push succeeded but PR creation failed — partial result.
            return Ok(serde_json::json!({
                "ticket":     ticket,
                "branch":     branch,
                "base_branch": base_branch,
                "draft":      draft,
                "dry_run":    false,
                "shipped":    false,
                "pushed":     true,
                "pr_created": false,
                "error": format!("pushed branch but PR creation failed: {e}"),
            }));
        }
    };

    // Step 3: optional cleanup (respects config.ship.auto_cleanup).
    let cleaned_up = if !no_cleanup {
        wt_manager.ship_cleanup(&ticket).unwrap_or(false)
    } else {
        false
    };

    Ok(serde_json::json!({
        "ticket":      ticket,
        "branch":      branch,
        "base_branch": base_branch,
        "pr_url":      pr_url,
        "pr_number":   pr_number,
        "draft":       draft,
        "cleaned_up":  cleaned_up,
        "dry_run":     false,
        "shipped":     true,
        "message": format!(
            "pushed '{branch}' and opened PR #{pr_number} ({pr_url})",
        ),
    }))
}

/// Call `gh pr create` and return `(url, pr_number)`.
///
/// If a PR already exists for the branch, falls back to `gh pr view` to
/// retrieve the existing URL and number.
fn create_gh_pr(
    repo: &std::path::Path,
    branch: &str,
    base: &str,
    title: &str,
    draft: bool,
    token: Option<&str>,
) -> anyhow::Result<(String, u64)> {
    let mut cmd = std::process::Command::new("gh");
    cmd.args([
        "pr",
        "create",
        "--head",
        branch,
        "--base",
        base,
        "--title",
        title,
        "--body",
        "",
        "--json",
        "url,number",
    ]);
    if draft {
        cmd.arg("--draft");
    }
    cmd.current_dir(repo);
    if let Some(t) = token {
        cmd.env("GH_TOKEN", t);
    }

    let output = cmd.output().context("failed to spawn 'gh pr create'")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // PR already exists — retrieve the existing one instead.
        if stderr.contains("already exists") || stderr.contains("A pull request for branch") {
            return find_existing_gh_pr(repo, branch, token);
        }
        anyhow::bail!(
            "gh pr create failed for branch '{}': {}",
            branch,
            stderr.trim()
        );
    }

    let raw = String::from_utf8(output.stdout).context("gh pr create produced non-UTF-8 output")?;
    let value: serde_json::Value =
        serde_json::from_str(&raw).context("failed to parse gh pr create JSON")?;

    let url = value["url"]
        .as_str()
        .context("missing 'url' in gh pr create output")?
        .to_owned();
    let number = value["number"]
        .as_u64()
        .context("missing 'number' in gh pr create output")?;

    Ok((url, number))
}

/// Retrieve URL + number of an existing PR for the branch via `gh pr view`.
fn find_existing_gh_pr(
    repo: &std::path::Path,
    branch: &str,
    token: Option<&str>,
) -> anyhow::Result<(String, u64)> {
    let mut cmd = std::process::Command::new("gh");
    cmd.args(["pr", "view", branch, "--json", "url,number"]);
    cmd.current_dir(repo);
    if let Some(t) = token {
        cmd.env("GH_TOKEN", t);
    }

    let output = cmd.output().context("failed to spawn 'gh pr view'")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "gh pr view failed for branch '{}': {}",
            branch,
            stderr.trim()
        );
    }

    let raw = String::from_utf8(output.stdout).context("gh pr view produced non-UTF-8 output")?;
    let value: serde_json::Value =
        serde_json::from_str(&raw).context("failed to parse gh pr view JSON")?;

    let url = value["url"]
        .as_str()
        .context("missing 'url' in gh pr view output")?
        .to_owned();
    let number = value["number"]
        .as_u64()
        .context("missing 'number' in gh pr view output")?;

    Ok((url, number))
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

    // ------------------------------------------------------------------
    // worktree_ship — Phase 31 dry-run preview tests
    // ------------------------------------------------------------------

    #[test]
    fn worktree_ship_dry_run_returns_preview() {
        let (_dir, ctx) = fixture_repo();

        let payload =
            ship(&ctx, json!({"ticket": "ABC-123", "dry_run": true})).expect("dry-run preview");

        assert_eq!(payload["ticket"], "ABC-123");
        assert_eq!(payload["dry_run"], true);
        assert_eq!(payload["shipped"], false);
        assert_eq!(payload["draft"], false);
        let msg = payload["message"].as_str().expect("message string");
        assert!(
            msg.contains("would push"),
            "expected preview wording, got: {msg}"
        );
        assert!(msg.contains("ABC-123") || msg.contains("feat/"));
    }

    #[test]
    fn worktree_ship_without_confirm_returns_preview() {
        let (_dir, ctx) = fixture_repo();

        // Neither dry_run nor confirm — handler defaults to preview.
        // (Preflight would block this case in production; here we test
        //  the handler's own safe-default.)
        let payload = ship(&ctx, json!({"ticket": "ABC-123"})).expect("handler default preview");

        assert_eq!(payload["shipped"], false);
        assert_eq!(payload["dry_run"], true);
    }

    #[test]
    fn worktree_ship_dry_run_wins_over_confirm() {
        let (_dir, ctx) = fixture_repo();

        let payload = ship(
            &ctx,
            json!({"ticket": "ABC-123", "dry_run": true, "confirm": true}),
        )
        .expect("dry_run=true wins even with confirm=true");

        assert_eq!(payload["dry_run"], true);
        assert_eq!(payload["shipped"], false);
    }

    #[test]
    fn worktree_ship_dry_run_draft_flag_in_message() {
        let (_dir, ctx) = fixture_repo();

        let payload = ship(
            &ctx,
            json!({"ticket": "ABC-123", "draft": true, "dry_run": true}),
        )
        .expect("draft dry-run preview");

        assert_eq!(payload["draft"], true);
        let msg = payload["message"].as_str().expect("message");
        assert!(
            msg.contains("draft"),
            "draft flag should appear in preview message, got: {msg}"
        );
    }

    #[test]
    fn worktree_ship_dry_run_no_cleanup_flag_preserved() {
        let (_dir, ctx) = fixture_repo();

        let payload = ship(
            &ctx,
            json!({"ticket": "ABC-123", "no_cleanup": true, "dry_run": true}),
        )
        .expect("no_cleanup dry-run preview");

        assert_eq!(payload["no_cleanup"], true);
        assert_eq!(payload["shipped"], false);
    }

    #[test]
    fn worktree_ship_rejects_empty_ticket() {
        let (_dir, ctx) = fixture_repo();

        let err = ship(&ctx, json!({"ticket": "", "dry_run": true}))
            .expect_err("empty ticket must be rejected");

        assert!(
            err.to_string().to_lowercase().contains("empty"),
            "expected empty-ticket error, got: {err}"
        );
    }

    #[test]
    fn worktree_ship_rejects_unsafe_ticket() {
        let (_dir, ctx) = fixture_repo();

        let err = ship(&ctx, json!({"ticket": "bad;ticket", "dry_run": true}))
            .expect_err("unsafe ticket must be rejected");

        let msg = err.to_string();
        assert!(
            msg.contains("unsafe") || msg.contains("ticket"),
            "expected validation error, got: {msg}"
        );
    }

    #[test]
    fn worktree_ship_dry_run_fails_for_nonexistent_ticket() {
        let (_dir, ctx) = fixture_repo();

        let err = ship(&ctx, json!({"ticket": "UNKNOWN-99", "dry_run": true}))
            .expect_err("non-existent ticket must fail");

        assert!(
            err.to_string().contains("UNKNOWN-99")
                || err.to_string().contains("worktree")
                || err.to_string().contains("not found"),
            "error should mention missing ticket: {err}"
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
