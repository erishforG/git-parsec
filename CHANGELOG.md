# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.5.0] - 2026-06-03 — _The visualization release_

v0.5 마일스톤 **16/16 완료**. Polish & Power-User UX: 워크트리/PR/CI를 하나의
시야에 모아주는 시각화·자동화 명령 6개 신규.

### Added
- **`parsec smartlog` (alias `sl`)** — 모든 활성 워크트리를 commit DAG로 시각화.
  ASCII 트리가 base branch별로 워크트리를 묶고 merge-base 이후 커밋을 표시.
  Phase 2 PR/CI status overlay (`#327`), Phase 3 worktree filter + ANSI 색상 +
  stack indicator (`#333`). `--json` 출력 지원. (`#245`, `#305`, `#318`, `#319`)
- **`parsec dashboard` (alias `dash`)** — 실시간 터미널 TUI 대시보드. 워크트리 /
  CI 상태 / GitHub PR을 3-pane 레이아웃으로 한 화면에. ratatui + crossterm 기반,
  키바인딩 `q` (종료) / `r` (즉시 새로고침) / `?` (도움말), `--refresh N` 인터벌,
  `--no-overlay` 오프라인 모드. (`#248`, `#337`)
- **`parsec test`** — 워크트리 병렬 테스트 러너 + tree-hash 결과 캐싱.
  `--all`로 모든 활성 워크트리 일괄 실행, `--jobs N` 병렬, `--cache`로 동일 tree
  재실행 시 즉시 스킵. `[test]` 설정 섹션(`command`, `jobs`, `cache`). 인간/JSON
  출력. (`#247`, `#336`)
- **`parsec health`** — 모든 활성 워크트리 헬스 체크. Phase 1: lock(`.git/index.lock`)
  · uncommitted 파일 수 · stale(7일 초과) 검사 (`#324`, `#325`). Phase 2: CI 상태
  overlay + configurable stale threshold (`#330`). CLI 통합 테스트 5개 (`#326`).
- **`parsec reviews`** — 워크트리별 받은/요청한 PR 리뷰를 한 표로. Phase 1 (`#301`,
  `#331`) + Phase 2 `--requested` (GitHub Search API) (`#334`).
- **`parsec conflicts --simulate`** — 기존 filename overlap 휴리스틱을 보완하는
  line-level 충돌 시뮬레이션. `git merge-tree --write-tree`로 워크트리 vs base +
  워크트리 페어 cross-simulate 두 패스. 머지 전 실제 충돌 파일을 read-only로 노출.
  (`#246`, `#335`)
- **`parsec commit`** — AI 커밋 메시지 생성 (OpenAI / Anthropic). staged diff 분석
  후 자동 prefix + Conventional Commits 포맷(`--conventional`). 수동 메시지
  override(`--message`). (`#274`)
- **`parsec sync`** — auto-sync `main`/`develop` into stale worktrees (rebase 또는
  merge 전략, `--all` 일괄, `--dry-run` behind 카운트, conflict hint). (`#290`)
- **AI-generated PR descriptions** — `parsec ship`이 OpenAI / Anthropic / Ollama
  공급자로 PR 본문 자동 작성. `[ai]` 설정. (`#242`, `#275`)
- **`parsec __complete` shell-completion 헬퍼** — 숨김 subcommand가 워크트리 / branch
  완성 후보를 newline-separated로 출력. zsh / bash / fish 동적 탭 완성 지원
  (`#291`, `#312`). Phase 2 dynamic 쉘 스크립트 (`#328`).
- **`parsec agent` mode (PARSEC_AGENT=1)** — non-interactive JSON 출력 모드, AI
  에이전트 호출용. (`#272`)

### Changed
- **Error messages standardized to 3-line format** — 모든 사용자 대상 에러가
  `error: <summary> / caused by: <root cause> / help: <action>` 포맷으로 통일
  (`#303`, `#306`).

### Fixed
- `parsec ship` falls back to `gh auth token` when `PARSEC_GITHUB_TOKEN` /
  `GITHUB_TOKEN` / `GH_TOKEN` env vars are absent — parity with `parsec doctor`
  and the tracker layer. GitHub host에만 한정해 Bitbucket / GitLab remote는 영향
  없음 (`#281`).

### CI
- Windows VS2026 (Visual Studio 2026 runner) pre-validation 잡 — MSVC toolchain
  회귀 사전 차단 (`#307`, `#311`).
- `parsec test`의 shell invocation을 cross-platform화 (sh -c / cmd /C),
  Windows test가 WSL을 호출하지 않도록 수정.

### Docs
- 모듈별 RustDoc 보강 — `diff` / `history` (`#321`), `stack` / `ci` (`#323`).
- CHANGELOG `[Unreleased]` 섹션 누락 항목 보완 (smartlog / complete / errors /
  win-ci) (`#316`, `#317`).

### Tests
- CLI 통합 테스트 대폭 추가 — `compress` / `config schema` / `log --export`
  (`#314`, `#315`), `smartlog` / `sl` (`#318`, `#319`), `health` (`#324`, `#326`),
  `parsec test` (5 신규), `parsec dashboard` (4 신규), `conflicts --simulate`
  (4 신규).

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
