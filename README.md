# git-parsec

> Git worktree lifecycle manager for parallel AI agent workflows

**parsec** manages isolated git worktrees tied to tickets (Jira, GitHub Issues), enabling multiple AI agents or developers to work on the same repository in parallel without lock conflicts.

## The Problem

Git uses a single working directory with a single `index.lock`. When multiple AI agents (or developers) try to work on the same repo simultaneously:

- `git add/commit` operations collide on `.git/index.lock`
- Context switching between tasks requires stashing or committing WIP
- Worktrees exist but have poor lifecycle management
- No connection between tickets and working directories

## The Solution

```bash
# Create an isolated workspace for a ticket
$ parsec start PROJ-1234
  Created worktree at .parsec/workspaces/PROJ-1234
  Branch: feature/PROJ-1234
  Ready for parallel work.

# Start another ticket simultaneously (no conflicts!)
$ parsec start PROJ-5678

# See all active workspaces
$ parsec list
  TICKET       BRANCH               STATUS    COMMITS  AGE
  PROJ-1234    feature/PROJ-1234    active    3        2h
  PROJ-5678    feature/PROJ-5678    active    1        5m

# Check if any workspaces touch the same files
$ parsec conflicts
  No file conflicts detected across active workspaces.

# Complete: push, create PR, and clean up in one step
$ parsec ship PROJ-1234
  Pushed feature/PROJ-1234 (3 commits)
  Created PR #42: "PROJ-1234: Add user authentication"
  Cleaned up worktree.

# Remove all merged/stale workspaces
$ parsec clean
  Removed 1 merged workspace.
```

## Features

- **Ticket-driven workspaces** — Create worktrees named after Jira/GitHub Issues tickets
- **Zero-conflict parallelism** — Each workspace has its own index, no lock contention
- **Conflict detection** — Warns when multiple workspaces modify the same files
- **One-step shipping** — `parsec ship` pushes, creates a PR, and cleans up
- **Agent-friendly output** — `--json` flag on every command for machine consumption
- **Status dashboard** — See all parallel work at a glance
- **Auto-cleanup** — Remove worktrees for merged branches automatically

## Installation

```bash
cargo install git-parsec
```

Or build from source:

```bash
git clone https://github.com/erishforG/git-parsec.git
cd git-parsec
cargo build --release
# Binary at ./target/release/parsec
```

## Configuration

```bash
# Interactive setup
$ parsec config init

# Or edit directly: ~/.config/parsec/config.toml
```

```toml
# ~/.config/parsec/config.toml

[workspace]
# Where to create worktrees (relative to repo root)
base_dir = ".parsec/workspaces"
# Branch prefix for new worktrees
branch_prefix = "feature/"

[tracker]
# "jira" | "github" | "none"
provider = "jira"

[tracker.jira]
base_url = "https://yourcompany.atlassian.net"
# Auth via PARSEC_JIRA_TOKEN env var

[tracker.github]
# Uses gh CLI authentication

[ship]
# Auto-create PR on ship
auto_pr = true
# Delete worktree after successful push
auto_cleanup = true
# PR draft mode
draft = false
```

## Commands

| Command | Description |
|---------|-------------|
| `parsec start <ticket>` | Create a new worktree for a ticket |
| `parsec list` | List all active worktrees |
| `parsec status [ticket]` | Detailed status of a workspace |
| `parsec ship <ticket>` | Push + create PR + cleanup |
| `parsec clean` | Remove merged/stale worktrees |
| `parsec conflicts` | Detect file conflicts across worktrees |
| `parsec switch <ticket>` | Print the workspace path (for `cd`) |
| `parsec config init` | Interactive configuration setup |
| `parsec config show` | Show current configuration |

### Global Flags

| Flag | Description |
|------|-------------|
| `--json` | Machine-readable JSON output |
| `--quiet` | Suppress non-essential output |
| `--repo <path>` | Target repository (default: current dir) |

## Comparison with Alternatives

| Feature | parsec | worktrunk | git worktree | git-town |
|---------|--------|-----------|--------------|----------|
| Ticket tracker integration | Jira/GitHub Issues | No | No | No |
| Conflict detection | Cross-worktree | No | No | No |
| One-step ship (push+PR+clean) | Yes | No | No | Yes |
| JSON output for AI agents | Yes | Yes | No | No |
| Auto-cleanup merged | Yes | Yes | Manual | No |
| Status dashboard | Yes | Yes | No | No |
| Zero config start | Yes | Yes | No | No |

## License

MIT
