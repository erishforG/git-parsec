# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
