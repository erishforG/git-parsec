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

        /// Create stacked on another ticket's branch
        #[arg(long)]
        on: Option<String>,

        /// Use an existing branch instead of creating a new one
        #[arg(long = "branch")]
        existing_branch: Option<String>,

        /// Run a command after worktree creation (one-off hook)
        #[arg(long)]
        hook: Option<String>,
    },

    /// List all active worktrees
    ///
    /// Shows a table of all parsec-managed worktrees with ticket, branch,
    /// status, creation time, and path. Use --json for machine-readable output.
    /// PR status is fetched automatically; use --no-pr to skip API calls.
    List {
        /// Skip PR status lookup (faster, works offline)
        #[arg(long)]
        no_pr: bool,
        /// Show extended metadata (commits, divergence, last commit)
        #[arg(long)]
        full: bool,
    },

    /// Show detailed status of a workspace
    ///
    /// Displays full details for one or all workspaces including ticket title,
    /// branch, base branch, status, and path.
    Status {
        /// Ticket identifier (optional, shows all if omitted)
        ticket: Option<String>,
    },

    /// View ticket details from tracker
    ///
    /// Fetches and displays ticket information (title, status, assignee)
    /// from the configured tracker. Auto-detects the ticket from the
    /// current worktree if no ticket is specified.
    Ticket {
        /// Ticket identifier (auto-detects from current worktree if omitted)
        ticket: Option<String>,

        /// Post a comment on the ticket
        #[arg(long)]
        comment: Option<String>,
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

        /// Target base branch for PR (default from config or worktree base)
        #[arg(long)]
        base: Option<String>,

        /// Override PR title (default: [TICKET] tracker title)
        #[arg(long, short)]
        title: Option<String>,

        /// Skip pre-ship hooks
        #[arg(long)]
        skip_hooks: bool,
    },

    /// Remove merged or stale worktrees
    ///
    /// By default, only removes worktrees whose branches have been merged
    /// into the base branch. Use --all to remove everything.
    /// Provide a ticket identifier to remove a specific workspace regardless
    /// of merge status.
    Clean {
        /// Specific ticket to clean (removes regardless of merge status)
        ticket: Option<String>,

        /// Remove all worktrees (including unmerged)
        #[arg(long)]
        all: bool,

        /// Dry run - show what would be removed
        #[arg(long)]
        dry_run: bool,

        /// Remove orphan entries (state entries without existing directory)
        #[arg(long)]
        orphans: bool,
    },

    /// Detect file conflicts across active worktrees
    ///
    /// Compares modified files across all active worktrees and reports
    /// any files that are being edited in more than one workspace.
    Conflicts,

    /// Check PR/MR CI and review status
    ///
    /// Shows CI check results, review approvals, and merge status for
    /// shipped PRs. Requires a GitHub/GitLab token.
    PrStatus {
        /// Ticket identifier (shows all shipped if omitted)
        ticket: Option<String>,
    },

    /// Merge a ticket's PR on GitHub and clean up
    ///
    /// Merges the PR via the GitHub API. By default uses squash merge
    /// and waits for CI to pass before merging. Cleans up the local
    /// worktree after a successful merge.
    /// Pass multiple tickets to merge them sequentially (batch mode).
    Merge {
        /// Ticket identifiers (auto-detects current worktree if omitted)
        tickets: Vec<String>,
        /// Use rebase merge instead of squash
        #[arg(long)]
        rebase: bool,
        /// Skip waiting for CI to pass
        #[arg(long)]
        no_wait: bool,
        /// Keep remote branch after merge (default: delete)
        #[arg(long)]
        no_delete_branch: bool,
    },

    /// Check CI/CD pipeline status for a ticket's PR
    ///
    /// Shows individual check runs with status, duration, and overall
    /// summary. Auto-detects the current worktree if no ticket is given.
    /// Use --watch to poll until all checks complete.
    Ci {
        /// Ticket identifiers (auto-detects current worktree if omitted)
        tickets: Vec<String>,
        /// Watch CI in real-time until completion (refresh every 5s)
        #[arg(long)]
        watch: bool,
        /// Show CI for all shipped PRs
        #[arg(long)]
        all: bool,
    },

    /// View changes in a worktree compared to base branch
    ///
    /// Shows the diff between the worktree branch and its base branch
    /// using the merge-base as the comparison point.
    Diff {
        /// Ticket identifier (auto-detects current worktree if omitted)
        ticket: Option<String>,
        /// Show file-level summary only
        #[arg(long)]
        stat: bool,
        /// List changed file names only
        #[arg(long)]
        name_only: bool,
    },

    /// Print workspace path for a ticket (use with cd)
    ///
    /// Outputs the absolute path to the worktree for a given ticket.
    /// When called without a ticket, shows an interactive picker.
    /// With shell integration (eval "$(parsec init zsh)"),
    /// this command changes your directory automatically.
    Switch {
        /// Ticket identifier (interactive picker if omitted)
        ticket: Option<String>,
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

    /// Open PR/MR or ticket page in browser
    ///
    /// Opens the associated PR/MR URL (if shipped) or the ticket tracker
    /// page in your default browser. Use --pr to force opening the PR,
    /// or --ticket to force opening the tracker page.
    Open {
        /// Ticket identifier
        ticket: String,

        /// Force open the PR/MR page
        #[arg(long)]
        pr: bool,

        /// Force open the ticket tracker page
        #[arg(long)]
        ticket_page: bool,
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

    /// List assigned tickets without active worktrees
    ///
    /// Fetches tickets assigned to you from Jira that don't yet have a
    /// parsec worktree. Shows a table of ticket key, title, priority,
    /// and status. Use --pick to interactively select one and auto-start
    /// a workspace.
    Inbox {
        /// Interactively pick a ticket and run `parsec start`
        #[arg(long)]
        pick: bool,
    },

    /// Show the sprint board as a Kanban view
    ///
    /// Fetches the active sprint from Jira and displays tickets grouped
    /// by status column. Active worktrees are marked with [wt] and
    /// shipped PRs with [pr]. Currently supports Jira only.
    Board {
        /// Jira board ID (auto-detected from project if omitted)
        #[arg(long)]
        board_id: Option<u64>,

        /// Jira project key (inferred from active worktrees if omitted)
        #[arg(long, short)]
        project: Option<String>,

        /// Filter by assignee (default from config/env)
        #[arg(long)]
        assignee: Option<String>,

        /// Show all tickets (ignore assignee filter)
        #[arg(long)]
        all: bool,
    },

    /// Show or manage stacked PR dependencies
    ///
    /// Displays the dependency graph of worktrees created with --on.
    /// Use `parsec stack --sync` to rebase the entire chain.
    Stack {
        /// Sync the entire stack (rebase chain)
        #[arg(long)]
        sync: bool,
    },

    /// Print the main repository root path
    ///
    /// Outputs the absolute path to the main (non-worktree) repository root.
    /// Useful for scripting and shell integration after worktree cleanup.
    Root,

    /// Output shell integration script
    ///
    /// Prints a shell function that wraps parsec for auto-cd on switch
    /// and auto-recovery after merge cleanup. Supports zsh and bash.
    /// Add eval "$(parsec init zsh)" to your ~/.zshrc.
    /// Use --install to automatically append the integration to your shell config.
    Init {
        /// Shell type (zsh or bash)
        #[arg(default_value = "zsh")]
        shell: String,
        /// Automatically install shell integration into shell config file
        #[arg(long)]
        install: bool,
        /// Skip confirmation prompt (for scripting)
        #[arg(long, short)]
        yes: bool,
    },

    /// Configure parsec
    ///
    /// Manage parsec configuration: run interactive setup, view current
    /// settings, or output shell integration scripts.
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },

    /// Validate environment and configuration
    ///
    /// Checks git version, token configuration, tracker connectivity,
    /// shell integration, and remote access. Prints ✓/✗ for each check
    /// with actionable fix instructions.
    Doctor,

    /// Create a release: merge to release branch, tag, and create GitHub Release
    ///
    /// Merges the current develop branch into the release branch (default: main),
    /// creates a git tag, and creates a GitHub Release with auto-generated changelog.
    Release {
        /// Version string (e.g., "0.3.0")
        version: String,
        /// Source branch to release from (default: develop or default branch)
        #[arg(long)]
        from: Option<String>,
        /// Skip creating GitHub Release
        #[arg(long)]
        no_github_release: bool,
        /// Dry run — show what would happen without making changes
        #[arg(long)]
        dry_run: bool,
    },

    /// Create a new issue on the tracker
    ///
    /// Creates a new ticket on GitHub Issues, Jira, or GitLab from the
    /// command line. Auto-detects the tracker from config. Use --start
    /// to immediately create a worktree for the new issue.
    #[command(visible_alias = "new-issue")]
    Create {
        /// Issue title
        #[arg(long)]
        title: String,

        /// Issue body/description
        #[arg(long)]
        body: Option<String>,

        /// Labels to add (can be specified multiple times)
        #[arg(long)]
        label: Vec<String>,

        /// Jira project key (e.g. PROJ). Auto-detected from config if omitted
        #[arg(long, short)]
        project: Option<String>,

        /// Jira issue type (default: Task)
        #[arg(long, default_value = "Task")]
        issue_type: String,

        /// Start a worktree after creation
        #[arg(long)]
        start: bool,
    },

    /// Rename a workspace to a different ticket ID
    ///
    /// Changes the ticket ID, renames the branch, and moves the worktree directory.
    Rename {
        /// Current ticket identifier
        old_ticket: String,
        /// New ticket identifier
        new_ticket: String,
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
    /// Output shell integration script (deprecated: use `parsec init` instead)
    ///
    /// Prints a shell function that wraps parsec switch to auto-cd.
    /// Prefer `parsec init zsh` which also handles merge CWD recovery.
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
            on,
            existing_branch,
            hook,
        } => {
            commands::start(
                &repo_path,
                &ticket,
                base.as_deref(),
                title,
                on.as_deref(),
                existing_branch.as_deref(),
                hook,
                output_mode,
            )
            .await
        }
        Command::List { no_pr, full } => commands::list(&repo_path, no_pr, full, output_mode).await,
        Command::Status { ticket } => {
            commands::status(&repo_path, ticket.as_deref(), output_mode).await
        }
        Command::Ticket { ticket, comment } => {
            commands::ticket(&repo_path, ticket.as_deref(), comment, output_mode).await
        }
        Command::Ship {
            ticket,
            draft,
            no_pr,
            base,
            title,
            skip_hooks,
        } => {
            commands::ship(
                &repo_path,
                &ticket,
                draft,
                no_pr,
                base,
                title,
                skip_hooks,
                output_mode,
            )
            .await
        }
        Command::Clean {
            ticket,
            all,
            dry_run,
            orphans,
        } => {
            commands::clean(
                &repo_path,
                ticket.as_deref(),
                all,
                dry_run,
                orphans,
                output_mode,
            )
            .await
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
        Command::Open {
            ticket,
            pr,
            ticket_page,
        } => commands::open(&repo_path, &ticket, pr, ticket_page, output_mode).await,
        Command::PrStatus { ticket } => {
            commands::pr_status(&repo_path, ticket.as_deref(), output_mode).await
        }
        Command::Merge {
            tickets,
            rebase,
            no_wait,
            no_delete_branch,
        } => {
            if tickets.len() > 1 {
                commands::merge_batch(
                    &repo_path,
                    &tickets.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
                    rebase,
                    no_wait,
                    no_delete_branch,
                    output_mode,
                )
                .await
            } else {
                commands::merge(
                    &repo_path,
                    tickets.first().map(|s| s.as_str()),
                    rebase,
                    no_wait,
                    no_delete_branch,
                    output_mode,
                )
                .await
            }
        }
        Command::Ci {
            tickets,
            watch,
            all,
        } => {
            let refs: Vec<&str> = tickets.iter().map(|s| s.as_str()).collect();
            commands::ci(&repo_path, &refs, watch, all, output_mode).await
        }
        Command::Diff {
            ticket,
            stat,
            name_only,
        } => commands::diff(&repo_path, ticket.as_deref(), stat, name_only, output_mode).await,
        Command::Conflicts => commands::conflicts(&repo_path, output_mode).await,
        Command::Switch { ticket } => {
            commands::switch(&repo_path, ticket.as_deref(), output_mode).await
        }
        Command::Log { ticket, last } => {
            commands::log(&repo_path, ticket.as_deref(), last, output_mode).await
        }
        Command::Undo { dry_run } => commands::undo(&repo_path, dry_run, output_mode).await,
        Command::Inbox { pick } => commands::inbox(&repo_path, pick, output_mode).await,
        Command::Board {
            board_id,
            project,
            assignee,
            all,
        } => commands::board(&repo_path, board_id, project, assignee, all, output_mode).await,
        Command::Stack { sync } => {
            if sync {
                commands::stack_sync(&repo_path, output_mode).await
            } else {
                commands::stack(&repo_path, output_mode).await
            }
        }
        Command::Root => commands::root(&repo_path).await,
        Command::Init {
            shell,
            install,
            yes,
        } => {
            if install {
                commands::init_install(&shell, yes).await
            } else {
                commands::init_shell(&shell).await
            }
        }
        Command::Config { action } => match action {
            ConfigAction::Init => commands::config_init(output_mode).await,
            ConfigAction::Show => commands::config_show(output_mode).await,
            ConfigAction::Shell { shell } => commands::config_shell(&shell, output_mode).await,
            ConfigAction::Man { dir } => commands::config_man(&dir).await,
            ConfigAction::Completions { shell } => commands::config_completions(shell).await,
        },
        Command::Doctor => commands::doctor(&repo_path, output_mode).await,
        Command::Release {
            version,
            from,
            no_github_release,
            dry_run,
        } => {
            commands::release(
                &repo_path,
                &version,
                from.as_deref(),
                no_github_release,
                dry_run,
                output_mode,
            )
            .await
        }
        Command::Create {
            title,
            body,
            label,
            project,
            issue_type,
            start,
        } => {
            commands::create(
                &repo_path,
                &title,
                body.as_deref(),
                &label,
                project.as_deref(),
                &issue_type,
                start,
                output_mode,
            )
            .await
        }
        Command::Rename {
            old_ticket,
            new_ticket,
        } => commands::rename(&repo_path, &old_ticket, &new_ticket, output_mode).await,
    }
}
