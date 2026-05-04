# git-parsec

[![Crates.io](https://img.shields.io/crates/v/git-parsec.svg)](https://crates.io/crates/git-parsec)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![CI](https://github.com/erishforG/git-parsec/actions/workflows/ci.yml/badge.svg)](https://github.com/erishforG/git-parsec/actions)

> **From ticket to PR. One command.**
> Git worktree lifecycle manager — isolated workspaces tied to your tickets, with stacked PRs, multi-forge support, and clean shipping.

![demo](demo.gif)

```bash
$ parsec start PROJ-1234     # creates worktree, fetches Jira title, sets up branch
$ parsec ship PROJ-1234      # pushes, opens PR, cleans worktree
```

That's the whole loop. Plain `git worktree` doesn't track state, doesn't talk to your tracker, doesn't open PRs, doesn't clean up. **parsec** does.

---

## Why use it

- **No `index.lock` collisions** — every workspace has its own `.git/index`, so multiple developers (or AI agents) can run `git add` on the same repo at the same time.
- **One command per phase** — `start` (worktree + tracker fetch), `ship` (push + PR + cleanup), `merge` (merge + branch cleanup), `clean` (sweep merged branches). No web UI round-trips.
- **Stacked PRs that don't melt your brain** — `parsec start FOO --on BAR` chains workspaces; `parsec stack --submit` opens the whole stack in topological order with auto-posted "← prev / next →" navigation comments.
- **Cross-worktree conflict detection** — `parsec conflicts` flags files modified in two workspaces *before* anyone pushes.
- **Multi-forge** — GitHub, GitLab, Bitbucket Cloud. Multi-tracker — Jira, GitHub Issues, GitLab Issues, Bitbucket.
- **Agent-friendly** — `--json` on every command, structured error codes, JSONL execution log, headless/offline modes.

---

## Install

```bash
# Homebrew / pre-built binary (recommended)
curl -LO https://github.com/erishforG/git-parsec/releases/latest/download/parsec-x86_64-unknown-linux-gnu.tar.gz
tar xzf parsec-*.tar.gz && sudo mv parsec /usr/local/bin/

# Cargo (Rust toolchain required)
cargo install git-parsec
```

Other targets (macOS arm64/x86_64, Windows x86_64) ship on every release — see [Releases](https://github.com/erishforG/git-parsec/releases). After install, run `parsec config init` for the interactive first-time setup, then `parsec doctor` to validate.

---

## 60-second tour

```bash
# Pull a ticket from Jira / GitHub / GitLab / Bitbucket and start work
$ parsec start PROJ-1234
✓ Worktree: ../myapp.PROJ-1234
  Title:    Add rate limiting (fetched from Jira)
  Branch:   feature/PROJ-1234

# See everything in flight
$ parsec list
╭───────────┬───────────────────┬────────┬─────────────────────────╮
│ Ticket    │ Branch            │ Status │ Path                    │
├───────────┼───────────────────┼────────┼─────────────────────────┤
│ PROJ-1234 │ feature/PROJ-1234 │ active │ ../myapp.PROJ-1234      │
│ PROJ-5678 │ feature/PROJ-5678 │ active │ ../myapp.PROJ-5678      │
╰───────────┴───────────────────┴────────┴─────────────────────────╯

# Hop in (shell integration auto-cd's)
$ parsec switch PROJ-1234

# Make commits the normal way, then ship — push + PR + cleanup in one shot
$ parsec ship PROJ-1234
✓ Pushed feature/PROJ-1234
✓ PR opened: github.com/org/repo/pull/42
✓ Worktree cleaned up

# Watch CI without leaving the terminal
$ parsec ci PROJ-1234 --watch

# Merge from terminal once CI is green
$ parsec merge PROJ-1234
```

---

## Top features

### 🌿 Stacked PRs that don't melt your brain
```bash
$ parsec start PROJ-2 --on PROJ-1   # new worktree on top of PROJ-1's branch
$ parsec stack --submit              # open all PRs in the stack, root first
```
parsec auto-posts `← previous PR` / `next PR →` navigation comments so reviewers can walk the chain.

### 🔄 Multi-forge, multi-tracker
- **Forges**: GitHub · GitLab · Bitbucket Cloud (full PR lifecycle on each)
- **Trackers**: Jira · GitHub Issues · GitLab Issues · Bitbucket
- **CI status**: GitHub Actions · GitLab CI · Bitbucket Pipelines

`parsec ci` and `pr-status` work the same shape across all of them.

### 🤖 Agent-friendly by design
Every command has `--json`. Errors emit structured codes (E001…E013). `parsec log --export` outputs JSONL with execution IDs and per-step timing for tooling/agents to consume. `--offline` and `[behavior].offline` config skip all network ops for air-gapped or CI environments.

### 🧹 Lifecycle hygiene
`parsec clean` sweeps worktrees for already-merged branches. `parsec conflicts` flags cross-worktree file overlap before you push. `parsec undo` reverses the last operation (start, ship, clean). `parsec doctor` validates every part of your setup with actionable fix instructions.

### 📂 Worktree build cache sharing
`[worktree].shared_cache = ["target", "node_modules", ".venv"]` lets new worktrees reuse the main repo's caches via symlink (default) or copy. Eliminates cold-build cost on `parsec start` for any project with significant dependency caches.

### 📋 Sprint board + issue creation
`parsec board` turns your active sprint into a Kanban board in the terminal. `parsec create` and `parsec new-issue` open issues in your tracker without leaving the shell.

> 27 commands total — see the [full command reference](https://erishforg.github.io/git-parsec/reference/) for every flag and example.

---

## Configuration (minimal example)

```toml
# ~/.config/parsec/config.toml
#:schema https://json.schemastore.org/parsec.json

[workspace]
layout         = "sibling"      # ../repo.ticket/  (alt: "internal")
branch_prefix  = "feature/"
offline        = false           # true = skip all network ops

[tracker]
provider = "jira"               # jira | github | gitlab | bitbucket | none

[tracker.jira]
base_url = "https://yourcompany.atlassian.net"
# Auth: PARSEC_JIRA_TOKEN env var

[ship]
auto_pr      = true
auto_cleanup = true
draft        = false             # true = open PRs as drafts
# template   = ".github/PULL_REQUEST_TEMPLATE.md"

[worktree]
shared_cache    = ["target", "node_modules", ".venv"]
cache_strategy  = "symlink"      # alt: "copy"
```

**Auth tokens** (set via env vars, all optional):

```
PARSEC_JIRA_TOKEN       PARSEC_GITHUB_TOKEN       PARSEC_GITLAB_TOKEN
PARSEC_BITBUCKET_TOKEN  GITHUB_TOKEN (fallback)   GITLAB_TOKEN (fallback)
PARSEC_OFFLINE=1        — force offline mode globally
```

Full schema and every option: `parsec config schema` (also published to [schemastore.org](https://schemastore.org)).

---

## Comparison

| | parsec | GitButler | worktrunk | git worktree | git-town |
|---|---|---|---|---|---|
| Tracker integration | Jira + GitHub + GitLab + Bitbucket | — | — | — | — |
| Physical worktree isolation | ✅ | ❌ (virtual) | ✅ | ✅ | ❌ |
| Cross-worktree conflict detection | ✅ | n/a | ❌ | ❌ | ❌ |
| One-step ship (push + PR + cleanup) | ✅ | ❌ | ❌ | ❌ | ✅ |
| Forges | GitHub + GitLab + Bitbucket | Both | GitHub | — | GitHub, GitLab, Gitea, Bitbucket |
| CI integrations | Actions + GitLab CI + Bitbucket Pipelines | — | — | — | — |
| Operation log + undo | ✅ | ✅ | ❌ | ❌ | partial |
| JSON output | ✅ | ✅ | ❌ | ❌ | ❌ |
| Stacked PRs | ✅ | ✅ | ❌ | ❌ | ✅ |
| GUI | CLI only | Desktop + TUI | CLI | CLI | CLI |

---

## Documentation

- 📘 **[Getting Started Guide](https://erishforg.github.io/git-parsec/guide/)** — install, first ship, tracker config, recipes
- 📗 **[Command Reference](https://erishforg.github.io/git-parsec/reference/)** — every command, every flag, with examples
- 🌐 **[Project home](https://erishforg.github.io/git-parsec/)** — features tour and live demo

---

## Error codes

Every command exits with a structured code. JSON output (`--json`) includes the same code:

| | | |
|---|---|---|
| `E001` no auth token  | `E005` workspace not found       | `E009` policy violation |
| `E002` CI failing     | `E006` workspace already exists  | `E010` PR not found |
| `E003` merge conflict | `E007` no active workspaces      | `E011` tracker not configured |
| `E004` PR not mergeable | `E008` pre-ship hook failed    | `E012` ship partial |
| | | `E013` cannot undo |

```bash
$ parsec ship PROJ-1234 --json 2>&1
{"error":{"code":"E001","message":"No GitHub token configured","hint":"Set PARSEC_GITHUB_TOKEN or run gh auth login"}}
$ echo $?
2
```

---

## License

MIT — see [LICENSE](LICENSE).
