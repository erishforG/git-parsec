# git-parsec

[![Crates.io](https://img.shields.io/crates/v/git-parsec.svg)](https://crates.io/crates/git-parsec)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![CI](https://github.com/erishforG/git-parsec/actions/workflows/ci.yml/badge.svg)](https://github.com/erishforG/git-parsec/actions)

> Git worktree lifecycle manager for parallel AI agent workflows

**parsec** manages isolated git worktrees tied to tickets (Jira, GitHub Issues), enabling multiple AI agents or developers to work on the same repository in parallel without lock conflicts.

![demo](demo.gif)

## What is parsec?

**parsec** is a command-line tool (CLI) that automates the full lifecycle of git worktrees: create an isolated workspace from a ticket ID, work in parallel without lock conflicts, then push + create PR + clean up in one command. It integrates with **Jira**, **GitHub Issues**, and **GitLab Issues** for automatic ticket title lookup, and supports **GitHub** and **GitLab** for PR/MR creation.

Unlike plain `git worktree`, parsec tracks workspace state, detects file conflicts across worktrees, provides operation history with undo, supports stacked PRs, and offers CI status monitoring — all from a single CLI.

---

## Why parsec?

### What changes day-to-day

| What you do today | With parsec |
|---|---|
| `git checkout -b feat/xyz`, `git worktree add`, configure manually | `parsec start TICKET` |
| `git push`, `gh pr create`, `git worktree remove`, delete branch | `parsec ship TICKET` |
| Open GitHub web UI to check CI results | `parsec ci --watch` |
| Merge PR on GitHub, delete branch, clean local worktree | `parsec merge TICKET` |

### Key metrics

- **PR lead time**: Less time between starting a ticket and opening a PR — no setup friction, no stash management
- **Context switches eliminated**: Jump between tickets without losing state; each ticket lives in its own directory
- **Conflict prevention**: `parsec conflicts` catches cross-ticket file collisions before they become merge problems
- **0 `index.lock` conflicts**: Every worktree has its own `.git` index — no serialized writes

### Use cases

**Solo developer** — Work on several tickets in parallel without stashing. Ship complete features (push + PR + cleanup) with one command. See all in-flight work at a glance with `parsec list`.

**Team** — View the active sprint as a Kanban board with `parsec board`. Detect which tickets touch the same files before review. Monitor all open PRs and their CI status from the terminal.

**AI agent orchestration** — Run multiple coding agents on the same repo simultaneously. Every agent gets its own isolated worktree with no `index.lock` contention. Use `--json` on every command for structured output agents can parse directly.

---

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
- **Attach to existing branches** -- Start a workspace from an existing local or remote branch with `--branch`
- **Operation history and undo** -- `parsec log` shows what happened, `parsec undo` reverts it
- **Keep branches fresh** -- `parsec sync` rebases or merges the latest base branch into any worktree
- **Agent-friendly output** -- `--json` flag on every command for machine consumption
- **Status dashboard** -- See all parallel work at a glance
- **Auto-cleanup** -- Remove worktrees for merged branches automatically
- **GitHub and GitLab** -- PR and MR creation for both platforms
- **Stacked PRs** -- Create dependent PR chains with `--on` and sync the entire stack
- **Sprint board view** -- See the active sprint as a Kanban board with `parsec board`
- **Environment diagnostics** -- `parsec doctor` validates your setup and shows what needs fixing
- **Pre-ship hooks** -- Run custom commands before shipping with configurable `[hooks]` pre_ship
- **Issue creation** -- Create GitHub/Jira issues and start worktrees in one step with `parsec create`
- **Release workflow** -- Merge, tag, and create GitHub Releases with `parsec release`
- **PR reviewers and labels** -- Assign reviewers and labels on ship with `--reviewer`/`--label` or config defaults
- **Stack submit** -- Ship an entire stack in topological order with `parsec stack --submit`
- **Cross-platform** -- Tested on Linux, macOS, and Windows CI

---

## Impact

### Concrete time savings

The typical "start ticket, work, ship" flow goes from 5+ commands to 2:

```bash
# Before parsec
git fetch origin
git checkout -b feature/PROJ-1234 origin/main
# ... open browser, look up Jira ticket title ...
git push -u origin feature/PROJ-1234
gh pr create --title "Add user authentication" --base main
git checkout main
git worktree remove ../myapp.PROJ-1234

# With parsec: 2 commands, no browser
parsec start PROJ-1234   # fetches title from Jira automatically
parsec ship PROJ-1234    # push + PR + cleanup in one step
```

### Concrete risk reduction

- **0 `index.lock` conflicts** — worktree isolation is physical; each workspace has its own `.git/index`
- **Conflict detection before it hurts** — `parsec conflicts` shows cross-worktree file overlap before any push
- **Undo for mistakes** — `parsec undo` reverses the last operation (start, ship, clean)

### Token efficiency for AI agents

Traditional AI agents waste tokens calling raw APIs. Each Jira or GitHub API call costs dozens of tokens for auth setup, pagination, and response parsing. **parsec packages git + tracker operations into single commands with structured output.**

#### Before: Raw API Calls

```bash
# Agent needs: sprint tickets + status + worktree info + PR status
# Step 1: Authenticate with Jira API
# Step 2: Find active sprint (GET /rest/agile/1.0/board/{id}/sprint?state=active)
# Step 3: Fetch sprint issues (GET /rest/agile/1.0/sprint/{id}/issue)
# Step 4: For each ticket, check local worktrees (git worktree list, parse output)
# Step 5: For each ticket, check PR status (GitHub API)
# → 5+ API calls, 100+ tokens, custom parsing logic
```

#### After: One parsec Command

```bash
parsec board --json
# → Sprint + status-grouped tickets + worktree/PR flags in one structured JSON
```

#### Key benefits for AI agents

| Capability | What it means |
|------------|---------------|
| `--json` on every command | Structured output AI can parse instantly |
| `parsec start` | git worktree + Jira fetch + state management in one call |
| `parsec board --json` | Sprint + tickets + worktree/PR status in one call |
| `parsec ship` | Push + PR creation + cleanup in one call |
| Env var defaults | Zero-arg commands after one-time setup |
| Conflict detection | AI agents can check before parallel edits |

---

## Use Cases

### Solo developer

Work on multiple tickets in parallel without stashing or losing context. Each ticket lives in its own sibling directory, so switching is just `cd`.

```bash
parsec start PROJ-1234     # new worktree from Jira ticket
parsec start PROJ-5678     # second worktree, works in parallel
cd $(parsec switch PROJ-1234)
# ... make changes, commit normally ...
parsec ship PROJ-1234      # push + PR + cleanup
```

### Team

Keep the whole team's sprint visible from the terminal. Catch file conflicts before they become merge problems. Track all open PRs without leaving the shell.

```bash
parsec board               # sprint board: In Progress / In Review / Done
parsec conflicts           # which tickets touch the same files?
parsec pr-status           # CI and review state for all open PRs
parsec ci PROJ-1234 --watch  # wait for CI to go green
```

### AI agent orchestration

Run multiple coding agents on the same repo simultaneously. Each agent calls `parsec start` to get an isolated worktree, uses `--json` for structured output, and calls `parsec ship` when done. No `index.lock` contention, no custom shell parsing.

```bash
# Agent 1
parsec start PROJ-100 --json   # isolated workspace, structured response

# Agent 2 (same repo, same time)
parsec start PROJ-101 --json   # separate worktree, no collision

# Coordinator
parsec conflicts --json        # detect overlap before agents commit
parsec board --json            # full sprint + PR status in one call
```

---

## Installation

### Pre-built binaries (recommended)

Download the latest release for your platform from [GitHub Releases](https://github.com/erishforG/git-parsec/releases):

```bash
# macOS (Apple Silicon)
curl -LO https://github.com/erishforG/git-parsec/releases/latest/download/parsec-{version}-aarch64-apple-darwin.tar.gz
tar xzf parsec-*-aarch64-apple-darwin.tar.gz
sudo mv parsec /usr/local/bin/

# macOS (Intel)
curl -LO https://github.com/erishforG/git-parsec/releases/latest/download/parsec-{version}-x86_64-apple-darwin.tar.gz

# Linux (x86_64)
curl -LO https://github.com/erishforG/git-parsec/releases/latest/download/parsec-{version}-x86_64-unknown-linux-gnu.tar.gz

# Windows — download .zip from the Releases page
```

### Via Cargo

```bash
cargo install git-parsec
```

### Build from source

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

## Command Reference

| Command | What it does |
|---------|-------------|
| [`parsec start`](#parsec-start-ticket) | Create an isolated worktree for a ticket |
| [`parsec adopt`](#parsec-adopt-ticket) | Import an existing branch into parsec management |
| [`parsec list`](#parsec-list) | List all active parsec-managed worktrees |
| [`parsec status`](#parsec-status-ticket) | Show detailed status of a workspace |
| [`parsec ticket`](#parsec-ticket-ticket) | View ticket details from the configured tracker |
| [`parsec ship`](#parsec-ship-ticket) | Push, create PR/MR, and clean up in one step |
| [`parsec clean`](#parsec-clean) | Remove worktrees for merged branches |
| [`parsec conflicts`](#parsec-conflicts) | Detect files modified in more than one worktree |
| [`parsec switch`](#parsec-switch-ticket) | Print (or cd to) a ticket's worktree path |
| [`parsec log`](#parsec-log-ticket) | Show operation history |
| [`parsec undo`](#parsec-undo) | Undo the last parsec operation |
| [`parsec sync`](#parsec-sync-ticket) | Rebase/merge latest base branch into a worktree |
| [`parsec open`](#parsec-open-ticket) | Open PR or ticket page in browser |
| [`parsec pr-status`](#parsec-pr-status-ticket) | Check CI and review status of shipped PRs |
| [`parsec ci`](#parsec-ci-ticket---watch---all) | Check CI pipeline status for a PR |
| [`parsec merge`](#parsec-merge-ticket---rebase---no-wait---no-delete-branch) | Merge a PR from the terminal |
| [`parsec diff`](#parsec-diff-ticket---stat---name-only) | View changes vs base branch |
| [`parsec stack`](#parsec-stack---sync---submit) | View and manage stacked PR dependencies |
| [`parsec board`](#parsec-board) | Show sprint as a Kanban board |
| [`parsec init`](#parsec-init) | Install shell integration |
| [`parsec config`](#parsec-config) | Configure parsec |
| [`parsec doctor`](#parsec-doctor) | Validate environment and configuration |
| [`parsec create`](#parsec-create) | Create a new issue and optionally start a worktree |
| [`parsec new-issue`](#parsec-new-issue) | Create a new issue (alias with extra options) |
| [`parsec release`](#parsec-release-version) | Merge, tag, and create a GitHub Release |
| [`parsec rename`](#parsec-rename-ticket---new-ticket-id) | Re-ticket a workspace to a different ticket ID |

---

### `parsec start <ticket>`

Create an isolated worktree for a ticket. Fetches the ticket title from your configured tracker (Jira, GitHub Issues) or accepts a manual title.

```
parsec start <ticket> [--base <branch>] [--title "text"] [--on <parent-ticket>] [--branch <name>] [--hook "cmd"]
```

| Option | Description |
|--------|-------------|
| `-b, --base <branch>` | Base branch to create from (default: main/master) |
| `--title "text"` | Set ticket title manually, skip tracker lookup |
| `--on <ticket>` | Stack on another ticket's branch (for dependent PRs) |
| `--branch <name>` | Use an existing branch instead of creating a new one |
| `--hook "cmd"` | Run a command after worktree creation (one-off hook) |

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

# Attach to an existing branch (local or remote)
$ parsec start CL-2208 --branch feature/CL-2208

# Attach to a remote-only branch (auto-fetches and tracks)
$ parsec start CL-2208 --branch origin/feature/CL-2208

# Run a setup command after creation
$ parsec start PROJ-42 --hook "npm install"
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
parsec list [--full] [--no-pr]
```

```bash
$ parsec list
╭────────┬────────────────┬────────┬──────────────────┬────────────────────────────╮
│ Ticket │ Branch         │ Status │ Created          │ Path                       │
├────────┼────────────────┼────────┼──────────────────┼────────────────────────────┤
│ TEST-1 │ feature/TEST-1 │ active │ 2026-04-15 09:00 │ /home/user/myapp.TEST-1    │
│ TEST-2 │ feature/TEST-2 │ active │ 2026-04-15 09:05 │ /home/user/myapp.TEST-2    │
╰────────┴────────────────┴────────┴──────────────────┴────────────────────────────╯

# Show extended metadata per worktree
$ parsec list --full
╭────────┬────────────────┬────────┬──────────────┬──────────┬─────────────────────┬───────────┬────────────────────────────╮
│ Ticket │ Branch         │ Status │ Ahead/Behind │ Unpushed │ Last Commit         │ Age       │ Path                       │
├────────┼────────────────┼────────┼──────────────┼──────────┼─────────────────────┼───────────┼────────────────────────────┤
│ TEST-1 │ feature/TEST-1 │ active │ +3 / -0      │ 1        │ Add rate limiting   │ 2h ago    │ /home/user/myapp.TEST-1    │
│ TEST-2 │ feature/TEST-2 │ active │ +1 / -2      │ 0        │ Fix auth redirect   │ 30m ago   │ /home/user/myapp.TEST-2    │
╰────────┴────────────────┴────────┴──────────────┴──────────┴─────────────────────┴───────────┴────────────────────────────╯

$ parsec list --json
[{"ticket":"TEST-1","path":"/home/user/myapp.TEST-1","branch":"feature/TEST-1","base_branch":"main","created_at":"2026-04-15T09:00:00Z","ticket_title":"Add auth","status":"active"}]
```

| Option | Description |
|--------|-------------|
| `--full` | Show extended metadata (commits, divergence, last commit) |
| `--no-pr` | Skip PR status lookup (faster, works offline) |

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

### `parsec ticket [ticket]`

View ticket details from the configured tracker. Auto-detects the ticket from the current worktree if no argument is given.

```
parsec ticket [ticket]
```

```bash
# Auto-detect from current worktree
$ parsec ticket
CL-2283: Implement rate limiting for API endpoints
  Status: In Progress
  Assignee: eric.signal
  URL: https://jira.example.com/browse/CL-2283

# Explicit ticket
$ parsec ticket CL-2283

# JSON output
$ parsec ticket CL-2283 --json
{"id":"CL-2283","title":"Implement rate limiting","status":"In Progress","assignee":"eric.signal","url":"https://jira.example.com/browse/CL-2283"}
```

---

### `parsec ship <ticket>`

Push the branch, create a PR (GitHub) or MR (GitLab), and clean up the worktree. The forge is auto-detected from the remote URL.

```
parsec ship <ticket> [--draft] [--no-pr] [--base <branch>] [--skip-hooks] [--reviewer <user>]... [--label <name>]...
```

| Option | Description |
|--------|-------------|
| `--draft` | Create the PR/MR as a draft |
| `--no-pr` | Push only, skip PR/MR creation |
| `--base <branch>` | Target base branch for PR (overrides config `default_base` and worktree base) |
| `--skip-hooks` | Skip pre-ship hooks defined in config |
| `-r, --reviewer <user>` | Request review from a GitHub user (repeatable) |
| `-l, --label <name>` | Add a label to the PR (repeatable) |

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

# Ship with reviewers and labels
$ parsec ship PROJ-1234 --reviewer alice --reviewer bob --label "needs-review"
```

Reviewers and labels can also be set as defaults in config:

```toml
[ship]
default_reviewers = ["alice", "bob"]
default_labels = ["team-backend"]
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

### `parsec pr-status [ticket]`

Check the CI and review status of shipped PRs. Shows CI check results, review approvals, and merge state in a color-coded table.

```
parsec pr-status [ticket]
```

```bash
# Check a specific ticket's PR
$ parsec pr-status PROJ-1234
┌───────────┬─────┬────────┬──────────┬──────────────┐
│ Ticket    │ PR  │ State  │ CI       │ Reviews      │
├───────────┼─────┼────────┼──────────┼──────────────┤
│ PROJ-1234 │ #42 │ open   │ ✓ passed │ ✓ approved   │
└───────────┴─────┴────────┴──────────┴──────────────┘

# Check all shipped PRs
$ parsec pr-status

# JSON output
$ parsec pr-status PROJ-1234 --json
```

Requires: `PARSEC_GITHUB_TOKEN` (or `GITHUB_TOKEN`, `GH_TOKEN`)

---

### `parsec ci [ticket] [--watch] [--all]`

Check CI/CD pipeline status for a ticket's PR. Shows individual check runs with status, duration, and an overall summary.

```
parsec ci [ticket] [--watch] [--all]
```

| Option | Description |
|--------|-------------|
| `ticket` | Ticket identifier (auto-detects current worktree if omitted) |
| `--watch` | Poll CI every 5s until all checks complete |
| `--all` | Show CI for all shipped PRs |

```bash
# Auto-detect from current worktree
$ parsec ci
CI for PROJ-1234 (PR #42, a1b2c3d)
┌────────────┬───────────┬──────────┐
│ Check      │ Status    │ Duration │
├────────────┼───────────┼──────────┤
│ Tests      │ ✓ passed  │ 2m 15s   │
│ Build      │ ✓ passed  │ 1m 42s   │
│ Lint       │ ● running │ running… │
└────────────┴───────────┴──────────┘
✓ CI: 2/3 — 2 passed, 1 running

# Check a specific ticket
$ parsec ci PROJ-1234

# Watch mode — refreshes every 5s until done
$ parsec ci PROJ-1234 --watch

# All shipped PRs
$ parsec ci --all

# JSON output
$ parsec ci PROJ-1234 --json
```

Requires: `PARSEC_GITHUB_TOKEN` (or `GITHUB_TOKEN`, `GH_TOKEN`)

---

### `parsec merge [ticket] [--rebase] [--no-wait] [--no-delete-branch]`

Merge a ticket's PR directly from the terminal. Waits for CI to pass before merging, then cleans up the local worktree.

```
parsec merge [ticket] [--rebase] [--no-wait] [--no-delete-branch]
```

| Option | Description |
|--------|-------------|
| `ticket` | Ticket identifier (auto-detects current worktree if omitted) |
| `--rebase` | Use rebase merge instead of squash (default: squash) |
| `--no-wait` | Skip CI check before merging |
| `--no-delete-branch` | Keep remote branch after merge |

```bash
# Squash merge (default)
$ parsec merge PROJ-1234
Waiting for CI to pass... ✓
Merged PR #42 for PROJ-1234!
  Method: squash
  SHA:    a1b2c3d

# Rebase merge
$ parsec merge PROJ-1234 --rebase

# Skip CI wait
$ parsec merge PROJ-1234 --no-wait

# JSON output
$ parsec merge PROJ-1234 --json
```

Requires: `PARSEC_GITHUB_TOKEN` (or `GITHUB_TOKEN`, `GH_TOKEN`)

---

### `parsec diff [ticket] [--stat] [--name-only]`

View changes in a worktree compared to its base branch. Uses merge-base for accurate comparison.

```
parsec diff [ticket] [--stat] [--name-only]
```

| Option | Description |
|--------|-------------|
| `ticket` | Ticket identifier (auto-detects current worktree if omitted) |
| `--stat` | Show file-level summary only |
| `--name-only` | List changed file names only |

```bash
# Full diff for current worktree
$ parsec diff

# File summary
$ parsec diff PROJ-1234 --stat

# Just file names
$ parsec diff --name-only

# JSON output (changed files list)
$ parsec diff PROJ-1234 --json
```

---

### `parsec stack [--sync] [--submit]`

View and manage stacked PR dependencies. Worktrees created with `--on` form a dependency chain. PRs include a **stack navigation table** showing parent/child relationships.

```
parsec stack [--sync] [--submit]
```

| Option | Description |
|--------|-------------|
| `--sync` | Rebase the entire stack chain |
| `--submit` | Ship the entire stack in topological order (root first) |

```bash
# Create a stack
$ parsec start PROJ-1 --title "Add models"
$ parsec start PROJ-2 --on PROJ-1 --title "Add API endpoints"
$ parsec start PROJ-3 --on PROJ-2 --title "Add frontend"

# View the dependency graph
$ parsec stack
Stack dependency graph:
└── PROJ-1 Add models
    └── PROJ-2 Add API endpoints
        └── PROJ-3 Add frontend

# Sync the entire stack
$ parsec stack --sync

# Ship creates PRs with correct base branches
$ parsec ship PROJ-1   # PR to main
$ parsec ship PROJ-2   # PR to feature/PROJ-1
$ parsec ship PROJ-3   # PR to feature/PROJ-2

# Or ship the entire stack at once
$ parsec stack --submit
Submitting stack (3 worktrees):
  1. PROJ-1
  2. PROJ-2
  3. PROJ-3
Stack submit complete: 3/3 shipped
```

Each PR body includes a stack navigation table:

| | Ticket | Branch |
|---|--------|--------|
| ⬆ Parent | PROJ-1 | `feature/PROJ-1` |
| **➡ Current** | **PROJ-2** | **`feature/PROJ-2`** |
| ⬇ Child | PROJ-3 | `feature/PROJ-3` |

---

### `parsec board`

Show the active sprint as a vertical board view. Fetches tickets from Jira grouped by status column, with worktree and PR indicators.

```
parsec board [--project <KEY>] [--board-id <ID>] [--assignee <name>] [--all]
```

| Option | Description |
|--------|-------------|
| `-p, --project <KEY>` | Jira project key (default from env/config) |
| `--board-id <ID>` | Jira board ID (auto-detected from project) |
| `--assignee <name>` | Filter by assignee (default from env/config) |
| `--all` | Show all tickets (ignore assignee filter) |

```bash
# Show your tickets (with PARSEC_JIRA_ASSIGNEE configured)
$ parsec board

26.04.06 ~ 26.04.20

In Progress (3)
  CL-2283 [wt]  로그 분석 서비스 개발
  CL-2284 [wt]  FDE 대시보드 관련
  CL-2291       반품 요청 API 개발

In Review (2)
  CL-2281 [pr]  ai 커피챗 준비
  CL-2280       이관 요청할 API 정리

# Show all team tickets
$ parsec board --all

# JSON output for AI agents
$ parsec board --json
{"sprint":{"id":123,"name":"...","start":"...","end":"..."},"total_count":48,"columns":{"In Progress":[...],...}}
```

Defaults can be set via environment variables or config file (see below).

---

### `parsec init`

Output or install shell integration for auto-cd on `parsec switch` and CWD recovery after `parsec merge`.

```
parsec init [shell] [--install] [--yes]
```

| Option | Description |
|--------|-------------|
| `shell` | Shell type: `zsh` (default) or `bash` |
| `--install` | Auto-append integration to shell config file |
| `-y, --yes` | Skip confirmation prompt (for scripting) |

```bash
# Print the shell function (pipe to eval)
$ parsec init zsh

# Auto-install into ~/.zshrc
$ parsec init --install
Add shell integration to /home/user/.zshrc? [Y/n] y
Shell integration added. Run `source ~/.zshrc` or restart your shell.

# Non-interactive install
$ parsec init --install --yes
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
  # default_base  = "develop"  # Target branch for PRs (default: worktree base)

# Output shell integration script
$ parsec config shell zsh

# Generate shell completions
$ parsec config completions zsh

# Install man page
$ sudo parsec config man
```

---

### `parsec doctor`

Validate your environment and configuration. Prints ✓/✗ for each check with actionable fix instructions.

```bash
$ parsec doctor
parsec doctor
  ✓ git version 2.43.0 (worktree support ok)
  ✓ config file found at ~/.config/parsec/config.toml
  ✓ GitHub token configured (github.com) via gh auth token
  ✗ shell integration not found in shell config
    Add to ~/.zshrc:  eval "$(parsec init zsh)"
  ✗ tab completions not configured
    Add to ~/.zshrc:  eval "$(parsec config completions zsh)"
  ✓ remote origin accessible

2 check(s) failed.

$ parsec doctor --json
{"checks":[...],"all_ok":false}
```

**AI agent mode** — output parsec workflow rules as a Markdown document for AI agents to consume:

```bash
$ parsec doctor --ai
# Outputs structured Markdown with workflow rules, command patterns,
# and best practices for AI agents using parsec
```

---

### `parsec create`

Create a new issue on the configured tracker (GitHub Issues or Jira) and optionally start a worktree for it immediately.

```
parsec create --title "text" [--body "text"] [--label "a,b"] [--project KEY] [--start]
```

| Option | Description |
|--------|-------------|
| `--title "text"` | Issue title (required) |
| `--body "text"` | Issue body/description |
| `--label "a,b"` | Comma-separated labels |
| `-p, --project KEY` | Jira project key (auto-detected from config) |
| `--start` | Start a worktree after creation |

```bash
# Create a GitHub issue
$ parsec create --title "Fix login redirect" --label "bug"
Created #145: Fix login redirect
  https://github.com/org/repo/issues/145

# Create and immediately start working
$ parsec create --title "Add caching layer" --start
Created #146: Add caching layer
Created workspace for #146 at /home/user/myapp.146
```

---

### `parsec new-issue`

Create a new issue on the tracker (alias for `create` with additional options). Supports GitHub Issues and Jira with configurable issue type.

```
parsec new-issue --title "text" [--body "text"] [--label "a"] [--project KEY] [--issue-type TYPE] [--start]
```

| Option | Description |
|--------|-------------|
| `--title "text"` | Issue title (required) |
| `--body "text"` | Issue body/description |
| `--label "a"` | Labels (can be specified multiple times) |
| `-p, --project KEY` | Jira project key (auto-detected from config) |
| `--issue-type TYPE` | Jira issue type (default: Task) |
| `--start` | Auto-start a worktree for the new issue |

```bash
# Create with issue type for Jira
$ parsec new-issue --title "Implement API caching" --issue-type Story --project CL

# Multiple labels
$ parsec new-issue --title "Fix auth bug" --label bug --label priority
```

---

### `parsec release <version>`

Create a release: merge develop to main, create a git tag, and optionally create a GitHub Release with auto-generated changelog.

```
parsec release <version> [--from <branch>] [--no-github-release] [--dry-run]
```

| Option | Description |
|--------|-------------|
| `<version>` | Version string (e.g., "0.3.0") |
| `--from <branch>` | Source branch to release from (default: develop) |
| `--no-github-release` | Skip creating GitHub Release |
| `--dry-run` | Show what would happen without making changes |

```bash
# Full release
$ parsec release 0.3.0
✓ Merged develop → main
✓ Tagged v0.3.0
✓ GitHub Release created: https://github.com/org/repo/releases/tag/v0.3.0

# Dry run first
$ parsec release 0.4.0 --dry-run

# Skip GitHub Release
$ parsec release 0.3.1 --no-github-release
```

---

### `parsec rename <ticket> --new <ticket-id>`

Re-ticket an existing workspace to a different ticket ID. Renames the branch and updates internal state. Useful when a ticket is split or re-assigned.

```
parsec rename <ticket> --new <ticket-id>
```

| Option | Description |
|--------|-------------|
| `--new <ticket-id>` | New ticket ID to assign (required) |

```bash
# Re-ticket a workspace
$ parsec rename PROJ-100 --new PROJ-200
Renamed PROJ-100 → PROJ-200
  Branch: feature/PROJ-100 → feature/PROJ-200
  Path: /home/user/myapp.PROJ-200

# JSON output
$ parsec rename PROJ-100 --new PROJ-200 --json
```

---

## Global Flags

These flags work on every command:

| Flag | Description |
|------|-------------|
| `--dry-run` | Preview what a command would do without making changes |
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
# Preferred: auto-install (appends to your shell config with confirmation)
$ parsec init --install
Add shell integration to /home/user/.zshrc? [Y/n] y
Shell integration added to /home/user/.zshrc. Run `source ~/.zshrc` or restart your shell.

# Or with --yes for scripted setup
$ parsec init --install --yes

# Manual: add to ~/.zshrc yourself
eval "$(parsec init zsh)"

# Or for bash
eval "$(parsec init bash)"
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

[worktree]
# Directories to share from the main repo into new worktrees so that
# `parsec start` doesn't trigger a cold rebuild. Default is empty (no sharing).
shared_cache = ["target", "node_modules", ".venv"]
# "symlink" (default): fast, zero-disk overhead. All worktrees and the main
#                      repo share one cache — running parallel builds of the
#                      same artifact may race.
# "copy":    full copy at start time. Each worktree gets an independent cache,
#            no race risk, but uses more disk and the initial copy takes time.
cache_strategy = "symlink"

[tracker]
# "jira" | "github" | "gitlab" | "none"
provider = "jira"

[tracker.jira]
base_url = "https://yourcompany.atlassian.net"
# Auth: PARSEC_JIRA_TOKEN or JIRA_PAT env var
# project = "CL"            # Default project for board
# board_id = 123            # Default board ID
# assignee = "eric.signal"  # Default assignee filter

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
# Commands to run before shipping (pre-push hooks)
pre_ship = ["cargo test", "cargo clippy"]

[release]
# branch = "main"       # Target release branch (default: main)
# tag_prefix = "v"       # Tag prefix (default: "v")
# changelog = true       # Generate changelog in release notes

[policy]
# protected_branches = ["main", "develop", "release/*"]  # Branches that cannot be shipped to
# allowed_ship_targets = ["develop"]                       # Restrict PR target branches
# require_ci = false                                        # Require CI pass before merge

[tracker.auto_transition]
# on_start = "In Progress"   # Transition when `parsec start` runs
# on_ship = "In Review"      # Transition when `parsec ship` runs
# on_merge = "Done"           # Transition when `parsec merge` runs
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
| `PARSEC_JIRA_PROJECT` | Default Jira project key for `board` |
| `PARSEC_JIRA_BOARD_ID` | Default Jira board ID for `board` |
| `PARSEC_JIRA_ASSIGNEE` | Default assignee filter for `board` |

Token priority: `PARSEC_*_TOKEN` > platform-specific variables.

---

## Error Codes

When using `--json`, errors include a structured error code for programmatic handling:

| Code | Meaning | Exit Code |
|------|---------|-----------|
| E001 | No authentication token configured | 2 |
| E002 | CI checks failing | 4 |
| E003 | Merge conflicts detected | 3 |
| E004 | PR not mergeable | 5 |
| E005 | Workspace not found | 5 |
| E006 | Workspace already exists | 5 |
| E007 | No active workspaces | 5 |
| E008 | Pre-ship hook failed | 1 |
| E009 | Policy violation | 6 |
| E010 | PR not found | 5 |
| E011 | Tracker not configured | 2 |
| E012 | Ship partially completed | 1 |
| E013 | Cannot undo operation | 1 |

```bash
# JSON error output example
$ parsec ship PROJ-1234 --json 2>&1
{"error":{"code":"E001","message":"No GitHub token configured","hint":"Set PARSEC_GITHUB_TOKEN or run gh auth login"}}
$ echo $?
2
```

---

## Comparison with Alternatives

| Feature | parsec | GitButler | worktrunk | git worktree | git-town |
|---------|--------|-----------|-----------|--------------|----------|
| Ticket tracker integration | Jira + GitHub Issues | No | No | No | No |
| Physical isolation | Yes (worktrees) | No (virtual branches) | Yes (worktrees) | Yes | No |
| Conflict detection | Cross-worktree | N/A | No | No | No |
| One-step ship (push+PR+clean) | Yes | No | No | No | Yes |
| GitHub + GitLab | Both | Both | GitHub | No | GitHub, GitLab, Gitea, Bitbucket |
| Operation history + undo | Yes | Yes | No | No | Yes (undo) |
| JSON output | Yes | Yes | No | No | No |
| CI monitoring | Yes (--watch) | No | No | No | No |
| Stacked PRs | Yes | Yes | No | No | Yes |
| Auto-cleanup merged | Yes | No | No | Manual | No |
| Post-create hooks | Yes | No | Yes | No | No |
| Issue creation from CLI | Yes | No | No | No | No |
| AI token efficiency | Single-command ops | N/A | N/A | N/A | N/A |
| GUI | CLI only | Desktop + TUI | CLI | CLI | CLI |
| Zero config start | Yes | No | Yes | No | No |

---

## FAQ

**How do I set up parsec with Jira?**
Set `PARSEC_JIRA_TOKEN` (or `JIRA_PAT`) and configure `[tracker.jira]` in `~/.config/parsec/config.toml` with your `base_url` and `project`. Run `parsec config init` for interactive setup, or `parsec doctor` to validate.

**How do I set up parsec with GitHub Issues?**
Set `provider = "github"` under `[tracker]` in your config. Authentication uses `PARSEC_GITHUB_TOKEN`, `GITHUB_TOKEN`, `GH_TOKEN`, or `gh auth token` automatically.

**Does parsec support GitLab?**
Yes. parsec supports GitLab for both issue tracking and MR creation. Set `provider = "gitlab"` and configure `[tracker.gitlab]` with your `base_url`. Set `PARSEC_GITLAB_TOKEN` for authentication.

**Can I use parsec without a ticket tracker?**
Yes. Set `provider = "none"` or use `--title` with `parsec start` to skip tracker lookup entirely.

**How do stacked PRs work?**
Use `parsec start CHILD --on PARENT` to create dependent worktrees. `parsec ship` automatically sets the correct base branch. `parsec stack --sync` rebases the entire chain.

**What happens if two worktrees modify the same file?**
`parsec conflicts` detects cross-worktree file overlap before you push. It compares changed files across all active worktrees and warns about collisions.

**Can I undo a ship or clean?**
Yes. `parsec undo` reverses the last operation. For `ship`, it re-creates the worktree from the branch. For `clean`, it restores from the remote branch if still available.

**How do I protect branches from accidental shipping?**
Add a `[policy]` section to your config with `protected_branches` and `allowed_ship_targets`. parsec will reject operations that violate these rules.

**Does parsec work with GitHub Enterprise?**
Yes. parsec auto-detects GitHub Enterprise from the remote URL and routes API calls to the correct host. Token resolution is host-aware.

**How do AI agents use parsec?**
Every command supports `--json` for structured output. Run `parsec doctor --ai` to get a Markdown document with workflow rules and command patterns optimized for AI agent consumption.

---

## License

MIT
