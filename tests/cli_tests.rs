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
        .args(["remote", "add", "origin", bare.path().to_str().unwrap()])
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
    let mut cmd = Command::cargo_bin("parsec").unwrap();
    // Isolate tests from user's global config (e.g. default_base = "develop")
    cmd.env("PARSEC_CONFIG_DIR", "/tmp/parsec-test-nonexistent");
    cmd
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
    parsec().arg("--version").assert().success();
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
    assert!(
        state_path.exists(),
        ".parsec/state.json should exist after start"
    );

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
fn test_start_duplicate_is_idempotent() {
    let (repo, _bare) = setup_repo_with_remote();
    let repo_path = repo.path().to_str().unwrap();

    parsec()
        .args(["start", "TEST-DUP", "--repo", repo_path])
        .assert()
        .success();

    // Starting the same ticket a second time succeeds (idempotent — switches to existing).
    parsec()
        .args(["start", "TEST-DUP", "--repo", repo_path])
        .assert()
        .success();
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
        .args([
            "clean",
            "--dry-run",
            "--repo",
            repo.path().to_str().unwrap(),
        ])
        .assert()
        .success();
}

// ---------------------------------------------------------------------------
// config
// ---------------------------------------------------------------------------

#[test]
fn test_config_show_defaults() {
    // config show reads the user-level config and should always succeed.
    parsec().arg("config").arg("show").assert().success();
}

// ---------------------------------------------------------------------------
// error cases
// ---------------------------------------------------------------------------

#[test]
fn test_switch_nonexistent_fails() {
    let repo = setup_repo();
    parsec()
        .args([
            "switch",
            "NONEXIST",
            "--repo",
            repo.path().to_str().unwrap(),
        ])
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
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("parsec list --json must produce valid JSON");

    let arr = parsed.as_array().expect("output should be a JSON array");
    assert!(
        !arr.is_empty(),
        "array should contain at least one workspace"
    );

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

    assert!(
        output.status.success(),
        "parsec status --json should succeed"
    );

    let stdout = String::from_utf8(output.stdout).unwrap();
    // Must be valid JSON (array of workspaces, possibly empty).
    let _: serde_json::Value =
        serde_json::from_str(&stdout).expect("parsec status --json must produce valid JSON");
}

// ---------------------------------------------------------------------------
// sync
// ---------------------------------------------------------------------------

#[test]
fn test_sync_rebases_worktree() {
    let (repo, _bare) = setup_repo_with_remote();
    let repo_path = repo.path().to_str().unwrap();

    // Create a worktree for SYNC-001.
    parsec()
        .args(["start", "SYNC-001", "--repo", repo_path])
        .assert()
        .success();

    // Make a new commit on main so the worktree branch is behind origin/main.
    StdCommand::new("git")
        .args(["commit", "--allow-empty", "-m", "advance main"])
        .current_dir(repo.path())
        .output()
        .unwrap();
    StdCommand::new("git")
        .args(["push", "origin", "main"])
        .current_dir(repo.path())
        .output()
        .unwrap();

    // parsec sync should rebase without error.
    parsec()
        .args(["sync", "SYNC-001", "--repo", repo_path])
        .assert()
        .success();
}

#[test]
fn test_sync_skips_when_already_up_to_date() {
    let (repo, _bare) = setup_repo_with_remote();
    let repo_path = repo.path().to_str().unwrap();

    // Create a worktree — it starts up-to-date.
    parsec()
        .args(["start", "SYNC-002", "--repo", repo_path])
        .assert()
        .success();

    // With default --min-behind 1, a worktree that is 0 commits behind should
    // be skipped (no error, no sync output line).
    let out = parsec()
        .args(["sync", "SYNC-002", "--repo", repo_path])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Should not report a successful rebase of SYNC-002.
    assert!(!stdout.contains("rebase") || stdout.contains("Skipped") || stdout.contains("Nothing"));
}

#[test]
fn test_sync_dry_run_shows_behind_count() {
    let (repo, _bare) = setup_repo_with_remote();
    let repo_path = repo.path().to_str().unwrap();

    parsec()
        .args(["start", "SYNC-003", "--repo", repo_path])
        .assert()
        .success();

    // Advance main by one commit so SYNC-003 is 1 behind.
    StdCommand::new("git")
        .args([
            "commit",
            "--allow-empty",
            "-m",
            "advance main for dry-run test",
        ])
        .current_dir(repo.path())
        .output()
        .unwrap();
    StdCommand::new("git")
        .args(["push", "origin", "main"])
        .current_dir(repo.path())
        .output()
        .unwrap();

    // --dry-run should report the action without modifying the worktree.
    let out = parsec()
        .args(["sync", "SYNC-003", "--dry-run", "--repo", repo_path])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("dry-run"),
        "expected dry-run output, got: {stderr}"
    );
}

// ---------------------------------------------------------------------------
// adopt
// ---------------------------------------------------------------------------

#[test]
fn test_adopt_imports_existing_branch() {
    let (repo, _bare) = setup_repo_with_remote();
    let repo_path = repo.path().to_str().unwrap();

    // Create a branch manually in the main repo.
    StdCommand::new("git")
        .args(["branch", "feature/ADOPT-001"])
        .current_dir(repo.path())
        .output()
        .unwrap();

    // parsec adopt should import it.
    parsec()
        .args([
            "adopt",
            "ADOPT-001",
            "--branch",
            "feature/ADOPT-001",
            "--repo",
            repo_path,
        ])
        .assert()
        .success();

    // The ticket should now appear in the list.
    parsec()
        .args(["list", "--repo", repo_path])
        .assert()
        .success()
        .stdout(predicate::str::contains("ADOPT-001"));
}

// ---------------------------------------------------------------------------
// undo
// ---------------------------------------------------------------------------

#[test]
fn test_undo_removes_worktree() {
    let (repo, _bare) = setup_repo_with_remote();
    let repo_path = repo.path().to_str().unwrap();

    // Start a worktree so there is something to undo.
    parsec()
        .args(["start", "UNDO-001", "--repo", repo_path])
        .assert()
        .success();

    // Worktree should be listed before undo.
    parsec()
        .args(["list", "--repo", repo_path])
        .assert()
        .success()
        .stdout(predicate::str::contains("UNDO-001"));

    // Undo the start operation.
    parsec()
        .args(["undo", "--repo", repo_path])
        .assert()
        .success();

    // The workspace should no longer appear in the list.
    parsec()
        .args(["list", "--repo", repo_path])
        .assert()
        .success()
        .stdout(predicate::str::contains("UNDO-001").not());
}

// ---------------------------------------------------------------------------
// diff
// ---------------------------------------------------------------------------

#[test]
fn test_diff_name_only_shows_changed_file() {
    let (repo, _bare) = setup_repo_with_remote();
    let repo_path = repo.path().to_str().unwrap();

    // Start a worktree for DIFF-001.
    parsec()
        .args(["start", "DIFF-001", "--repo", repo_path])
        .assert()
        .success();

    // Locate the worktree path by reading state.json.
    let state_path = repo.path().join(".parsec").join("state.json");
    let state_contents = std::fs::read_to_string(&state_path).unwrap();
    let state: serde_json::Value = serde_json::from_str(&state_contents).unwrap();

    // Extract the path from the state JSON – workspaces is an object keyed by ticket.
    let wt_path = state["workspaces"]["DIFF-001"]["path"]
        .as_str()
        .expect("state.json should contain path for DIFF-001");

    // Create and commit a file inside the worktree.
    std::fs::write(format!("{}/changed.txt", wt_path), "hello").unwrap();
    StdCommand::new("git")
        .args(["add", "changed.txt"])
        .current_dir(wt_path)
        .output()
        .unwrap();
    StdCommand::new("git")
        .args(["commit", "-m", "add changed.txt"])
        .current_dir(wt_path)
        .output()
        .unwrap();

    // parsec diff --name-only should list changed.txt.
    parsec()
        .args(["diff", "DIFF-001", "--name-only", "--repo", repo_path])
        .assert()
        .success()
        .stdout(predicate::str::contains("changed.txt"));
}

// ---------------------------------------------------------------------------
// log
// ---------------------------------------------------------------------------

#[test]
fn test_log_shows_operations() {
    let (repo, _bare) = setup_repo_with_remote();
    let repo_path = repo.path().to_str().unwrap();

    parsec()
        .args(["start", "LOG-001", "--repo", repo_path])
        .assert()
        .success();

    // parsec log should list the start operation for LOG-001.
    parsec()
        .args(["log", "--repo", repo_path])
        .assert()
        .success()
        .stdout(predicate::str::contains("LOG-001"));
}

#[test]
fn test_log_json_format() {
    let (repo, _bare) = setup_repo_with_remote();
    let repo_path = repo.path().to_str().unwrap();

    parsec()
        .args(["start", "LOG-JSON", "--repo", repo_path])
        .assert()
        .success();

    let output = parsec()
        .args(["--json", "log", "--repo", repo_path])
        .output()
        .unwrap();

    assert!(output.status.success(), "parsec --json log should succeed");

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("parsec --json log must produce valid JSON");

    let arr = parsed.as_array().expect("log JSON should be an array");
    assert!(!arr.is_empty(), "log array should have at least one entry");

    // Each entry must have 'op' and 'ticket' fields.
    let first = &arr[0];
    assert!(
        first.get("op").is_some(),
        "log entry should have 'op' field"
    );
    assert!(
        first.get("ticket").is_some(),
        "log entry should have 'ticket' field"
    );
    assert_eq!(first["ticket"].as_str().unwrap(), "LOG-JSON");
    assert_eq!(first["op"].as_str().unwrap(), "start");
}

// ---------------------------------------------------------------------------
// ship --no-pr
// ---------------------------------------------------------------------------

#[test]
fn test_ship_no_pr_pushes_branch() {
    let (repo, _bare) = setup_repo_with_remote();
    let repo_path = repo.path().to_str().unwrap();

    parsec()
        .args(["start", "SHIP-NP", "--repo", repo_path])
        .assert()
        .success();

    // Locate the worktree path.
    let state_path = repo.path().join(".parsec").join("state.json");
    let state_contents = std::fs::read_to_string(&state_path).unwrap();
    let state: serde_json::Value = serde_json::from_str(&state_contents).unwrap();
    let wt_path = state["workspaces"]["SHIP-NP"]["path"]
        .as_str()
        .expect("state.json should contain path for SHIP-NP")
        .to_owned();

    // Make a commit in the worktree so git push has something to send.
    std::fs::write(format!("{}/ship.txt", wt_path), "ship").unwrap();
    StdCommand::new("git")
        .args(["add", "ship.txt"])
        .current_dir(&wt_path)
        .output()
        .unwrap();
    StdCommand::new("git")
        .args(["commit", "-m", "ship commit"])
        .current_dir(&wt_path)
        .output()
        .unwrap();

    // parsec ship --no-pr should push without attempting PR creation.
    parsec()
        .args(["ship", "SHIP-NP", "--no-pr", "--repo", repo_path])
        .assert()
        .success();
}

// ---------------------------------------------------------------------------
// clean specific ticket
// ---------------------------------------------------------------------------

#[test]
fn test_clean_specific_ticket_leaves_other() {
    let (repo, _bare) = setup_repo_with_remote();
    let repo_path = repo.path().to_str().unwrap();

    // Start two worktrees.
    parsec()
        .args(["start", "CLEAN-A", "--repo", repo_path])
        .assert()
        .success();

    parsec()
        .args(["start", "CLEAN-B", "--repo", repo_path])
        .assert()
        .success();

    // Clean only CLEAN-A.
    parsec()
        .args(["clean", "CLEAN-A", "--repo", repo_path])
        .assert()
        .success();

    // CLEAN-A should be gone; CLEAN-B should remain.
    parsec()
        .args(["list", "--repo", repo_path])
        .assert()
        .success()
        .stdout(predicate::str::contains("CLEAN-A").not())
        .stdout(predicate::str::contains("CLEAN-B"));
}

// ---------------------------------------------------------------------------
// root
// ---------------------------------------------------------------------------

#[test]
fn test_root_prints_repo_path() {
    let repo = setup_repo();
    let repo_path = repo.path().to_str().unwrap();

    // parsec root should print a path that is a prefix of the repo path
    // (may be the canonical/realpath form).
    parsec()
        .args(["root", "--repo", repo_path])
        .assert()
        .success()
        .stdout(predicate::str::is_empty().not());
}

// ---------------------------------------------------------------------------
// quiet mode
// ---------------------------------------------------------------------------

#[test]
fn test_quiet_mode_suppresses_output() {
    let (repo, _bare) = setup_repo_with_remote();
    let repo_path = repo.path().to_str().unwrap();

    parsec()
        .args(["start", "QUIET-001", "--repo", repo_path])
        .assert()
        .success();

    // --quiet list should produce no stdout output (empty or whitespace-only).
    let output = parsec()
        .args(["--quiet", "list", "--repo", repo_path])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(
        String::from_utf8(output.stdout).unwrap().trim().is_empty(),
        "quiet mode should suppress normal output"
    );
}

// ---------------------------------------------------------------------------
// start --title
// ---------------------------------------------------------------------------

#[test]
fn test_start_with_title() {
    let (repo, _bare) = setup_repo_with_remote();
    let repo_path = repo.path().to_str().unwrap();

    parsec()
        .args([
            "start",
            "TITLE-001",
            "--title",
            "My Custom Title",
            "--repo",
            repo_path,
        ])
        .assert()
        .success();

    // The title should be stored in state.json.
    let state_path = repo.path().join(".parsec").join("state.json");
    let contents = std::fs::read_to_string(&state_path).unwrap();
    assert!(
        contents.contains("My Custom Title"),
        "state.json should store the custom title"
    );
}

// ---------------------------------------------------------------------------
// start --base (custom base branch)
// ---------------------------------------------------------------------------

#[test]
fn test_start_with_base_branch() {
    let (repo, _bare) = setup_repo_with_remote();
    let repo_path = repo.path().to_str().unwrap();

    // Create and push a "develop" branch.
    StdCommand::new("git")
        .args(["checkout", "-b", "develop"])
        .current_dir(repo.path())
        .output()
        .unwrap();
    StdCommand::new("git")
        .args(["commit", "--allow-empty", "-m", "develop init"])
        .current_dir(repo.path())
        .output()
        .unwrap();
    StdCommand::new("git")
        .args(["push", "origin", "develop"])
        .current_dir(repo.path())
        .output()
        .unwrap();
    StdCommand::new("git")
        .args(["checkout", "main"])
        .current_dir(repo.path())
        .output()
        .unwrap();

    parsec()
        .args([
            "start", "BASE-001", "--base", "develop", "--repo", repo_path,
        ])
        .assert()
        .success();

    // Verify the worktree was created with develop as base.
    let state_path = repo.path().join(".parsec").join("state.json");
    let contents = std::fs::read_to_string(&state_path).unwrap();
    let state: serde_json::Value = serde_json::from_str(&contents).unwrap();
    assert_eq!(
        state["workspaces"]["BASE-001"]["base_branch"]
            .as_str()
            .unwrap(),
        "develop"
    );
}

// ---------------------------------------------------------------------------
// start --on (stacked worktrees)
// ---------------------------------------------------------------------------

#[test]
fn test_start_stacked_on_parent() {
    let (repo, _bare) = setup_repo_with_remote();
    let repo_path = repo.path().to_str().unwrap();

    // Start a parent worktree.
    parsec()
        .args(["start", "STACK-PARENT", "--repo", repo_path])
        .assert()
        .success();

    // Start a child stacked on the parent.
    parsec()
        .args([
            "start",
            "STACK-CHILD",
            "--on",
            "STACK-PARENT",
            "--repo",
            repo_path,
        ])
        .assert()
        .success();

    // Verify parent_ticket is set in state.json.
    let state_path = repo.path().join(".parsec").join("state.json");
    let contents = std::fs::read_to_string(&state_path).unwrap();
    let state: serde_json::Value = serde_json::from_str(&contents).unwrap();
    assert_eq!(
        state["workspaces"]["STACK-CHILD"]["parent_ticket"]
            .as_str()
            .unwrap(),
        "STACK-PARENT"
    );
}

// ---------------------------------------------------------------------------
// ship --dry-run
// ---------------------------------------------------------------------------

#[test]
fn test_ship_dry_run() {
    let (repo, _bare) = setup_repo_with_remote();
    let repo_path = repo.path().to_str().unwrap();

    parsec()
        .args(["start", "DRY-SHIP", "--repo", repo_path])
        .assert()
        .success();

    // --dry-run should succeed without actually shipping.
    parsec()
        .args(["--dry-run", "ship", "DRY-SHIP", "--repo", repo_path])
        .assert()
        .success();

    // The worktree should still be listed (not cleaned up).
    parsec()
        .args(["list", "--repo", repo_path])
        .assert()
        .success()
        .stdout(predicate::str::contains("DRY-SHIP"));
}

// ---------------------------------------------------------------------------
// doctor
// ---------------------------------------------------------------------------

#[test]
fn test_doctor_succeeds() {
    let repo = setup_repo();
    parsec()
        .args(["doctor", "--repo", repo.path().to_str().unwrap()])
        .assert()
        .success();
}

// ---------------------------------------------------------------------------
// log --ticket filter
// ---------------------------------------------------------------------------

#[test]
fn test_log_filter_by_ticket() {
    let (repo, _bare) = setup_repo_with_remote();
    let repo_path = repo.path().to_str().unwrap();

    parsec()
        .args(["start", "LOGF-A", "--repo", repo_path])
        .assert()
        .success();

    parsec()
        .args(["start", "LOGF-B", "--repo", repo_path])
        .assert()
        .success();

    // Filter log to LOGF-A only (ticket is a positional arg).
    let output = parsec()
        .args(["log", "LOGF-A", "--repo", repo_path])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("LOGF-A"), "filtered log should show LOGF-A");
    assert!(
        !stdout.contains("LOGF-B"),
        "filtered log should not show LOGF-B"
    );
}

// ---------------------------------------------------------------------------
// clean --orphans
// ---------------------------------------------------------------------------

#[test]
fn test_clean_orphans() {
    let (repo, _bare) = setup_repo_with_remote();
    let repo_path = repo.path().to_str().unwrap();

    parsec()
        .args(["start", "ORPHAN-001", "--repo", repo_path])
        .assert()
        .success();

    // Manually delete the worktree directory to create an orphan.
    let state_path = repo.path().join(".parsec").join("state.json");
    let state_contents = std::fs::read_to_string(&state_path).unwrap();
    let state: serde_json::Value = serde_json::from_str(&state_contents).unwrap();
    let wt_path = state["workspaces"]["ORPHAN-001"]["path"].as_str().unwrap();

    // Remove the worktree directory and prune git worktree list.
    std::fs::remove_dir_all(wt_path).unwrap();
    StdCommand::new("git")
        .args(["worktree", "prune"])
        .current_dir(repo.path())
        .output()
        .unwrap();

    // clean --orphans should remove the stale entry.
    parsec()
        .args(["clean", "--orphans", "--repo", repo_path])
        .assert()
        .success();

    // The orphaned workspace should be gone.
    parsec()
        .args(["list", "--repo", repo_path])
        .assert()
        .success()
        .stdout(predicate::str::contains("ORPHAN-001").not());
}

// ---------------------------------------------------------------------------
// rename
// ---------------------------------------------------------------------------

#[test]
fn test_rename_ticket() {
    let (repo, _bare) = setup_repo_with_remote();
    let repo_path = repo.path().to_str().unwrap();

    parsec()
        .args(["start", "OLD-NAME", "--repo", repo_path])
        .assert()
        .success();

    parsec()
        .args(["rename", "OLD-NAME", "NEW-NAME", "--repo", repo_path])
        .assert()
        .success();

    // OLD-NAME gone, NEW-NAME present.
    parsec()
        .args(["list", "--repo", repo_path])
        .assert()
        .success()
        .stdout(predicate::str::contains("OLD-NAME").not())
        .stdout(predicate::str::contains("NEW-NAME"));
}

// ---------------------------------------------------------------------------
// start --branch (existing branch)
// ---------------------------------------------------------------------------

#[test]
fn test_start_with_existing_branch() {
    let (repo, _bare) = setup_repo_with_remote();
    let repo_path = repo.path().to_str().unwrap();

    // Create an existing branch.
    StdCommand::new("git")
        .args(["branch", "my-existing-branch"])
        .current_dir(repo.path())
        .output()
        .unwrap();

    parsec()
        .args([
            "start",
            "EXIST-001",
            "--branch",
            "my-existing-branch",
            "--repo",
            repo_path,
        ])
        .assert()
        .success();

    // Should be listed with the correct branch.
    let state_path = repo.path().join(".parsec").join("state.json");
    let contents = std::fs::read_to_string(&state_path).unwrap();
    assert!(contents.contains("my-existing-branch"));
}

// ---------------------------------------------------------------------------
// shared_cache (issue #207)
// ---------------------------------------------------------------------------

/// Build a custom config dir containing a config.toml with the given body and
/// return its path. Caller must keep the TempDir alive.
fn write_config_toml(body: &str) -> TempDir {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("config.toml"), body).unwrap();
    dir
}

#[test]
fn test_shared_cache_symlink_creates_link() {
    let (repo, _bare) = setup_repo_with_remote();
    let repo_path = repo.path();

    // Pre-populate a `target/` directory in the main repo with a build artifact.
    std::fs::create_dir_all(repo_path.join("target")).unwrap();
    std::fs::write(repo_path.join("target").join("artifact.txt"), "pre-built").unwrap();

    let config_dir = write_config_toml(
        r#"
[worktree]
shared_cache = ["target"]
cache_strategy = "symlink"
"#,
    );

    let mut cmd = Command::cargo_bin("parsec").unwrap();
    cmd.env("PARSEC_CONFIG_DIR", config_dir.path())
        .args(["start", "CACHE-1", "--repo", repo_path.to_str().unwrap()])
        .assert()
        .success();

    // Worktree path follows sibling layout: <parent>/<repo_name>.CACHE-1
    let repo_name = repo_path.file_name().unwrap().to_string_lossy().to_string();
    let wt_path = repo_path
        .parent()
        .unwrap()
        .join(format!("{}.CACHE-1", repo_name));
    let dest = wt_path.join("target");

    assert!(dest.exists(), "worktree should have shared target/");
    let meta = std::fs::symlink_metadata(&dest).unwrap();
    assert!(
        meta.file_type().is_symlink(),
        "symlink strategy must produce a symlink, got: {:?}",
        meta.file_type()
    );
    let contents = std::fs::read_to_string(dest.join("artifact.txt")).unwrap();
    assert_eq!(contents, "pre-built");
}

#[test]
fn test_shared_cache_copy_creates_real_dir() {
    let (repo, _bare) = setup_repo_with_remote();
    let repo_path = repo.path();

    std::fs::create_dir_all(repo_path.join("target").join("nested")).unwrap();
    std::fs::write(repo_path.join("target").join("a.txt"), "alpha").unwrap();
    std::fs::write(
        repo_path.join("target").join("nested").join("b.txt"),
        "beta",
    )
    .unwrap();

    let config_dir = write_config_toml(
        r#"
[worktree]
shared_cache = ["target"]
cache_strategy = "copy"
"#,
    );

    let mut cmd = Command::cargo_bin("parsec").unwrap();
    cmd.env("PARSEC_CONFIG_DIR", config_dir.path())
        .args(["start", "CACHE-2", "--repo", repo_path.to_str().unwrap()])
        .assert()
        .success();

    let repo_name = repo_path.file_name().unwrap().to_string_lossy().to_string();
    let wt_path = repo_path
        .parent()
        .unwrap()
        .join(format!("{}.CACHE-2", repo_name));
    let dest = wt_path.join("target");

    assert!(dest.exists());
    let meta = std::fs::symlink_metadata(&dest).unwrap();
    assert!(
        !meta.file_type().is_symlink(),
        "copy strategy must NOT produce a symlink"
    );
    assert!(meta.is_dir());
    assert_eq!(
        std::fs::read_to_string(dest.join("a.txt")).unwrap(),
        "alpha"
    );
    assert_eq!(
        std::fs::read_to_string(dest.join("nested").join("b.txt")).unwrap(),
        "beta"
    );
}

#[test]
fn test_shared_cache_missing_entry_skipped() {
    let (repo, _bare) = setup_repo_with_remote();
    let repo_path = repo.path();

    // Don't pre-create `.venv` in the main repo.
    let config_dir = write_config_toml(
        r#"
[worktree]
shared_cache = [".venv"]
cache_strategy = "symlink"
"#,
    );

    let mut cmd = Command::cargo_bin("parsec").unwrap();
    cmd.env("PARSEC_CONFIG_DIR", config_dir.path())
        .args(["start", "CACHE-3", "--repo", repo_path.to_str().unwrap()])
        .assert()
        .success();

    let repo_name = repo_path.file_name().unwrap().to_string_lossy().to_string();
    let wt_path = repo_path
        .parent()
        .unwrap()
        .join(format!("{}.CACHE-3", repo_name));

    // Worktree was created (start succeeded), but `.venv` was simply skipped.
    assert!(wt_path.exists(), "worktree should still be created");
    assert!(
        !wt_path.join(".venv").exists(),
        "missing source should NOT be linked into worktree"
    );
}

// ---------------------------------------------------------------------------
// __complete (hidden dynamic-completion helper, #291)
// ---------------------------------------------------------------------------

#[test]
fn test_complete_is_hidden_from_help() {
    let assertion = parsec().arg("--help").assert().success();
    let stdout = String::from_utf8(assertion.get_output().stdout.clone()).unwrap();
    assert!(
        !stdout.contains("__complete"),
        "__complete should be hidden from --help, got:\n{stdout}"
    );
}

#[test]
fn test_complete_branches_lists_local_branches() {
    let repo = setup_repo();
    let repo_path = repo.path().to_str().unwrap();

    // Add a second local branch on top of the default main.
    StdCommand::new("git")
        .args(["branch", "feature/x"])
        .current_dir(repo.path())
        .output()
        .unwrap();

    let assertion = parsec()
        .args(["__complete", "branches", "--repo", repo_path])
        .assert()
        .success();
    let stdout = String::from_utf8(assertion.get_output().stdout.clone()).unwrap();

    assert!(stdout.contains("main"), "expected 'main', got:\n{stdout}");
    assert!(
        stdout.contains("feature/x"),
        "expected 'feature/x', got:\n{stdout}"
    );
}

#[test]
fn test_complete_worktrees_lists_tickets() {
    let (repo, _bare) = setup_repo_with_remote();
    let repo_path = repo.path().to_str().unwrap();

    parsec()
        .args(["start", "COMP-001", "--repo", repo_path])
        .assert()
        .success();

    let assertion = parsec()
        .args(["__complete", "worktrees", "--repo", repo_path])
        .assert()
        .success();
    let stdout = String::from_utf8(assertion.get_output().stdout.clone()).unwrap();

    assert!(
        stdout.contains("COMP-001"),
        "expected 'COMP-001', got:\n{stdout}"
    );
}

#[test]
fn test_complete_outside_git_repo_is_silent_success() {
    // Empty temp dir, no git repo — completion must not error or print noise.
    let dir = TempDir::new().unwrap();
    let assertion = parsec()
        .args([
            "__complete",
            "branches",
            "--repo",
            dir.path().to_str().unwrap(),
        ])
        .assert()
        .success();
    let stdout = String::from_utf8(assertion.get_output().stdout.clone()).unwrap();
    assert!(
        stdout.trim().is_empty(),
        "expected empty stdout outside repo, got:\n{stdout}"
    );
}

// ---------------------------------------------------------------------------
// JSON error format
// ---------------------------------------------------------------------------

#[test]
fn test_json_error_format() {
    let repo = setup_repo();
    let output = parsec()
        .args([
            "--json",
            "ship",
            "NONEXIST",
            "--repo",
            repo.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("JSON error output must be parseable");
    assert!(parsed["error"].as_bool().unwrap());
    assert!(parsed.get("code").is_some());
    assert!(parsed.get("message").is_some());
}

// ---------------------------------------------------------------------------
// compress command (issue #314)
// ---------------------------------------------------------------------------

/// When the worktree has only the initial "start" commit (0 commits above
/// merge-base), compress must report "Nothing to compress" and exit 0.
#[test]
fn test_compress_nothing_to_do() {
    let (repo, _bare) = setup_repo_with_remote();
    let repo_path = repo.path().to_str().unwrap();

    // Create a workspace
    parsec()
        .args(["start", "COMP-1", "--repo", repo_path])
        .assert()
        .success();

    // compress: single commit (merge-base == HEAD), nothing to squash
    parsec()
        .args(["compress", "COMP-1", "--repo", repo_path])
        .assert()
        .success()
        .stdout(predicate::str::contains("Nothing to compress"));
}

/// When the worktree has 2+ commits above merge-base, compress squashes them
/// into one and reports the count.
#[test]
fn test_compress_squashes_commits() {
    let (repo, _bare) = setup_repo_with_remote();
    let repo_path = repo.path();

    // Start a workspace
    parsec()
        .args(["start", "COMP-2", "--repo", repo_path.to_str().unwrap()])
        .assert()
        .success();

    // Locate the sibling worktree directory
    let repo_name = repo_path.file_name().unwrap().to_string_lossy().to_string();
    let wt_path = repo_path
        .parent()
        .unwrap()
        .join(format!("{}.COMP-2", repo_name));

    // Make two distinct commits in the worktree
    std::fs::write(wt_path.join("a.txt"), "alpha").unwrap();
    StdCommand::new("git")
        .args(["add", "a.txt"])
        .current_dir(&wt_path)
        .output()
        .unwrap();
    StdCommand::new("git")
        .args(["commit", "-m", "first change"])
        .current_dir(&wt_path)
        .output()
        .unwrap();

    std::fs::write(wt_path.join("b.txt"), "beta").unwrap();
    StdCommand::new("git")
        .args(["add", "b.txt"])
        .current_dir(&wt_path)
        .output()
        .unwrap();
    StdCommand::new("git")
        .args(["commit", "-m", "second change"])
        .current_dir(&wt_path)
        .output()
        .unwrap();

    // compress should squash both commits and report "Compressed 2 commits"
    parsec()
        .args(["compress", "COMP-2", "--repo", repo_path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Compressed 2 commits"));

    // Verify only 1 commit now sits above merge-base
    let merge_base = StdCommand::new("git")
        .args(["merge-base", "HEAD", "main"])
        .current_dir(&wt_path)
        .output()
        .unwrap();
    let merge_base_sha = String::from_utf8(merge_base.stdout)
        .unwrap()
        .trim()
        .to_string();

    let count_out = StdCommand::new("git")
        .args(["rev-list", "--count", &format!("{}..HEAD", merge_base_sha)])
        .current_dir(&wt_path)
        .output()
        .unwrap();
    let count: u64 = String::from_utf8(count_out.stdout)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert_eq!(
        count, 1,
        "compress should leave exactly 1 commit above merge-base"
    );
}

// ---------------------------------------------------------------------------
// config schema command (issue #314)
// ---------------------------------------------------------------------------

/// `parsec config schema` must exit 0 and emit well-formed JSON.
#[test]
fn test_config_schema_outputs_json() {
    let repo = setup_repo();

    let output = parsec()
        .args(["config", "schema", "--repo", repo.path().to_str().unwrap()])
        .output()
        .unwrap();

    assert!(output.status.success(), "config schema should exit 0");

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("config schema output must be valid JSON");

    // JSON Schema documents must have a $schema or type/properties field
    assert!(
        parsed.get("$schema").is_some()
            || parsed.get("type").is_some()
            || parsed.get("properties").is_some(),
        "output does not look like a JSON Schema document"
    );
}

// ---------------------------------------------------------------------------
// history log --export command (issue #314)
// ---------------------------------------------------------------------------

/// `parsec log --export` in a repo with no prior parsec operations should exit
/// 0. When the execlog is empty it writes a message to stderr and nothing to
/// stdout (or exits successfully with empty stdout).
#[test]
fn test_history_log_export_empty() {
    let repo = setup_repo();

    let output = parsec()
        .args(["log", "--export", "--repo", repo.path().to_str().unwrap()])
        .output()
        .unwrap();

    // Should not fail
    assert!(
        output.status.success(),
        "log --export should succeed even when log is empty"
    );

    // Either stdout is empty OR stderr mentions the empty state
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stdout.is_empty() || stderr.contains("No execution log"),
        "expected empty stdout or informational stderr, got stdout={:?} stderr={:?}",
        stdout,
        stderr
    );
}

// ---------------------------------------------------------------------------
// parsec smartlog / sl (issue #245, #305)
// ---------------------------------------------------------------------------

/// `parsec smartlog` in a repo with no active worktrees should exit 0 and
/// print the "No active worktrees" placeholder message.
#[test]
fn test_smartlog_empty_repo() {
    let repo = setup_repo();

    parsec()
        .args(["smartlog", "--repo", repo.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("No active worktrees"));
}

/// `parsec sl` (alias) must behave identically to `parsec smartlog`.
#[test]
fn test_sl_alias_works_like_smartlog() {
    let repo = setup_repo();

    let smartlog_out = parsec()
        .args(["smartlog", "--repo", repo.path().to_str().unwrap()])
        .output()
        .unwrap();
    let sl_out = parsec()
        .args(["sl", "--repo", repo.path().to_str().unwrap()])
        .output()
        .unwrap();

    assert!(smartlog_out.status.success(), "smartlog should succeed");
    assert!(sl_out.status.success(), "sl alias should succeed");
    assert_eq!(
        smartlog_out.stdout, sl_out.stdout,
        "`sl` and `smartlog` must produce identical output"
    );
}

/// `parsec smartlog --json` in an empty repo must return a valid, empty JSON
/// array and exit 0.
#[test]
fn test_smartlog_json_empty_is_array() {
    let repo = setup_repo();

    let output = parsec()
        .args([
            "smartlog",
            "--json",
            "--repo",
            repo.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(output.status.success(), "smartlog --json should exit 0");

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("smartlog --json must emit valid JSON");
    assert!(
        parsed.is_array(),
        "smartlog --json should emit a JSON array, got: {parsed}"
    );
    assert_eq!(
        parsed.as_array().unwrap().len(),
        0,
        "empty repo → empty array"
    );
}

/// After creating a workspace, `parsec smartlog` should display the ticket
/// name, branch, and base branch in the ASCII tree.
#[test]
fn test_smartlog_shows_worktree() {
    let (repo, _bare) = setup_repo_with_remote();
    let repo_path = repo.path().to_str().unwrap();

    parsec()
        .args(["start", "SL-1", "--repo", repo_path])
        .assert()
        .success();

    let output = parsec()
        .args(["smartlog", "--repo", repo_path])
        .assert()
        .success();

    let stdout = String::from_utf8(output.get_output().stdout.clone()).unwrap();
    assert!(
        stdout.contains("SL-1"),
        "smartlog should show ticket 'SL-1', got:\n{stdout}"
    );
    // The ASCII tree marks base branch with "○ <base> (base)"
    assert!(
        stdout.contains("(base)"),
        "smartlog should show a base-branch marker, got:\n{stdout}"
    );
}

/// `parsec smartlog --json` with one active worktree must return a JSON array
/// containing exactly one object with expected fields.
#[test]
fn test_smartlog_json_one_worktree() {
    let (repo, _bare) = setup_repo_with_remote();
    let repo_path = repo.path().to_str().unwrap();

    parsec()
        .args(["start", "SL-2", "--repo", repo_path])
        .assert()
        .success();

    let output = parsec()
        .args(["smartlog", "--json", "--repo", repo_path])
        .output()
        .unwrap();

    assert!(output.status.success(), "smartlog --json should exit 0");

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("must be valid JSON");

    let arr = parsed.as_array().expect("must be a JSON array");
    assert_eq!(arr.len(), 1, "expected exactly 1 worktree entry");

    let entry = &arr[0];
    assert_eq!(
        entry["ticket"].as_str().unwrap(),
        "SL-2",
        "ticket field mismatch"
    );
    assert!(
        entry.get("branch").is_some(),
        "entry must have a 'branch' field"
    );
    assert!(
        entry.get("base_branch").is_some(),
        "entry must have a 'base_branch' field"
    );
    assert!(
        entry.get("commits").is_some(),
        "entry must have a 'commits' field"
    );
    // PR / CI overlay fields must NOT appear when unset (skip_serializing_if)
    assert!(
        entry.get("pr").is_none(),
        "unset 'pr' field must be omitted from JSON"
    );
    assert!(
        entry.get("ci").is_none(),
        "unset 'ci' field must be omitted from JSON"
    );
}

// ---------------------------------------------------------------------------
// parsec health (#324, Phase 1)
// ---------------------------------------------------------------------------

/// `parsec health` on a repo with no active worktrees must exit 0 and print
/// "No active worktrees."
#[test]
fn test_health_empty_repo() {
    let repo = setup_repo();
    let repo_path = repo.path().to_str().unwrap();

    parsec()
        .args(["health", "--repo", repo_path])
        .assert()
        .success()
        .stdout(predicate::str::contains("No active worktrees."));
}

/// `parsec health --json` on a repo with no active worktrees must exit 0 and
/// emit exactly the JSON array `[]` (Health.rs emits `[]` for empty set).
#[test]
fn test_health_empty_repo_json() {
    let repo = setup_repo();
    let repo_path = repo.path().to_str().unwrap();

    let output = parsec()
        .args(["health", "--json", "--repo", repo_path])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "health --json should exit 0 on empty repo"
    );

    let stdout = String::from_utf8(output.stdout).unwrap();
    let trimmed = stdout.trim();
    assert_eq!(trimmed, "[]", "empty repo → health --json must emit `[]`");
}

/// After creating a workspace, `parsec health` must exit 0 and display the
/// ticket name in the output.
#[test]
fn test_health_shows_worktree() {
    let (repo, _bare) = setup_repo_with_remote();
    let repo_path = repo.path().to_str().unwrap();

    parsec()
        .args(["start", "HL-1", "--repo", repo_path])
        .assert()
        .success();

    parsec()
        .args(["health", "--repo", repo_path])
        .assert()
        .success()
        .stdout(predicate::str::contains("HL-1"));
}

/// `parsec health --json` with one active worktree must return a JSON object
/// with `worktrees` array and `all_healthy` boolean.  The single entry must
/// contain the mandatory fields: `ticket`, `has_lock`, `uncommitted`,
/// `stale_days`, `stale`.
#[test]
fn test_health_json_one_worktree() {
    let (repo, _bare) = setup_repo_with_remote();
    let repo_path = repo.path().to_str().unwrap();

    parsec()
        .args(["start", "HL-2", "--repo", repo_path])
        .assert()
        .success();

    let output = parsec()
        .args(["health", "--json", "--repo", repo_path])
        .output()
        .unwrap();

    assert!(output.status.success(), "health --json should exit 0");

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("health --json must emit valid JSON");

    // Top-level shape
    assert!(
        parsed.get("worktrees").is_some(),
        "top-level must have 'worktrees' key"
    );
    assert!(
        parsed.get("all_healthy").is_some(),
        "top-level must have 'all_healthy' key"
    );
    assert!(
        parsed["all_healthy"].is_boolean(),
        "'all_healthy' must be a boolean"
    );

    let worktrees = parsed["worktrees"]
        .as_array()
        .expect("'worktrees' must be an array");
    assert_eq!(worktrees.len(), 1, "expected exactly 1 worktree entry");

    let entry = &worktrees[0];
    assert_eq!(
        entry["ticket"].as_str().unwrap(),
        "HL-2",
        "ticket field mismatch"
    );
    assert!(
        entry.get("has_lock").is_some(),
        "entry must have 'has_lock' field"
    );
    assert!(
        entry.get("uncommitted").is_some(),
        "entry must have 'uncommitted' field"
    );
    assert!(
        entry.get("stale_days").is_some(),
        "entry must have 'stale_days' field"
    );
    assert!(
        entry.get("stale").is_some(),
        "entry must have 'stale' field"
    );

    // A fresh worktree must NOT have a lock file
    assert!(
        !entry["has_lock"].as_bool().unwrap(),
        "fresh worktree must not have index.lock"
    );

    // A fresh worktree with no pending changes has 0 uncommitted files
    assert_eq!(
        entry["uncommitted"].as_u64().unwrap(),
        0,
        "fresh worktree must have 0 uncommitted files"
    );
}

/// `parsec health` must exit 0 even when worktrees have issues — health is
/// informational and must not be used as a CI gate in Phase 1.
#[test]
fn test_health_exit_zero_with_issues() {
    let (repo, _bare) = setup_repo_with_remote();
    let repo_path = repo.path().to_str().unwrap();

    parsec()
        .args(["start", "HL-3", "--repo", repo_path])
        .assert()
        .success();

    // Simulate a stale lock file inside the worktree's git dir.
    // The worktree is a linked worktree, so its .git is a file pointing to the
    // real git dir.  Locate the real git dir and write a lock file there.
    let worktree_path = repo.path().parent().unwrap().join("HL-3");
    let git_file = worktree_path.join(".git");
    let lock_path = if git_file.is_file() {
        let contents = std::fs::read_to_string(&git_file).unwrap();
        let real_git = contents
            .strip_prefix("gitdir: ")
            .unwrap_or("")
            .trim()
            .to_string();
        std::path::PathBuf::from(&real_git).join("index.lock")
    } else {
        git_file.join("index.lock")
    };

    // Write a dummy lock file
    if let Some(parent) = lock_path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::write(&lock_path, b"dummy lock").unwrap();

    // Health must still exit 0 — it is purely informational in Phase 1
    parsec()
        .args(["health", "--repo", repo_path])
        .assert()
        .success();

    // Clean up
    std::fs::remove_file(&lock_path).ok();
}

// ---------------------------------------------------------------------------
// Shell completion scripts (issue #291 Phase 2)
//
// Sanity tests only — verify the scripts ship in the repo and reference the
// __complete subcommand from PR #312. We cannot exercise real shell behavior
// (zsh/bash/fish parsers in the test sandbox is too fragile / heavy), so the
// scripts themselves stand in for the "would this complete?" question.
// ---------------------------------------------------------------------------

fn read_completion(name: &str) -> String {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("completions")
        .join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("completion script {} should exist: {}", path.display(), e))
}

#[test]
fn completion_zsh_present_and_dynamic() {
    let s = read_completion("_parsec");
    assert!(s.contains("#compdef parsec"), "must start with #compdef");
    assert!(
        s.contains("parsec __complete worktrees"),
        "zsh script must call __complete worktrees"
    );
    assert!(
        s.contains("parsec __complete branches"),
        "zsh script must call __complete branches"
    );
    // Confirm we wire ticket-shaped subcommands.
    for sub in ["start", "switch", "ship", "open", "clean", "merge", "ci"] {
        assert!(s.contains(sub), "zsh script must mention {}", sub);
    }
}

#[test]
fn completion_bash_present_and_dynamic() {
    let s = read_completion("parsec.bash");
    assert!(s.contains("complete -F _parsec parsec"));
    assert!(s.contains("parsec __complete worktrees"));
    assert!(s.contains("parsec __complete branches"));
    for sub in ["start", "switch", "ship", "open", "clean", "merge", "ci"] {
        assert!(s.contains(sub), "bash script must mention {}", sub);
    }
}

#[test]
fn completion_fish_present_and_dynamic() {
    let s = read_completion("parsec.fish");
    assert!(s.contains("__parsec_worktrees"));
    assert!(s.contains("__parsec_branches"));
    assert!(s.contains("parsec __complete worktrees"));
    assert!(s.contains("parsec __complete branches"));
    for sub in ["start", "switch", "ship", "open", "clean", "merge", "ci"] {
        assert!(s.contains(sub), "fish script must mention {}", sub);
    }
}

#[test]
fn completion_scripts_reference_phase1_subcommand_signature() {
    // The __complete subcommand only accepts `worktrees` and `branches` kinds
    // today (PR #312). Phase 2 scripts must not use any other kind name or
    // they'll silently emit nothing.
    for name in ["_parsec", "parsec.bash", "parsec.fish"] {
        let s = read_completion(name);
        let valid_kinds = ["worktrees", "branches"];
        for line in s.lines() {
            if let Some(rest) = line.find("parsec __complete ").map(|i| &line[i + 18..]) {
                let kind: String = rest
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric())
                    .collect();
                assert!(
                    valid_kinds.contains(&kind.as_str()),
                    "{}: unknown __complete kind '{}' (allowed: {:?})",
                    name,
                    kind,
                    valid_kinds
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// parsec health Phase 2 (#299) — --stale-days and --no-overlay flags
// ---------------------------------------------------------------------------

/// `parsec health --stale-days 99` exits 0 — the flag is accepted and a
/// generous threshold means the fresh worktree is never flagged stale.
#[test]
fn test_health_stale_days_flag_accepted() {
    let (repo, _bare) = setup_repo_with_remote();
    let repo_path = repo.path().to_str().unwrap();

    parsec()
        .args(["start", "PH2-1", "--repo", repo_path])
        .assert()
        .success();

    parsec()
        .args(["health", "--stale-days", "99", "--repo", repo_path])
        .assert()
        .success();
}

/// `parsec health --no-overlay` must exit 0 on an empty repo — the flag is
/// accepted and the command degrades gracefully to "No active worktrees."
#[test]
fn test_health_no_overlay_flag_accepted() {
    let repo = setup_repo();
    let repo_path = repo.path().to_str().unwrap();

    parsec()
        .args(["health", "--no-overlay", "--repo", repo_path])
        .assert()
        .success()
        .stdout(predicate::str::contains("No active worktrees."));
}

/// `parsec health --no-overlay --json` with one worktree must include the
/// `ci_status` key (value `null`) in the JSON output — schema is stable
/// regardless of whether the CI overlay was attempted.
#[test]
fn test_health_no_overlay_json_has_ci_status_key() {
    let (repo, _bare) = setup_repo_with_remote();
    let repo_path = repo.path().to_str().unwrap();

    parsec()
        .args(["start", "PH2-2", "--repo", repo_path])
        .assert()
        .success();

    let output = parsec()
        .args(["health", "--no-overlay", "--json", "--repo", repo_path])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "health --no-overlay --json must exit 0; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("health --no-overlay --json must emit valid JSON");

    let worktrees = parsed["worktrees"]
        .as_array()
        .expect("'worktrees' must be an array");
    assert!(
        !worktrees.is_empty(),
        "expected at least one worktree entry"
    );

    let entry = &worktrees[0];
    assert!(
        entry.get("ci_status").is_some(),
        "JSON entry must have 'ci_status' key (null when no-overlay)"
    );
    assert!(
        entry.get("pr_number").is_some(),
        "JSON entry must have 'pr_number' key"
    );
}

// ---------------------------------------------------------------------------
// test (parsec test — issue #247)
// ---------------------------------------------------------------------------

#[test]
fn test_test_help_shows_command() {
    parsec()
        .args(["test", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("worktree"));
}

#[test]
fn test_test_runs_in_single_worktree() {
    let (repo, _bare) = setup_repo_with_remote();
    let repo_path = repo.path().to_str().unwrap();

    parsec()
        .args(["start", "TEST-T01", "--repo", repo_path])
        .assert()
        .success();

    parsec()
        .args([
            "test",
            "TEST-T01",
            "--command",
            "exit 0",
            "--repo",
            repo_path,
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("TEST-T01"));
}

#[test]
fn test_test_all_runs_each_worktree() {
    let (repo, _bare) = setup_repo_with_remote();
    let repo_path = repo.path().to_str().unwrap();

    parsec()
        .args(["start", "TEST-T02", "--repo", repo_path])
        .assert()
        .success();
    parsec()
        .args(["start", "TEST-T03", "--repo", repo_path])
        .assert()
        .success();

    parsec()
        .args(["test", "--all", "--command", "exit 0", "--repo", repo_path])
        .assert()
        .success()
        .stdout(predicate::str::contains("TEST-T02"))
        .stdout(predicate::str::contains("TEST-T03"));
}

#[test]
fn test_test_cache_skips_second_run() {
    let (repo, _bare) = setup_repo_with_remote();
    let repo_path = repo.path().to_str().unwrap();

    parsec()
        .args(["start", "TEST-T04", "--repo", repo_path])
        .assert()
        .success();

    // First run: populates the cache.
    parsec()
        .args([
            "test",
            "TEST-T04",
            "--cache",
            "--command",
            "exit 0",
            "--repo",
            repo_path,
        ])
        .assert()
        .success();

    // Second run: must serve from cache (from_cache = true in JSON).
    let output = parsec()
        .args([
            "--json",
            "test",
            "TEST-T04",
            "--cache",
            "--command",
            "exit 0",
            "--repo",
            repo_path,
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "second cached run must exit 0; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("test --json must emit valid JSON");
    let arr = parsed.as_array().expect("test --json must be an array");
    assert_eq!(arr.len(), 1);
    let entry = &arr[0];
    assert_eq!(
        entry["from_cache"].as_bool(),
        Some(true),
        "second invocation must hit the tree-hash cache"
    );
    assert_eq!(entry["exit_code"].as_i64(), Some(0));
}

#[test]
fn test_test_failure_propagates_nonzero() {
    let (repo, _bare) = setup_repo_with_remote();
    let repo_path = repo.path().to_str().unwrap();

    parsec()
        .args(["start", "TEST-T05", "--repo", repo_path])
        .assert()
        .success();

    let output = parsec()
        .args([
            "--json",
            "test",
            "TEST-T05",
            "--command",
            "exit 7",
            "--repo",
            repo_path,
        ])
        .output()
        .unwrap();

    assert!(
        !output.status.success(),
        "test with failing command must exit non-zero"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("\"exit_code\": 7") || stdout.contains("\"exit_code\":7"),
        "JSON must surface the underlying exit code; got: {stdout}"
    );
}

#[cfg(unix)]
#[test]
fn test_test_jobs_parallel_completes() {
    let (repo, _bare) = setup_repo_with_remote();
    let repo_path = repo.path().to_str().unwrap();

    parsec()
        .args(["start", "TEST-T06", "--repo", repo_path])
        .assert()
        .success();
    parsec()
        .args(["start", "TEST-T07", "--repo", repo_path])
        .assert()
        .success();

    let started = std::time::Instant::now();
    parsec()
        .args([
            "test",
            "--all",
            "--jobs",
            "4",
            "--command",
            "sleep 0.2",
            "--repo",
            repo_path,
        ])
        .assert()
        .success();
    let elapsed = started.elapsed();

    // Two sequential sleeps would take >= 0.4s. Parallel must beat that
    // comfortably even with process spawn overhead.
    assert!(
        elapsed < std::time::Duration::from_millis(2_000),
        "parallel --jobs run should finish well under 2s, took {:?}",
        elapsed
    );
}
