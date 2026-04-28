use std::path::Path;

use anyhow::Result;

use crate::config::ParsecConfig;
use crate::git;
use crate::output::Mode;
use crate::worktree::WorktreeManager;

pub async fn compress(
    repo: &Path,
    ticket: Option<&str>,
    message: Option<String>,
    mode: Mode,
) -> Result<()> {
    let config = ParsecConfig::load()?;
    let manager = WorktreeManager::new(repo, &config)?;

    // Resolve ticket from arg or current worktree
    let ticket = match ticket {
        Some(t) => t.to_string(),
        None => {
            let cwd = std::env::current_dir()?;
            let workspaces = manager.list()?;
            workspaces
                .iter()
                .find(|w| cwd.starts_with(&w.path))
                .map(|w| w.ticket.clone())
                .ok_or_else(|| anyhow::anyhow!("Not in a parsec worktree. Specify a ticket."))?
        }
    };

    let workspace = manager.get(&ticket)?;

    // Find merge-base with the base branch
    let merge_base = git::run_output(
        &workspace.path,
        &["merge-base", "HEAD", &workspace.base_branch],
    )?;

    // Count commits to squash
    let log_output = git::run_output(
        &workspace.path,
        &["rev-list", "--count", &format!("{}..HEAD", merge_base)],
    )?;
    let commit_count: u64 = log_output.parse().unwrap_or(0);

    if commit_count <= 1 {
        if mode == Mode::Human {
            println!(
                "Nothing to compress — branch has {} commit(s) since base.",
                commit_count
            );
        }
        return Ok(());
    }

    // Get the default commit message (combine all commit messages)
    let combined_msg = if let Some(ref msg) = message {
        msg.clone()
    } else {
        git::run_output(
            &workspace.path,
            &["log", "--format=%s", &format!("{}..HEAD", merge_base)],
        )?
    };

    // Soft reset to merge-base
    git::run(&workspace.path, &["reset", "--soft", &merge_base])?;

    // Recommit with combined or custom message
    let final_message = if message.is_some() {
        combined_msg
    } else {
        // Use first commit message as primary, rest as bullet points
        let lines: Vec<&str> = combined_msg.lines().collect();
        if lines.len() == 1 {
            lines[0].to_string()
        } else {
            format!(
                "{}\n\nSquashed {} commits:\n{}",
                lines[0],
                commit_count,
                lines
                    .iter()
                    .map(|l| format!("- {}", l))
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        }
    };

    git::run(&workspace.path, &["commit", "-m", &final_message])?;

    if mode == Mode::Human {
        println!(
            "Compressed {} commits into 1 for ticket {}.",
            commit_count, ticket
        );
    }

    Ok(())
}
