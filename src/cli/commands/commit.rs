use std::path::Path;

use anyhow::{bail, Context, Result};

use crate::config::ParsecConfig;
use crate::git;
use crate::output::Mode;
use crate::worktree::WorktreeManager;

pub async fn commit(
    repo: &Path,
    ticket_override: Option<&str>,
    conventional: bool,
    message_override: Option<&str>,
    mode: Mode,
) -> Result<()> {
    let config = ParsecConfig::load()?;

    // If user provides a message directly, just commit with it (no AI)
    if let Some(msg) = message_override {
        git::run(repo, &["commit", "-m", msg])?;
        if mode == Mode::Human {
            eprintln!("Committed with provided message.");
        }
        if mode == Mode::Json {
            let value = serde_json::json!({
                "action": "commit",
                "message": msg,
                "ai_generated": false,
            });
            println!("{}", value);
        }
        return Ok(());
    }

    // Resolve AI API key
    let api_key = crate::env::ai_api_key(config.ai.api_key.as_deref()).ok_or_else(|| {
        anyhow::anyhow!(
            "No AI API key found. Set PARSEC_AI_API_KEY, OPENAI_API_KEY, \
             or ANTHROPIC_API_KEY, or add [ai] api_key in config.toml"
        )
    })?;

    // Get staged diff
    let diff = git::run_output(repo, &["diff", "--cached", "--stat"])
        .context("Failed to get staged changes")?;
    if diff.trim().is_empty() {
        bail!("No staged changes. Use `git add` to stage files first.");
    }

    let full_diff = git::run_output(repo, &["diff", "--cached"])?;

    // Auto-detect ticket from current worktree
    let ticket = if let Some(t) = ticket_override {
        Some(t.to_string())
    } else {
        detect_ticket(repo, &config)
    };

    if mode == Mode::Human {
        eprintln!("Generating commit message...");
    }

    // Call AI
    let message = crate::ai::generate_commit_message(
        &config.ai.provider,
        &config.ai.model,
        &api_key,
        &full_diff,
        ticket.as_deref(),
        conventional,
    )
    .await
    .context("AI commit message generation failed")?;

    if mode == Mode::Human {
        eprintln!("\n{}", message);
        eprintln!();
    }

    // In non-interactive modes (JSON/Quiet), commit directly. Otherwise confirm.
    let should_commit = if mode != Mode::Human {
        true
    } else {
        dialoguer::Confirm::new()
            .with_prompt("Commit with this message?")
            .default(true)
            .interact()
            .context("Failed to read confirmation")?
    };

    if should_commit {
        git::run(repo, &["commit", "-m", &message])?;
        if mode == Mode::Human {
            eprintln!("Committed!");
        }
    } else {
        if mode == Mode::Human {
            eprintln!("Aborted.");
        }
        return Ok(());
    }

    if mode == Mode::Json {
        let value = serde_json::json!({
            "action": "commit",
            "message": message,
            "ticket": ticket,
            "ai_generated": true,
            "provider": format!("{:?}", config.ai.provider),
            "model": config.ai.model,
        });
        println!("{}", value);
    }

    Ok(())
}

/// Try to detect the ticket ID from the current worktree.
fn detect_ticket(repo: &Path, config: &ParsecConfig) -> Option<String> {
    let manager = WorktreeManager::new(repo, config).ok()?;
    // Check if we're inside a managed worktree
    let repo_root = git::get_repo_root(repo).ok()?;
    let workspaces = manager.list().ok()?;
    for ws in &workspaces {
        if ws.path == repo_root {
            return Some(ws.ticket.clone());
        }
    }
    None
}
