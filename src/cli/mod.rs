mod commands;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

use crate::output;

#[derive(Parser)]
#[command(
    name = "parsec",
    about = "Git worktree lifecycle manager for parallel AI agent workflows",
    long_about = "Git worktree lifecycle manager for parallel AI agent workflows.\n\nCreate isolated workspaces tied to issue tickets (Jira, GitHub Issues, GitLab),\nwork in parallel without lock conflicts, and ship with one command.\n\nQuick start:\n  parsec start PROJ-123          Create workspace for a ticket\n  parsec list                    See all active workspaces\n  parsec switch PROJ-123         Jump into a workspace\n  parsec ship PROJ-123           Push + PR + cleanup\n  parsec log                     View operation history\n  parsec undo                    Revert last operation",
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
    ///
    /// Creates an isolated git worktree linked to a ticket identifier.
    /// If a tracker (Jira/GitHub Issues) is configured, the ticket title
    /// is fetched automatically. Use --title to set it manually.
    Start {
        /// Ticket identifier (e.g., PROJ-1234, #42)
        ticket: String,

        /// Base branch to create from (default: main/master)
        #[arg(long, short)]
        base: Option<String>,

        /// Manually set the ticket title (skips tracker lookup)
        #[arg(long)]
        title: Option<String>,
    },

    /// List all active worktrees
    ///
    /// Shows a table of all parsec-managed worktrees with ticket, branch,
    /// status, creation time, and path. Use --json for machine-readable output.
    List,

    /// Show detailed status of a workspace
    ///
    /// Displays full details for one or all workspaces including ticket title,
    /// branch, base branch, status, and path.
    Status {
        /// Ticket identifier (optional, shows all if omitted)
        ticket: Option<String>,
    },

    /// Push, create PR/MR, and clean up a workspace
    ///
    /// Pushes the branch to remote, creates a GitHub PR or GitLab MR
    /// with the ticket title, and removes the worktree. The forge type
    /// is auto-detected from the remote URL.
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
    ///
    /// By default, only removes worktrees whose branches have been merged
    /// into the base branch. Use --all to remove everything.
    Clean {
        /// Remove all worktrees (including unmerged)
        #[arg(long)]
        all: bool,

        /// Dry run - show what would be removed
        #[arg(long)]
        dry_run: bool,
    },

    /// Detect file conflicts across active worktrees
    ///
    /// Compares modified files across all active worktrees and reports
    /// any files that are being edited in more than one workspace.
    Conflicts,

    /// Print workspace path for a ticket (use with cd)
    ///
    /// Outputs the absolute path to the worktree for a given ticket.
    /// With shell integration (eval "$(parsec config shell zsh)"),
    /// this command changes your directory automatically.
    Switch {
        /// Ticket identifier
        ticket: String,
    },

    /// Sync worktree with latest base branch changes
    ///
    /// Fetches the latest changes from the remote base branch and rebases
    /// (or merges) the worktree branch on top. Use --all to sync every
    /// active worktree at once. Strategy is configurable in config.toml.
    Sync {
        /// Ticket identifier (syncs current worktree if omitted)
        ticket: Option<String>,

        /// Sync all active worktrees
        #[arg(long)]
        all: bool,

        /// Sync strategy: rebase or merge (default: rebase)
        #[arg(long, default_value = "rebase")]
        strategy: String,
    },

    /// Import an existing branch into parsec management
    ///
    /// Brings an existing branch under parsec lifecycle management.
    /// Useful when you started work before using parsec or when
    /// taking over someone else's branch.
    Adopt {
        /// Ticket identifier to associate with the branch
        ticket: String,

        /// Branch name to adopt (default: current branch)
        #[arg(long, short)]
        branch: Option<String>,

        /// Ticket title (optional)
        #[arg(long)]
        title: Option<String>,
    },

    /// Show operation history
    ///
    /// Displays a table of all recorded parsec operations (start, adopt,
    /// ship, clean, undo) with timestamps. Filter by ticket or limit
    /// the number of entries shown.
    Log {
        /// Filter by ticket identifier
        ticket: Option<String>,

        /// Show last N entries (default: 20)
        #[arg(long, short = 'n', default_value = "20")]
        last: usize,
    },

    /// Undo the last parsec operation
    ///
    /// Reverses the most recent parsec operation:
    ///   start/adopt → removes worktree and deletes branch
    ///   ship/clean  → restores worktree from branch
    /// Use --dry-run to preview before executing.
    Undo {
        /// Preview what would be undone without making changes
        #[arg(long)]
        dry_run: bool,
    },

    /// Configure parsec
    ///
    /// Manage parsec configuration: run interactive setup, view current
    /// settings, or output shell integration scripts.
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
}

#[derive(Subcommand)]
pub enum ConfigAction {
    /// Interactive configuration setup
    ///
    /// Walks through tracker provider, branch prefix, ship behavior,
    /// and other settings interactively.
    Init,
    /// Show current configuration
    ///
    /// Prints the active configuration from ~/.config/parsec/config.toml.
    Show,
    /// Output shell integration script
    ///
    /// Prints a shell function that wraps parsec switch to auto-cd.
    /// Add eval "$(parsec config shell zsh)" to your ~/.zshrc.
    Shell {
        /// Shell type (zsh or bash)
        #[arg(default_value = "zsh")]
        shell: String,
    },
    /// Install man page
    ///
    /// Generates and installs the parsec(1) man page so that
    /// `man parsec` works. Requires write access to the man directory.
    Man {
        /// Man page base directory (default: /usr/local/share/man)
        #[arg(long, default_value = "/usr/local/share/man")]
        dir: PathBuf,
    },
    /// Output shell completions
    ///
    /// Generates tab-completion scripts for your shell.
    /// Add eval "$(parsec config completions zsh)" to your ~/.zshrc.
    Completions {
        /// Shell type (zsh, bash, fish, elvish, powershell)
        shell: clap_complete::Shell,
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
        Command::Start {
            ticket,
            base,
            title,
        } => commands::start(&repo_path, &ticket, base.as_deref(), title, output_mode).await,
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
        Command::Sync {
            ticket,
            all,
            strategy,
        } => commands::sync(&repo_path, ticket.as_deref(), all, &strategy, output_mode).await,
        Command::Adopt {
            ticket,
            branch,
            title,
        } => commands::adopt(&repo_path, &ticket, branch.as_deref(), title, output_mode).await,
        Command::Conflicts => commands::conflicts(&repo_path, output_mode).await,
        Command::Switch { ticket } => commands::switch(&repo_path, &ticket, output_mode).await,
        Command::Log { ticket, last } => {
            commands::log(&repo_path, ticket.as_deref(), last, output_mode).await
        }
        Command::Undo { dry_run } => commands::undo(&repo_path, dry_run, output_mode).await,
        Command::Config { action } => match action {
            ConfigAction::Init => commands::config_init(output_mode).await,
            ConfigAction::Show => commands::config_show(output_mode).await,
            ConfigAction::Shell { shell } => commands::config_shell(&shell, output_mode).await,
            ConfigAction::Man { dir } => commands::config_man(&dir).await,
            ConfigAction::Completions { shell } => commands::config_completions(shell).await,
        },
    }
}
