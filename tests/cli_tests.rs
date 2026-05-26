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
