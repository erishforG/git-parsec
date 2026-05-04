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
