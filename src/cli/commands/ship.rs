use std::path::Path;

use anyhow::{Context, Result};

use crate::config::ParsecConfig;
use crate::git;
use crate::github;
use crate::gitlab;
use crate::output::{self, Mode};
use crate::tracker;
use crate::worktree::WorktreeManager;

pub async fn ship(
    repo: &Path,
    ticket: &str,
    draft: bool,
    no_pr: bool,
    base_override: Option<String>,
    skip_hooks: bool,
    mode: Mode,
) -> Result<()> {
    let mut config = ParsecConfig::load()?;
    let manager = WorktreeManager::new(repo, &config)?;
    config.resolve_for_repo(manager.repo_root());

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
                anyhow::bail!(
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
                    },
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

    // Phase 2: Create PR/MR (async)
    let mut pr_failed = false;
    if !no_pr && config.ship.auto_pr {
        let (ticket_title, ticket_url) =
            match tracker::fetch_ticket(&config, ticket, Some(manager.repo_root())).await {
                Ok(Some(t)) => (Some(t.title), t.url),
                _ => (None, None),
            };

        // Prefer freshly fetched title over stored one
        let effective_title = ticket_title.as_deref().or(result.ticket_title.as_deref());

        let pr_title = effective_title
            .map(|t| format!("{}: {}", result.ticket, t))
            .unwrap_or_else(|| result.ticket.clone());

        let pr_body = build_pr_body(&result.ticket, effective_title, ticket_url.as_deref());

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
        }),
    ) {
        eprintln!("warning: failed to write oplog: {e}");
    }

    if pr_failed {
        anyhow::bail!("Ship partial: branch pushed but PR/MR creation failed. Worktree preserved.");
    }

    Ok(())
}

fn build_pr_body(ticket: &str, title: Option<&str>, ticket_url: Option<&str>) -> String {
    let mut body = String::new();

    if let Some(title) = title {
        body.push_str(&format!("## {}\n\n", title));
    }

    // Add ticket link if URL is available (works for any tracker)
    if let Some(url) = ticket_url {
        body.push_str(&format!("**Ticket**: [{ticket}]({url})\n\n"));
    }

    body.push_str(&format!("Shipped via `parsec ship {ticket}`\n"));

    body
}
