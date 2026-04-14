use assert_cmd::Command;
use predicates::prelude::*;
use std::process::Command as StdCommand;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Create a minimal git repo with one empty commit (no remote).
/// Suitable for commands that do NOT call `git fetch origin`.
fn setup_repo() -> TempDir {
    let dir = TempDir::new().unwrap();

    StdCommand::new("git")
        .args(["init"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    StdCommand::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    StdCommand::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    // Ensure the default branch is named "main".
    StdCommand::new("git")
        .args(["checkout", "-b", "main"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    StdCommand::new("git")
        .args(["commit", "--allow-empty", "-m", "init"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    dir
}

/// Create a working repo with a bare repo wired as `origin`.
/// The initial commit is pushed so `git fetch origin` and
/// `git worktree add -b <branch> <path> main` both succeed.
///
/// Returns `(working_dir, bare_dir)`. Both TempDirs must stay alive for the
/// duration of the test.
fn setup_repo_with_remote() -> (TempDir, TempDir) {
    // ---- bare "remote" ----
    let bare = TempDir::new().unwrap();
    StdCommand::new("git")
        .args(["init", "--bare"])
        .current_dir(bare.path())
        .output()
        .unwrap();

    // ---- working copy ----
    let dir = TempDir::new().unwrap();

    StdCommand::new("git")
        .args(["init"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    StdCommand::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    StdCommand::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    // Ensure the branch is "main" before the first commit.
    StdCommand::new("git")
        .args(["checkout", "-b", "main"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    StdCommand::new("git")
        .args([
            "remote",
            "add",
            "origin",
            bare.path().to_str().unwrap(),
        ])
        .current_dir(dir.path())
        .output()
        .unwrap();

    StdCommand::new("git")
        .args(["commit", "--allow-empty", "-m", "init"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    StdCommand::new("git")
        .args(["push", "-u", "origin", "main"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    (dir, bare)
}

fn parsec() -> Command {
    Command::cargo_bin("parsec").unwrap()
}

// ---------------------------------------------------------------------------
// Basic invocation
// ---------------------------------------------------------------------------

#[test]
fn test_help() {
    parsec()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("worktree"));
}

#[test]
fn test_version() {
    parsec()
        .arg("--version")
        .assert()
        .success();
}

// ---------------------------------------------------------------------------
// list
// ---------------------------------------------------------------------------

#[test]
fn test_list_empty() {
    let repo = setup_repo();
    parsec()
        .args(["list", "--repo", repo.path().to_str().unwrap()])
        .assert()
        .success();
}

#[test]
fn test_list_json_empty() {
    let repo = setup_repo();
    parsec()
        .args(["--json", "list", "--repo", repo.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("[]"));
}

// ---------------------------------------------------------------------------
// start
// ---------------------------------------------------------------------------

#[test]
fn test_start_creates_worktree() {
    let (repo, _bare) = setup_repo_with_remote();
    let repo_path = repo.path().to_str().unwrap();

    parsec()
        .args(["start", "TEST-001", "--repo", repo_path])
        .assert()
        .success();

    // .parsec/state.json must exist and contain the ticket.
    let state_path = repo.path().join(".parsec").join("state.json");
    assert!(state_path.exists(), ".parsec/state.json should exist after start");

    let contents = std::fs::read_to_string(&state_path).unwrap();
    assert!(
        contents.contains("TEST-001"),
        "state.json should reference TEST-001"
    );
}

#[test]
fn test_start_then_list() {
    let (repo, _bare) = setup_repo_with_remote();
    let repo_path = repo.path().to_str().unwrap();

    parsec()
        .args(["start", "TEST-002", "--repo", repo_path])
        .assert()
        .success();

    parsec()
        .args(["list", "--repo", repo_path])
        .assert()
        .success()
        .stdout(predicate::str::contains("TEST-002"));
}

#[test]
fn test_start_then_status() {
    let (repo, _bare) = setup_repo_with_remote();
    let repo_path = repo.path().to_str().unwrap();

    parsec()
        .args(["start", "TEST-003", "--repo", repo_path])
        .assert()
        .success();

    parsec()
        .args(["status", "TEST-003", "--repo", repo_path])
        .assert()
        .success()
        .stdout(predicate::str::contains("TEST-003"));
}

#[test]
fn test_start_then_switch() {
    let (repo, _bare) = setup_repo_with_remote();
    let repo_path = repo.path().to_str().unwrap();

    parsec()
        .args(["start", "TEST-004", "--repo", repo_path])
        .assert()
        .success();

    // switch should print the worktree path (which includes the ticket name).
    parsec()
        .args(["switch", "TEST-004", "--repo", repo_path])
        .assert()
        .success()
        .stdout(predicate::str::contains("TEST-004"));
}

#[test]
fn test_start_duplicate_fails() {
    let (repo, _bare) = setup_repo_with_remote();
    let repo_path = repo.path().to_str().unwrap();

    parsec()
        .args(["start", "TEST-DUP", "--repo", repo_path])
        .assert()
        .success();

    // Starting the same ticket a second time must fail.
    parsec()
        .args(["start", "TEST-DUP", "--repo", repo_path])
        .assert()
        .failure();
}

// ---------------------------------------------------------------------------
// conflicts
// ---------------------------------------------------------------------------

#[test]
fn test_conflicts_empty() {
    let repo = setup_repo();
    parsec()
        .args(["conflicts", "--repo", repo.path().to_str().unwrap()])
        .assert()
        .success();
}

// ---------------------------------------------------------------------------
// clean
// ---------------------------------------------------------------------------

#[test]
fn test_clean_empty() {
    let repo = setup_repo();
    parsec()
        .args(["clean", "--repo", repo.path().to_str().unwrap()])
        .assert()
        .success();
}

#[test]
fn test_clean_dry_run() {
    let repo = setup_repo();
    parsec()
        .args(["clean", "--dry-run", "--repo", repo.path().to_str().unwrap()])
        .assert()
        .success();
}

// ---------------------------------------------------------------------------
// config
// ---------------------------------------------------------------------------

#[test]
fn test_config_show_defaults() {
    // config show reads the user-level config and should always succeed.
    parsec()
        .arg("config")
        .arg("show")
        .assert()
        .success();
}

// ---------------------------------------------------------------------------
// error cases
// ---------------------------------------------------------------------------

#[test]
fn test_switch_nonexistent_fails() {
    let repo = setup_repo();
    parsec()
        .args(["switch", "NONEXIST", "--repo", repo.path().to_str().unwrap()])
        .assert()
        .failure();
}

#[test]
fn test_ship_nonexistent_fails() {
    let repo = setup_repo();
    parsec()
        .args(["ship", "NONEXIST", "--repo", repo.path().to_str().unwrap()])
        .assert()
        .failure();
}

// ---------------------------------------------------------------------------
// JSON output format
// ---------------------------------------------------------------------------

#[test]
fn test_list_json_format() {
    let (repo, _bare) = setup_repo_with_remote();
    let repo_path = repo.path().to_str().unwrap();

    parsec()
        .args(["start", "TEST-JSON", "--repo", repo_path])
        .assert()
        .success();

    let output = parsec()
        .args(["--json", "list", "--repo", repo_path])
        .output()
        .unwrap();

    assert!(output.status.success(), "parsec list --json should succeed");

    let stdout = String::from_utf8(output.stdout).unwrap();
    // Must be parseable as a JSON array.
    let parsed: serde_json::Value = serde_json::from_str(&stdout)
        .expect("parsec list --json must produce valid JSON");

    let arr = parsed.as_array().expect("output should be a JSON array");
    assert!(!arr.is_empty(), "array should contain at least one workspace");

    // Each element must have a "ticket" field.
    let first = &arr[0];
    assert!(
        first.get("ticket").is_some(),
        "workspace JSON should have a 'ticket' field"
    );
    assert_eq!(first["ticket"].as_str().unwrap(), "TEST-JSON");
}

#[test]
fn test_status_json_format() {
    let repo = setup_repo();
    let repo_path = repo.path().to_str().unwrap();

    let output = parsec()
        .args(["--json", "status", "--repo", repo_path])
        .output()
        .unwrap();

    assert!(output.status.success(), "parsec status --json should succeed");

    let stdout = String::from_utf8(output.stdout).unwrap();
    // Must be valid JSON (array of workspaces, possibly empty).
    let _: serde_json::Value = serde_json::from_str(&stdout)
        .expect("parsec status --json must produce valid JSON");
}
