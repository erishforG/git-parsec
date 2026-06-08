# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.5.0] - 2026-06-03 — _The visualization release_

v0.5 completes the **16/16** milestone for polish and power-user UX, adding
six visualization and automation commands that bring worktrees, PRs, and CI
into one view.

### Added
- **`parsec smartlog` (alias `sl`)** — visualizes every active worktree as a
  commit DAG. The ASCII tree groups worktrees by base branch and shows commits
  since the merge base. Phase 2 adds the PR/CI status overlay (`#327`); Phase 3
  adds worktree filtering, ANSI colors, stack indicators (`#333`), and `--json`
  output. (`#245`, `#305`, `#318`, `#319`)
- **`parsec dashboard` (alias `dash`)** — real-time terminal TUI dashboard with
  worktrees, CI state, and GitHub PRs in a 3-pane layout. Built on ratatui and
  crossterm, with `q` to quit, `r` to refresh immediately, `?` for help,
  `--refresh N` for the polling interval, and `--no-overlay` for offline mode.
  (`#248`, `#337`)
- **`parsec test`** — parallel worktree test runner with tree-hash result
  caching. `--all` runs every active worktree, `--jobs N` controls concurrency,
  and `--cache` skips reruns for the same tree. Adds the `[test]` config section
  (`command`, `jobs`, `cache`) and both human-readable and JSON output.
  (`#247`, `#336`)
- **`parsec health`** — health checks for every active worktree. Phase 1 checks
  `.git/index.lock`, uncommitted file counts, and stale worktrees older than
  seven days (`#324`, `#325`). Phase 2 adds CI status overlays and a configurable
  stale threshold (`#330`). Includes five CLI integration tests (`#326`).
- **`parsec reviews`** — shows received and requested PR reviews per worktree
  in one table. Phase 1 (`#301`,
  `#331`) + Phase 2 `--requested` (GitHub Search API) (`#334`).
- **`parsec conflicts --simulate`** — line-level conflict simulation that
  complements the existing filename-overlap heuristic. Uses
  `git merge-tree --write-tree` for two read-only passes: worktree vs base and
  cross-simulation across worktree pairs. Exposes actual conflict files before
  merge.
  (`#246`, `#335`)
- **`parsec commit`** — AI commit message generation via OpenAI or Anthropic.
  Analyzes staged diffs, adds automatic prefixes, supports Conventional Commits
  with `--conventional`, and allows manual overrides with `--message`. (`#274`)
- **`parsec sync`** — auto-syncs `main`/`develop` into stale worktrees with rebase
  or merge strategies, `--all` batch mode, `--dry-run` behind counts, and
  conflict hints. (`#290`)
- **AI-generated PR descriptions** — `parsec ship` can generate PR bodies with
  OpenAI, Anthropic, or Ollama providers via `[ai]` settings. (`#242`, `#275`)
- **`parsec __complete` shell-completion helper** — hidden subcommand that emits
  worktree and branch completion candidates as newline-separated output. Supports
  dynamic tab completion for zsh, bash, and fish
  (`#291`, `#312`). Phase 2 adds dynamic shell scripts (`#328`).
- **`parsec agent` mode (PARSEC_AGENT=1)** — non-interactive JSON output mode
  for AI agent invocations. (`#272`)

### Changed
- **Error messages standardized to 3-line format** — all user-facing errors now
  use the `error: <summary> / caused by: <root cause> / help: <action>` format
  (`#303`, `#306`).

### Fixed
- `parsec ship` falls back to `gh auth token` when `PARSEC_GITHUB_TOKEN` /
  `GITHUB_TOKEN` / `GH_TOKEN` env vars are absent — parity with `parsec doctor`
  and the tracker layer. The fallback is limited to GitHub hosts and does not
  affect Bitbucket or GitLab remotes (`#281`).

### CI
- Windows VS2026 (Visual Studio 2026 runner) pre-validation job to catch MSVC
  toolchain regressions early (`#307`, `#311`).
- Made `parsec test` shell invocation cross-platform (`sh -c` / `cmd /C`) so
  Windows tests no longer invoke WSL.

### Docs
- Expanded module-level RustDoc for `diff` / `history` (`#321`) and
  `stack` / `ci` (`#323`).
- Filled missing CHANGELOG `[Unreleased]` entries for smartlog, completion,
  errors, and win-ci (`#316`, `#317`).

### Tests
- Added broad CLI integration coverage for `compress`, `config schema`,
  `log --export` (`#314`, `#315`), `smartlog` / `sl` (`#318`, `#319`), `health`
  (`#324`, `#326`), `parsec test` (five new tests), `parsec dashboard` (four new
  tests), and `conflicts --simulate` (four new tests).

## [0.4.0] - 2026-05-04

### Added
- **Bitbucket Cloud forge** — full PR lifecycle support (create, list, view,
  merge, comments). New tracker/forge entries `bitbucket` selectable via
  `parsec config` and `[forge]` settings (#240).
- **Bitbucket Pipelines CI integration** — `parsec ci` and `pr-status`
  commands now report Bitbucket Pipelines build state alongside GitHub
  Actions and GitLab CI (#279).
- **`parsec compress` command** — squash a stack of related commits into a
  single tidy commit before shipping, preserving co-author trailers (#236).
- **`parsec ship --template`** — auto-populate the PR description from a
  repository's `.github/PULL_REQUEST_TEMPLATE.md` (or first match under
  `.github/PULL_REQUEST_TEMPLATE/`) (#233).
- **`ship --reviewer` and `--label`** — attach reviewers and labels at PR
  creation time (#261).
- **Stack `--submit`** — open all PRs in a stack in one command (#261).
- **Stack navigation comments** — auto-posted "← prev / next →" comments on
  every PR in a stack so reviewers can walk the chain (#234).
- **`ship.draft` config + `--draft` flag** — open PRs as drafts by default
  when working in throwaway / WIP branches (#238).
- **`[worktree]` shared build cache** — `shared_cache` and `cache_strategy`
  settings let new worktrees reuse `target/`, `node_modules/`, `.venv/`, etc.
  from the main repo via symlink (default) or recursive copy, eliminating
  cold-build cost on `parsec start` (#207).
- **Offline mode toggle** — `[behavior].offline` config and per-command
  `--no-pr` / `--no-tracker` flags so parsec can operate without forge or
  tracker connectivity (#237).
- **Observability lite** — every command run now has an execution ID and
  step timing; opt in to JSONL export via `[observability]` settings for
  tooling/agents to consume (#166).
- **Config JSON Schema + `parsec schema`** — schema published to
  schemastore.org so editors auto-complete `parsec.toml`. The new
  `parsec schema` subcommand emits the schema on demand (#239).
- **Windows CI coverage** — full test matrix on Windows runners (#257).
- 11 new integration tests across forge adapters and worktree paths (#278).

### Changed
- README and reference docs updated to cover ship `--reviewer` / `--label`,
  stack `--submit`, Bitbucket adapter, offline flags, build cache config,
  and `parsec compress` (#265).

### Fixed
- Windows UNC path issue (`\\?\` prefix) breaking worktree operations on
  Windows hosts — resolved via the `dunce` crate (#263).

### CI
- Trigger CI on `release/**` branches in addition to feature branches and
  develop, so release-prep work is exercised before merge (#277).

## [0.3.3] - 2026-04-22

### Added
- Global `--dry-run` flag for all mutating commands (start, ship, clean, sync, merge, rename)
- Policy guard for protected branches and allowed ship targets (`[policy]` config)
- `parsec doctor --ai` — output workflow rules as Markdown for AI agents
- Standard error codes (E001–E013) with structured JSON error format
- PR number stored as first-class field in operation log
- `parsec ship --title` flag for custom PR titles

### Fixed
- reqwest 0.13 missing `rustls` TLS backend feature
- rustls-webpki vulnerabilities (RUSTSEC-2026-0098/0099/0104)
- `parsec open --pr` fails on GitHub Enterprise (hardcoded github.com)
- Doctor command token handling and GHE support
- `.atlassian-env` parser does not strip shell-style quotes
- `unwrap()` panic paths in tracker and stack sync

### Changed
- Major dependency upgrades (reqwest 0.13, colored 3, indicatif 0.18, tabled 0.20, toml 1)
- Extracted Jira config boilerplate into helper function
- Ticket ID validation to prevent path traversal and invalid branch names

## [0.3.2] - 2026-04-21

### Fixed
- Windows binary release upload (use bash shell)

## [0.3.1] - 2026-04-21

### Added
- `parsec rename` command to re-ticket a workspace
- `parsec create` and `parsec new-issue` for creating tracker issues from CLI
- `parsec release` command for automated release workflow
- `parsec doctor` for environment validation
- `parsec list --full` with richer worktree metadata
- `parsec init --install` for automatic shell integration setup
- `parsec switch pr:NUMBER` to checkout PR branches
- `parsec start --hook` for post-create automation
- Pre-ship hooks support (`[hooks] pre_ship`)
- `parsec ci` accepts multiple tickets
- `parsec merge --batch` for merging multiple PRs sequentially
- `parsec clean` supports cleaning a specific ticket
- `parsec ship` auto-closes issues on merge
- Automated multi-platform binary releases
- Documentation site with guide and reference pages

### Fixed
- Jira integration improvements (config token, HTML error, status display)
- Proper error propagation in CI

### Changed
- Output module refactored with `dispatch_output!` macro
- Split monolithic `commands.rs` into per-command modules
- Unified Create and NewIssue into single command
- Typed response structs replace raw `serde_json::Value`
- HTTP client timeout and retry for transient failures
- Idempotency guarantees for ship/merge re-execution
- Expanded integration test coverage

## [0.3.0] - 2026-04-16

### Added
- `parsec board` — sprint as vertical Kanban board
- `parsec ticket` command for tracker lookup with `--comment`
- `parsec inbox` for viewing assigned tickets
- `parsec init` and `parsec root` for shell integration
- Automatic ticket status transitions
- `parsec ship --base` flag and `default_base` config
- `gh auth token` fallback for GitHub token resolution
- Host-aware GitHub token resolution for GitHub Enterprise

### Fixed
- Merge/pr-status work with adopted branches
- Conflicts detection with origin fallback
- Prune stale remote-tracking references after merge

### Security
- Pinned GitHub Actions to SHA hashes
- Added cargo audit job

## [0.2.4] - 2026-04-15

### Added
- `parsec ci` to check CI/CD pipeline status
- `parsec merge` to merge PRs from terminal
- `parsec diff` to view worktree changes
- `parsec stack` for stacked PR dependency tracking
- `parsec start --branch` for existing branches

## [0.2.3] - 2026-04-15

### Added
- `parsec open` to open PR or ticket page in browser
- `parsec pr-status` for CI and review status
- `parsec sync` to rebase/merge base branch
- `parsec adopt` to import existing branches

## [0.2.0] - 2026-04-14

### Added
- Initial release with core commands (start, list, status, ship, clean, conflicts, switch, log, undo)
- Jira and GitHub Issues integration
- `--json` output on all commands
- Sibling and internal worktree layouts

[0.3.3]: https://github.com/erishforG/git-parsec/compare/v0.3.2...v0.3.3
[0.3.2]: https://github.com/erishforG/git-parsec/compare/v0.3.1...v0.3.2
[0.3.1]: https://github.com/erishforG/git-parsec/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/erishforG/git-parsec/compare/v0.2.4...v0.3.0
[0.2.4]: https://github.com/erishforG/git-parsec/compare/v0.2.3...v0.2.4
[0.2.3]: https://github.com/erishforG/git-parsec/compare/v0.2.0...v0.2.3
[0.2.0]: https://github.com/erishforG/git-parsec/releases/tag/v0.2.0
