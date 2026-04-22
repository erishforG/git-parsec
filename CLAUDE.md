# CLAUDE.md — git-parsec Project Instructions

## Project Overview

- **Name**: git-parsec (binary: `parsec`)
- **Language**: Rust (edition 2021)
- **Repo**: https://github.com/erishforG/git-parsec
- **Docs site**: https://erishforg.github.io/git-parsec/ (GitHub Pages from `docs/` on `main`)

## Branch Strategy

- **Main branch**: `main` — production, auto-releases on push
- **Dev branch**: `develop` — development work
- PRs go from `develop` → `main`
- Default branch prefix: `feature/`

## Release Process

### How Releases Work

Releases are **fully automated via CI** (`.github/workflows/release.yml`). The workflow triggers on every push to `main` and:

1. Reads version from `Cargo.toml`
2. Checks if a git tag for that version already exists
3. If new: creates git tag, publishes to crates.io, creates GitHub Release, builds binaries, and snapshots versioned docs

### NEVER Create Tags Manually

**Do NOT run `git tag` or create tags manually.** The CI workflow handles tag creation automatically. Manually creating tags will cause the release workflow to skip (it checks `if tag exists → skip`).

### How to Release a New Version

1. Bump `version` in `Cargo.toml`
2. Ensure all changes are on `develop`
3. Merge `develop` → `main` via PR
4. CI does the rest automatically

### Pre-Release Checklist (MANDATORY)

**CRITICAL: Do NOT merge to `main` without completing ALL items below. Skipping README or docs updates has caused issues in the past.**

Before merging to `main`, verify:

- [ ] `Cargo.toml` version bumped
- [ ] **`README.md` updated** — new commands, changed flags, feature descriptions, command count
- [ ] **`docs/` pages updated** — this is the public-facing site, must reflect current version
  - `docs/index.html` — feature list, command count, examples
  - `docs/guide/index.html` — installation, workflows, new features
  - `docs/reference/index.html` — all commands with correct options/examples
  - `softwareVersion` in structured data (`<script type="application/ld+json">`) matches new version
- [ ] `docs/sitemap.xml` `<lastmod>` dates updated if docs changed
- [ ] Integration tests pass (`cargo test`)
- [ ] `cargo build --release` succeeds

If a new command was added or an existing command changed, README and docs MUST be updated in the same PR. Do not defer documentation to a follow-up.

## Architecture

- Output: `dispatch_output!` macro for Human/Json/Quiet modes
- State: `.parsec/state.json` with file locking (atomic writes via temp+rename)
- Oplog: `.parsec/oplog.json` for undo support
- Config: `~/.config/parsec/config.toml` (TOML format)
- Env vars: centralized in `src/env.rs` with priority-based token resolution
- GitHub Enterprise: host-based API URL routing
- Per-repo tracker overrides: `[repos."owner/repo"]` config section
- Worktree layout: Sibling (default, `../repo.ticket/`) or Internal (`.parsec/workspaces/ticket/`)

## Versioned Documentation

The docs site supports versioned documentation:

- `docs/versions.json` — version manifest (latest version + version list)
- `docs/version-switcher.js` — shared dropdown logic
- `docs/v/{VERSION}/` — versioned snapshots (auto-created by CI on release)
- Versioned pages have `noindex` meta tag and canonical pointing to root
- `demo.gif` is referenced via absolute path (`/git-parsec/demo.gif`), not copied per version

## Testing

- Integration tests: `tests/cli_tests.rs`
- Run: `cargo test`
- Dev deps: assert_cmd, predicates, tempfile

## Code Conventions

- Error handling: `anyhow` for application errors, `thiserror` for library-style enums
- CLI: clap 4 with derive macros
- Async: tokio (full features)
- HTTP: reqwest with rustls-tls
- All commands support `--json` for machine-readable output
