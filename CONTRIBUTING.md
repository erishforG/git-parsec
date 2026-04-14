# Contributing to git-parsec

Thank you for your interest in contributing to git-parsec!

## Development Setup

```bash
git clone https://github.com/erishforG/git-parsec.git
cd git-parsec
cargo build
cargo test
```

## Branch Strategy

We follow **git flow**:

- `main` — stable releases
- `develop` — integration branch
- `feature/*` — new features (branch from `develop`)
- `fix/*` — bug fixes (branch from `develop`)

Both `main` and `develop` are protected. All changes must go through a Pull Request with at least 1 approval.

## Making Changes

1. Fork the repo and create your branch from `develop`:
   ```bash
   git checkout develop
   git checkout -b feature/your-feature
   ```

2. Write your code. Run tests:
   ```bash
   cargo test
   cargo fmt
   cargo clippy
   ```

3. Commit with a clear message:
   ```bash
   git commit -m "Add support for GitLab Issues tracker"
   ```

4. Push and open a PR targeting `develop`:
   ```bash
   git push origin feature/your-feature
   ```

## Code Style

- Run `cargo fmt` before committing
- Run `cargo clippy` and fix all warnings
- Keep functions focused and small
- Add tests for new functionality

## Adding a New Tracker

To add a new issue tracker (e.g., GitLab, Linear):

1. Create `src/tracker/your_tracker.rs`
2. Implement the same pattern as `jira.rs` or `github_issues.rs`:
   - A struct with `new()` and `async fn fetch_ticket(&self, id: &str) -> Result<Ticket>`
3. Add a variant to `TrackerProvider` in `src/config/settings.rs`
4. Wire it in `src/tracker/mod.rs` `fetch_ticket()` function
5. Add config options if needed

## Reporting Issues

- Use [GitHub Issues](https://github.com/erishforG/git-parsec/issues)
- Include your OS, Rust version, and steps to reproduce

## License

By contributing, you agree that your contributions will be licensed under the MIT License.
