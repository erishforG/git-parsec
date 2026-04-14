mod commands;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

use crate::output;

#[derive(Parser)]
#[command(
    name = "parsec",
    about = "Git worktree lifecycle manager for parallel AI agent workflows",
    version,
    arg_required_else_help = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,

    /// Output in JSON format (machine-readable)
    #[arg(long, global = true)]
    pub json: bool,

    /// Suppress non-essential output
    #[arg(long, short, global = true)]
    pub quiet: bool,

    /// Target repository path (default: current directory)
    #[arg(long, global = true)]
    pub repo: Option<PathBuf>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Create a new worktree for a ticket
    Start {
        /// Ticket identifier (e.g., PROJ-1234, #42)
        ticket: String,

        /// Base branch to create from (default: main/master)
        #[arg(long, short)]
        base: Option<String>,
    },

    /// List all active worktrees
    List,

    /// Show detailed status of a workspace
    Status {
        /// Ticket identifier (optional, shows all if omitted)
        ticket: Option<String>,
    },

    /// Push, create PR, and clean up a workspace
    Ship {
        /// Ticket identifier
        ticket: String,

        /// Create PR as draft
        #[arg(long)]
        draft: bool,

        /// Skip PR creation, only push
        #[arg(long)]
        no_pr: bool,
    },

    /// Remove merged or stale worktrees
    Clean {
        /// Remove all worktrees (including unmerged)
        #[arg(long)]
        all: bool,

        /// Dry run - show what would be removed
        #[arg(long)]
        dry_run: bool,
    },

    /// Detect file conflicts across active worktrees
    Conflicts,

    /// Print workspace path for a ticket (use with cd)
    Switch {
        /// Ticket identifier
        ticket: String,
    },

    /// Configure parsec
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
}

#[derive(Subcommand)]
pub enum ConfigAction {
    /// Interactive configuration setup
    Init,
    /// Show current configuration
    Show,
    /// Output shell integration script (add to .zshrc/.bashrc)
    Shell {
        /// Shell type (zsh or bash)
        #[arg(default_value = "zsh")]
        shell: String,
    },
}

pub async fn run(cli: Cli) -> Result<()> {
    let repo_path = cli.repo.unwrap_or_else(|| PathBuf::from("."));
    let output_mode = if cli.json {
        output::Mode::Json
    } else if cli.quiet {
        output::Mode::Quiet
    } else {
        output::Mode::Human
    };

    match cli.command {
        Command::Start { ticket, base } => {
            commands::start(&repo_path, &ticket, base.as_deref(), output_mode).await
        }
        Command::List => commands::list(&repo_path, output_mode).await,
        Command::Status { ticket } => {
            commands::status(&repo_path, ticket.as_deref(), output_mode).await
        }
        Command::Ship {
            ticket,
            draft,
            no_pr,
        } => commands::ship(&repo_path, &ticket, draft, no_pr, output_mode).await,
        Command::Clean { all, dry_run } => {
            commands::clean(&repo_path, all, dry_run, output_mode).await
        }
        Command::Conflicts => commands::conflicts(&repo_path, output_mode).await,
        Command::Switch { ticket } => commands::switch(&repo_path, &ticket, output_mode).await,
        Command::Config { action } => match action {
            ConfigAction::Init => commands::config_init(output_mode).await,
            ConfigAction::Show => commands::config_show(output_mode).await,
            ConfigAction::Shell { shell } => commands::config_shell(&shell, output_mode).await,
        },
    }
}
