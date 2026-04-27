use std::path::Path;

use anyhow::{Context, Result};

use crate::config::ParsecConfig;
use crate::errors::ErrorCode;
use crate::git;
use crate::github;
use crate::gitlab;
use crate::output::{self, Mode};
use crate::tracker;
use crate::worktree::WorktreeManager;

#[allow(clippy::too_many_arguments)]
pub async fn ship(
    repo: &Path,
    ticket: &str,
    draft: bool,
    no_pr: bool,
    base_override: Option<String>,
    title_override: Option<String>,
    skip_hooks: bool,
    reviewers: Vec<String>,
    labels: Vec<String>,
    mode: Mode,
) -> Result<()> {
    let mut config = ParsecConfig::load()?;
    let manager = WorktreeManager::new(repo, &config)?;
    config.resolve_for_repo(manager.repo_root());

    // Merge CLI args with config defaults (CLI overrides when non-empty)
    let effective_reviewers = if reviewers.is_empty() {
        config.ship.default_reviewers.clone()
    } else {
        reviewers
    };
    let effective_labels = if labels.is_empty() {
        config.ship.default_labels.clone()
    } else {
        labels
    };

    // Run pre-ship hooks before pushing
    if !skip_hooks && !config.hooks.pre_ship.is_empty() {
        let workspace = manager.get(ticket)?;
        for hook_cmd in &config.hooks.pre_ship {
            if mode == Mode::Human {
                eprintln!("Running pre-ship hook: {}", hook_cmd);
            }
            let output = std::process::Command::new("sh")
                .args(["-c", hook_cmd])
                .current_dir(&workspace.path)
                .output()
                .with_context(|| format!("Failed to spawn hook: {}", hook_cmd))?;
            if !output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                bail_code!(
                    ErrorCode::E008,
                    "pre-ship hook failed: {}\n{}{}",
                    hook_cmd,
                    if stdout.is_empty() {
                        String::new()
                    } else {
                        format!("stdout:\n{}", stdout)
                    },
                    if stderr.is_empty() {
                        String::new()
                    } else {
                        format!("stderr:\n{}", stderr)
                    }
                );
            }
        }
    }

    // Phase 1: Push only (don't clean up yet)
    // Idempotency: if workspace is already gone (cleaned up after a prior ship),
    // treat push as a no-op — the branch is already on the remote.
    let mut result = match manager.ship_push(ticket) {
        Ok(r) => r,
        Err(e) => {
            // If workspace not in state, check if a PR already exists for this ticket.
            // If so, the ticket was already shipped — succeed silently.
            let remote_url = git::get_remote_url(manager.repo_root()).unwrap_or_default();
            if !remote_url.is_empty() {
                // Try to find a PR via oplog branch info
                let oplog = crate::oplog::OpLog::load(manager.repo_root()).unwrap_or_default();
                let branch = oplog
                    .get_entries(Some(ticket))
                    .into_iter()
                    .rev()
                    .filter(|entry| matches!(entry.op, crate::oplog::OpKind::Ship))
                    .find_map(|entry| entry.undo_info.as_ref().and_then(|u| u.branch.clone()));
                if let Some(ref br) = branch {
                    let already_shipped =
                        if let Ok(Some(gh)) = github::GitHubClient::new(&remote_url, &config) {
                            matches!(gh.find_pr_by_branch(br).await, Ok(Some(_)))
                        } else {
                            false
                        };
                    if already_shipped {
                        if mode == Mode::Human {
                            eprintln!("note: ticket {} already shipped. Nothing to do.", ticket);
                        }
                        return Ok(());
                    }
                }
            }
            return Err(e);
        }
    };

    // Resolve base branch: --base CLI > config default_base > worktree's base_branch
    if let Some(base) = base_override {
        result.base_branch = base;
    } else if let Some(ref default_base) = config.ship.default_base {
        result.base_branch = default_base.clone();
    }
    // else: keep the worktree's original base_branch

    // Policy guard: check if the target branch is allowed
    if !config.policy.is_allowed_target(&result.base_branch) {
        anyhow::bail!(
            "Policy violation: shipping to '{}' is not allowed.\n  \
             Protected branches: {:?}\n  \
             Allowed targets: {:?}\n  \
             Use --base to specify a different target branch.",
            result.base_branch,
            config.policy.protected_branches,
            config.policy.allowed_ship_targets,
        );
    }

    // Phase 2: Create PR/MR (async)
    let mut pr_failed = false;
    if !no_pr && config.ship.auto_pr {
        let (ticket_title, ticket_url) =
            match tracker::fetch_ticket(&config, ticket, Some(manager.repo_root())).await {
                Ok(Some(t)) => (Some(t.title), t.url),
                _ => (None, None),
            };

        // Priority: --title flag > fresh tracker fetch > stored workspace title
        let effective_title = ticket_title.as_deref().or(result.ticket_title.as_deref());

        let pr_title = if let Some(ref t) = title_override {
            t.clone()
        } else {
            effective_title
                .map(|t| format!("[{}] {}", result.ticket, t))
                .unwrap_or_else(|| result.ticket.clone())
        };

        // Gather stack context for PR body (#234)
        let stack_info = gather_stack_info(&manager, ticket);

        let pr_body = build_pr_body(
            &result.ticket,
            effective_title,
            ticket_url.as_deref(),
            stack_info.as_ref(),
        );

        let remote_url = git::get_remote_url(manager.repo_root());
        if let Ok(ref remote_url) = remote_url {
            if let Some(gh) = github::GitHubClient::new(remote_url, &config)? {
                // Check if a PR already exists for this branch (#98)
                if let Ok(Some(existing_pr)) = gh.find_pr_by_branch(&result.branch).await {
                    let r = gh.remote();
                    let pr_url = format!(
                        "https://{}/{}/{}/pull/{}",
                        r.host, r.owner, r.repo, existing_pr
                    );
                    result.pr_url = Some(pr_url);
                } else {
                    match gh
                        .create_pr(
                            &result.branch,
                            &result.base_branch,
                            &pr_title,
                            &pr_body,
                            draft || config.ship.draft,
                        )
                        .await
                    {
                        Ok(pr) => {
                            // Request reviewers if specified
                            if !effective_reviewers.is_empty() {
                                if let Err(e) =
                                    gh.request_reviewers(pr.number, &effective_reviewers).await
                                {
                                    eprintln!("warning: failed to request reviewers: {e}");
                                }
                            }
                            // Add labels if specified
                            if !effective_labels.is_empty() {
                                if let Err(e) = gh.add_labels(pr.number, &effective_labels).await {
                                    eprintln!("warning: failed to add labels: {e}");
                                }
                            }
                            result.pr_url = Some(pr.url);
                        }
                        Err(e) => {
                            eprintln!("error: PR creation failed: {e}");
                            pr_failed = true;
                        }
                    }
                }
            } else {
                // No GitHub token — try GitLab
                match gitlab::create_mr(
                    remote_url,
                    &result.branch,
                    &result.base_branch,
                    &pr_title,
                    &pr_body,
                    draft || config.ship.draft,
                )
                .await
                {
                    Ok(Some(mr)) => {
                        result.pr_url = Some(mr.url);
                    }
                    Ok(None) => {
                        eprintln!(
                            "note: PR/MR creation skipped — no token found.\n      \
                             Set PARSEC_GITHUB_TOKEN or PARSEC_GITLAB_TOKEN to enable."
                        );
                        pr_failed = true;
                    }
                    Err(e) => {
                        eprintln!("error: GitLab MR creation failed: {e}");
                        pr_failed = true;
                    }
                }
            }
        }
    }

    // Auto-comment PR link on the ticket if configured
    if config.tracker.comment_on_ship {
        if let Some(ref pr_url) = result.pr_url {
            let comment_body = format!("PR opened: {}", pr_url);
            if let Err(e) =
                tracker::post_comment(&config, ticket, &comment_body, Some(manager.repo_root()))
                    .await
            {
                eprintln!("warning: failed to post comment on ticket: {e}");
            }
        }
    }

    if pr_failed {
        eprintln!(
            "note: worktree preserved at {} — fix the issue and retry `parsec ship {}`",
            manager
                .get(ticket)
                .map(|ws| ws.path.display().to_string())
                .unwrap_or_default(),
            ticket
        );
    }

    output::print_ship(&result, mode);

    // Auto-transition ticket status
    if let Some(ref auto) = config.tracker.auto_transition {
        if let Some(ref status) = auto.on_ship {
            tracker::try_transition(&config, ticket, status).await;
        }
    }

    if let Err(e) = crate::oplog::record(
        manager.repo_root(),
        crate::oplog::OpKind::Ship,
        Some(ticket),
        &format!(
            "Shipped branch '{}'{}{}",
            result.branch,
            result
                .pr_url
                .as_ref()
                .map(|u| format!(" -> {}", u))
                .unwrap_or_default(),
            if pr_failed {
                " (partial: PR failed)"
            } else {
                ""
            },
        ),
        Some(crate::oplog::UndoInfo {
            branch: Some(result.branch.clone()),
            base_branch: Some(result.base_branch.clone()),
            path: None,
            ticket_title: result.ticket_title.clone(),
            pr_number: result
                .pr_url
                .as_ref()
                .and_then(|u| u.rsplit('/').next().and_then(|n| n.parse::<u64>().ok())),
            pr_url: result.pr_url.clone(),
        }),
    ) {
        eprintln!("warning: failed to write oplog: {e}");
    }

    if pr_failed {
        bail_code!(
            ErrorCode::E012,
            "Ship partial: branch pushed but PR/MR creation failed. Worktree preserved."
        );
    }

    Ok(())
}

/// Stack context for PR body navigation links.
struct StackPrInfo {
    parent_ticket: Option<String>,
    parent_branch: Option<String>,
    child_tickets: Vec<(String, String)>, // (ticket, branch)
    current_branch: String,
}

/// Gather stack relationship info for a ticket, if it's part of a stack.
fn gather_stack_info(manager: &WorktreeManager, ticket: &str) -> Option<StackPrInfo> {
    let workspaces = manager.list().ok()?;
    let current_ws = manager.get(ticket).ok()?;

    let parent = current_ws
        .parent_ticket
        .as_ref()
        .and_then(|pt| workspaces.iter().find(|w| w.ticket == *pt));

    let children: Vec<_> = workspaces
        .iter()
        .filter(|w| w.parent_ticket.as_deref() == Some(ticket))
        .collect();

    if parent.is_none() && children.is_empty() {
        return None;
    }

    Some(StackPrInfo {
        parent_ticket: current_ws.parent_ticket.clone(),
        parent_branch: parent.map(|p| p.branch.clone()),
        child_tickets: children
            .iter()
            .map(|c| (c.ticket.clone(), c.branch.clone()))
            .collect(),
        current_branch: current_ws.branch.clone(),
    })
}

fn build_pr_body(
    ticket: &str,
    title: Option<&str>,
    ticket_url: Option<&str>,
    stack_info: Option<&StackPrInfo>,
) -> String {
    let mut body = String::new();

    if let Some(title) = title {
        body.push_str(&format!("## {}\n\n", title));
    }

    // Add ticket link if URL is available (works for any tracker)
    if let Some(url) = ticket_url {
        body.push_str(&format!("**Ticket**: [{ticket}]({url})\n\n"));
    }

    // Add stack navigation section (#234)
    if let Some(stack) = stack_info {
        body.push_str("### Stack\n\n");
        body.push_str("| | Ticket | Branch |\n");
        body.push_str("|---|--------|--------|\n");

        if let (Some(ref pt), Some(ref pb)) = (&stack.parent_ticket, &stack.parent_branch) {
            body.push_str(&format!("| \u{2b06} Parent | {} | `{}` |\n", pt, pb));
        }

        body.push_str(&format!(
            "| **\u{27a1} Current** | **{}** | **`{}`** |\n",
            ticket, stack.current_branch
        ));

        for (ct, cb) in &stack.child_tickets {
            body.push_str(&format!("| \u{2b07} Child | {} | `{}` |\n", ct, cb));
        }

        body.push('\n');
    }

    body.push_str(&format!("Shipped via `parsec ship {ticket}`\n"));

    body
}
