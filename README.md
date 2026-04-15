# git-parsec

> Git worktree lifecycle manager for parallel AI agent workflows

**parsec** manages isolated git worktrees tied to tickets (Jira, GitHub Issues), enabling multiple AI agents or developers to work on the same repository in parallel without lock conflicts.

![demo](demo.gif)

## The Problem

Git uses a single working directory with a single `index.lock`. When multiple AI agents (or developers) try to work on the same repo simultaneously:

- `git add/commit` operations collide on `.git/index.lock`
- Context switching between tasks requires stashing or committing WIP
- Worktrees exist but have poor lifecycle management
- No connection between tickets and working directories

## The Solution

```bash
# Create isolated workspaces for two tickets
$ parsec start PROJ-1234 --title "Add user authentication"
Created workspace for PROJ-1234 at /home/user/myapp.PROJ-1234
  Add user authentication

$ parsec start PROJ-5678 --title "Fix payment timeout"
Created workspace for PROJ-5678 at /home/user/myapp.PROJ-5678
  Fix payment timeout

# See all active workspaces
$ parsec list
╭───────────┬──────────────────────┬────────┬──────────────────┬──────────────────────────────╮
│ Ticket    │ Branch               │ Status │ Created          │ Path                         │
├───────────┼──────────────────────┼────────┼──────────────────┼──────────────────────────────┤
│ PROJ-1234 │ feature/PROJ-1234    │ active │ 2026-04-15 09:00 │ /home/user/myapp.PROJ-1234   │
│ PROJ-5678 │ feature/PROJ-5678    │ active │ 2026-04-15 09:01 │ /home/user/myapp.PROJ-5678   │
╰───────────┴──────────────────────┴────────┴──────────────────┴──────────────────────────────╯

# Check if any workspaces touch the same files
$ parsec conflicts
No conflicts detected.

# Complete: push, create PR, and clean up in one step
$ parsec ship PROJ-1234
Shipped PROJ-1234!
  PR: https://github.com/org/repo/pull/42
  Workspace cleaned up.

# Remove all remaining workspaces
$ parsec clean --all
Removed 1 worktree(s):
  - PROJ-5678
```

## Features

- **Ticket-driven workspaces** -- Create worktrees named after Jira/GitHub Issues tickets
- **Zero-conflict parallelism** -- Each workspace has its own index, no lock contention
- **Conflict detection** -- Warns when multiple workspaces modify the same files
- **One-step shipping** -- `parsec ship` pushes, creates a GitHub PR or GitLab MR, and cleans up
- **Adopt existing branches** -- Import branches already in progress with `parsec adopt`
- **Operation history and undo** -- `parsec log` shows what happened, `parsec undo` reverts it
- **Keep branches fresh** -- `parsec sync` rebases or merges the latest base branch into any worktree
- **Agent-friendly output** -- `--json` flag on every command for machine consumption
- **Status dashboard** -- See all parallel work at a glance
- **Auto-cleanup** -- Remove worktrees for merged branches automatically
- **GitHub and GitLab** -- PR and MR creation for both platforms

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

## Quick Start

```bash
# 1. (Optional) Run interactive setup
$ parsec config init

# 2. Start work on a ticket
$ parsec start PROJ-1234 --title "Add rate limiting"
Created workspace for PROJ-1234 at /home/user/myapp.PROJ-1234
  Add rate limiting

  Tip: cd $(parsec switch PROJ-1234)

# 3. Switch into the workspace
$ cd $(parsec switch PROJ-1234)

# 4. Work, commit as normal...
$ git add . && git commit -m "Implement rate limiter"

# 5. Start a second ticket in parallel
$ parsec start PROJ-5678 --title "Fix auth bug"

# 6. Check for file conflicts across workspaces
$ parsec conflicts

# 7. Ship when done
$ parsec ship PROJ-1234
Shipped PROJ-1234!
  PR: https://github.com/org/repo/pull/42
  Workspace cleaned up.

# 8. See what happened
$ parsec log
```

---

## Commands

### `parsec start <ticket>`

Create an isolated worktree for a ticket. Fetches the ticket title from your configured tracker (Jira, GitHub Issues) or accepts a manual title.

```
parsec start <ticket> [--base <branch>] [--title "text"]
```

| Option | Description |
|--------|-------------|
| `-b, --base <branch>` | Base branch to create from (default: main/master) |
| `--title "text"` | Set ticket title manually, skip tracker lookup |

```bash
# With Jira integration (title auto-fetched)
$ parsec start CL-2283
Created workspace for CL-2283 at /home/user/myapp.CL-2283
  Implement rate limiting for API endpoints

  Tip: cd $(parsec switch CL-2283)

# With manual title
$ parsec start 42 --title "Fix login redirect"
Created workspace for 42 at /home/user/myapp.42
  Fix login redirect

  Tip: cd $(parsec switch 42)

# From a specific base branch
$ parsec start PROJ-99 --base release/2.0
```

---

### `parsec adopt <ticket>`

Import an existing branch into parsec management. Useful when you started work before using parsec, or when taking over someone else's branch.

```
parsec adopt <ticket> [--branch <name>] [--title "text"]
```

| Option | Description |
|--------|-------------|
| `-b, --branch <name>` | Branch to adopt (default: `<prefix><ticket>`) |
| `--title "text"` | Set ticket title manually |

```bash
# Adopt a branch matching the default prefix
$ parsec adopt PROJ-1234
Adopted branch 'feature/PROJ-1234' as PROJ-1234 at /home/user/myapp.PROJ-1234

# Adopt a branch with a different name
$ parsec adopt PROJ-99 --branch fix/payment-timeout
Adopted branch 'fix/payment-timeout' as PROJ-99 at /home/user/myapp.PROJ-99
```

---

### `parsec list`

List all active parsec-managed worktrees.

```
parsec list
```

```bash
$ parsec list
╭────────┬────────────────┬────────┬──────────────────┬────────────────────────────╮
│ Ticket │ Branch         │ Status │ Created          │ Path                       │
├────────┼────────────────┼────────┼──────────────────┼────────────────────────────┤
│ TEST-1 │ feature/TEST-1 │ active │ 2026-04-15 09:00 │ /home/user/myapp.TEST-1    │
│ TEST-2 │ feature/TEST-2 │ active │ 2026-04-15 09:05 │ /home/user/myapp.TEST-2    │
╰────────┴────────────────┴────────┴──────────────────┴────────────────────────────╯

$ parsec list --json
[{"ticket":"TEST-1","path":"/home/user/myapp.TEST-1","branch":"feature/TEST-1","base_branch":"main","created_at":"2026-04-15T09:00:00Z","ticket_title":"Add auth","status":"active"}]
```

---

### `parsec status [ticket]`

Show detailed status of a workspace. Shows all workspaces if no ticket is specified.

```
parsec status [ticket]
```

```bash
$ parsec status PROJ-1234
──────────────────────────────────────────────────
  Ticket: PROJ-1234
  Title: Add user authentication
  Branch: feature/PROJ-1234
  Base: main
  Status: active
  Created: 2026-04-15 09:00 UTC
  Path: /home/user/myapp.PROJ-1234
──────────────────────────────────────────────────
```

---

### `parsec ship <ticket>`

Push the branch, create a PR (GitHub) or MR (GitLab), and clean up the worktree. The forge is auto-detected from the remote URL.

```
parsec ship <ticket> [--draft] [--no-pr]
```

| Option | Description |
|--------|-------------|
| `--draft` | Create the PR/MR as a draft |
| `--no-pr` | Push only, skip PR/MR creation |

```bash
# Push + PR + cleanup
$ parsec ship PROJ-1234
Shipped PROJ-1234!
  PR: https://github.com/org/repo/pull/42
  Workspace cleaned up.

# Draft PR
$ parsec ship PROJ-5678 --draft

# Push only, no PR
$ parsec ship PROJ-9000 --no-pr
```

Token required: set `PARSEC_GITHUB_TOKEN` (or `GITHUB_TOKEN`, `GH_TOKEN`) for GitHub, or `PARSEC_GITLAB_TOKEN` (or `GITLAB_TOKEN`) for GitLab.

---

### `parsec clean`

Remove worktrees whose branches have been merged. Use `--all` to remove everything.

```
parsec clean [--all] [--dry-run]
```

| Option | Description |
|--------|-------------|
| `--all` | Remove all worktrees, including unmerged |
| `--dry-run` | Preview what would be removed |

```bash
# Preview first
$ parsec clean --dry-run
Would remove 1 worktree(s):
  - PROJ-1234

# Remove merged worktrees
$ parsec clean
Removed 1 worktree(s):
  - PROJ-1234

# Remove everything
$ parsec clean --all
Removed 3 worktree(s):
  - PROJ-1234
  - PROJ-5678
  - PROJ-9000
```

---

### `parsec conflicts`

Detect files modified in more than one active worktree. Workspaces with no changes are skipped.

```
parsec conflicts
```

```bash
# No conflicts
$ parsec conflicts
No conflicts detected.

# Conflicts found
$ parsec conflicts
╭──────────────────┬──────────────────────╮
│ File             │ Worktrees            │
├──────────────────┼──────────────────────┤
│ src/api/router.rs│ PROJ-1234, PROJ-5678 │
╰──────────────────┴──────────────────────╯
```

---

### `parsec switch [ticket]`

Print the absolute path to a ticket's worktree. When called without a ticket, shows an interactive picker. Designed for `cd $(parsec switch ...)`.

```
parsec switch [ticket]
```

```bash
# Direct switch
$ parsec switch PROJ-1234
/home/user/myapp.PROJ-1234

# Interactive picker (no argument)
$ parsec switch
? Switch to workspace ›
❯ PROJ-1234 — Add user authentication
  PROJ-5678 — Fix payment timeout

# Use with cd
$ cd $(parsec switch PROJ-1234)
```

---

### `parsec log [ticket]`

Show the history of parsec operations. Each mutating command (start, adopt, ship, clean, undo) is recorded with a timestamp.

```
parsec log [ticket] [-n, --last N]
```

| Option | Description |
|--------|-------------|
| `[ticket]` | Filter to a specific ticket |
| `-n, --last N` | Show last N entries (default: 20) |

```bash
$ parsec log
╭───┬───────┬───────────┬───────────────────────────────────────────────┬──────────────────╮
│ # │ Op    │ Ticket    │ Detail                                        │ Time             │
├───┼───────┼───────────┼───────────────────────────────────────────────┼──────────────────┤
│ 4 │ clean │ PROJ-5678 │ Cleaned workspace for branch 'feature/5678'   │ 2026-04-15 14:30 │
│ 3 │ ship  │ PROJ-1234 │ Shipped branch 'feature/PROJ-1234'            │ 2026-04-15 14:02 │
│ 2 │ start │ PROJ-5678 │ Created workspace at /home/user/myapp.5678    │ 2026-04-15 13:55 │
│ 1 │ start │ PROJ-1234 │ Created workspace at /home/user/myapp.1234    │ 2026-04-15 09:14 │
╰───┴───────┴───────────┴───────────────────────────────────────────────┴──────────────────╯

# Filter by ticket
$ parsec log PROJ-1234

# Last 3 entries only
$ parsec log --last 3
```

---

### `parsec undo`

Undo the last parsec operation.

- Undo `start` or `adopt`: removes the worktree and deletes the branch
- Undo `ship` or `clean`: re-creates the worktree from the branch (if still available locally or on remote)

```
parsec undo [--dry-run]
```

| Option | Description |
|--------|-------------|
| `--dry-run` | Preview what would be undone |

```bash
# Preview
$ parsec undo --dry-run
Would undo: start PROJ-5678
  Would remove worktree at /home/user/myapp.PROJ-5678
  Would delete branch 'feature/PROJ-5678'

# Execute
$ parsec undo
Undid start for PROJ-5678
  Worktree removed.

# Nothing to undo
$ parsec undo
Error: nothing to undo. Run `parsec log` to see operation history.
```

---

### `parsec sync [ticket]`

Fetch the latest base branch and rebase (or merge) the worktree on top. Detects the current worktree automatically when no ticket is given.

```
parsec sync [ticket] [--all] [--strategy rebase|merge]
```

| Option | Description |
|--------|-------------|
| `--all` | Sync all active worktrees |
| `--strategy` | `rebase` (default) or `merge` |

```bash
# Sync current worktree
$ parsec sync
✓ rebase 1 worktree(s):
  - PROJ-1234

# Sync a specific worktree
$ parsec sync PROJ-5678

# Sync all worktrees at once
$ parsec sync --all

# Use merge instead of rebase
$ parsec sync --strategy merge
```

---

### `parsec open <ticket>`

Open the associated PR/MR or ticket tracker page in your default browser. If the ticket has been shipped, opens the PR by default; otherwise opens the tracker page.

```
parsec open <ticket> [--pr] [--ticket-page]
```

| Option | Description |
|--------|-------------|
| `--pr` | Force open the PR/MR page |
| `--ticket-page` | Force open the ticket tracker page |

```bash
# Open PR if shipped, otherwise ticket page
$ parsec open PROJ-1234
Opening https://github.com/org/repo/pull/42

# Force open the Jira ticket
$ parsec open PROJ-1234 --ticket-page
Opening https://yourcompany.atlassian.net/browse/PROJ-1234

# Force open the PR
$ parsec open PROJ-1234 --pr
Opening https://github.com/org/repo/pull/42
```

---

### `parsec config`

```bash
# Interactive setup wizard
$ parsec config init

# Show current configuration
$ parsec config show
[workspace]
  layout          = sibling
  base_dir        = .parsec/workspaces
  branch_prefix   = feature/

[tracker]
  provider       = jira
  jira.base_url  = https://yourcompany.atlassian.net

[ship]
  auto_pr         = true
  auto_cleanup    = true
  draft           = false

# Output shell integration script
$ parsec config shell zsh

# Generate shell completions
$ parsec config completions zsh

# Install man page
$ sudo parsec config man
```

---

## Global Flags

These flags work on every command:

| Flag | Description |
|------|-------------|
| `--json` | Machine-readable JSON output |
| `-q, --quiet` | Suppress non-essential output |
| `--repo <path>` | Target a different repository |

```bash
$ parsec list --json
$ parsec ship PROJ-1234 --quiet
$ parsec status --repo /path/to/other-repo
```

---

## Shell Integration

`parsec switch` prints a path but cannot `cd` for you. The shell integration wraps `parsec switch` so it changes your directory automatically:

```bash
# Add to ~/.zshrc
eval "$(parsec config shell zsh)"

# Or for bash, add to ~/.bashrc
eval "$(parsec config shell bash)"
```

After sourcing, `parsec switch <ticket>` will `cd` into the worktree directly:

```bash
$ parsec switch PROJ-1234
# Now you're in /home/user/myapp.PROJ-1234
```

---

## Shell Completions

Generate tab-completion scripts for your shell:

```bash
# Zsh — add to ~/.zshrc
eval "$(parsec config completions zsh)"

# Bash — add to ~/.bashrc
eval "$(parsec config completions bash)"

# Fish — add to ~/.config/fish/config.fish
parsec config completions fish | source

# Other shells
parsec config completions elvish
parsec config completions powershell
```

---

## Man Page

Install the man page so `man parsec` works:

```bash
sudo parsec config man
# Man page installed to /usr/local/share/man/man1/parsec.1

# Custom directory
parsec config man --dir ~/.local/share/man
```

---

## Configuration

Config file: `~/.config/parsec/config.toml`

```toml
[workspace]
# "sibling" (default) creates worktrees next to repo: ../repo.ticket/
# "internal" creates inside repo: .parsec/workspaces/ticket/
layout = "sibling"
base_dir = ".parsec/workspaces"
branch_prefix = "feature/"

[tracker]
# "jira" | "github" | "gitlab" | "none"
provider = "jira"

[tracker.jira]
base_url = "https://yourcompany.atlassian.net"
# Auth: PARSEC_JIRA_TOKEN or JIRA_PAT env var

[tracker.gitlab]
base_url = "https://gitlab.com"
# Auth: PARSEC_GITLAB_TOKEN env var

[ship]
auto_pr = true        # Create PR/MR on ship
auto_cleanup = true   # Remove worktree after ship
draft = false         # Create PRs as drafts

[hooks]
# Commands to run in new worktrees after creation
post_create = ["npm install"]
```

---

## Environment Variables

| Variable | Description |
|----------|-------------|
| `PARSEC_JIRA_TOKEN` | Jira API token (or personal access token) |
| `JIRA_PAT` | Alternative Jira token variable |
| `JIRA_BASE_URL` | Jira URL (overrides config) |
| `PARSEC_GITHUB_TOKEN` | GitHub token for PR creation |
| `GITHUB_TOKEN` | Fallback GitHub token |
| `GH_TOKEN` | Fallback GitHub token |
| `PARSEC_GITLAB_TOKEN` | GitLab token for MR creation |
| `GITLAB_TOKEN` | Fallback GitLab token |

Token priority: `PARSEC_*_TOKEN` > platform-specific variables.

---

## Comparison with Alternatives

| Feature | parsec | GitButler | worktrunk | git worktree | git-town |
|---------|--------|-----------|-----------|--------------|----------|
| Ticket tracker integration | Jira + GitHub Issues | No | No | No | No |
| Physical isolation | Yes (worktrees) | No (virtual branches) | Yes | Yes | No |
| Conflict detection | Cross-worktree | N/A | No | No | No |
| One-step ship (push+PR+clean) | Yes | No | No | No | Yes |
| GitHub + GitLab | Both | Both | No | No | No |
| Operation history + undo | Yes | Yes | No | No | No |
| JSON output | Yes | Yes | Yes | No | No |
| Auto-cleanup merged | Yes | No | Yes | Manual | No |
| GUI | CLI only | Desktop + TUI | CLI | CLI | CLI |
| Zero config start | Yes | No | Yes | No | No |

---

## License

MIT
