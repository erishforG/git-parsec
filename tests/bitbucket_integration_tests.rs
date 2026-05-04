//! End-to-end tests that exercise the Bitbucket Cloud code path of `parsec ci`
//! and `parsec pr-status` against a mocked Bitbucket API server.
//!
//! These tests verify (a) the dispatch logic actually picks the Bitbucket path
//! instead of GitHub when the origin remote is Bitbucket, and (b) the response
//! mapping (ci_status, review_status) reflects the live API payload.

use assert_cmd::Command;
use mockito::{Matcher, Server, ServerGuard};
use std::process::Command as StdCommand;
use tempfile::TempDir;

const WORKSPACE: &str = "fakews";
const REPO_SLUG: &str = "fakerepo";

/// Initialize a git repo whose `origin` points at a Bitbucket Cloud URL.
/// No actual remote backs the URL — these tests only exercise API calls,
/// never `git fetch` / `git push`.
fn setup_bitbucket_repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    let p = dir.path();

    StdCommand::new("git")
        .args(["init"])
        .current_dir(p)
        .output()
        .unwrap();
    StdCommand::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(p)
        .output()
        .unwrap();
    StdCommand::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(p)
        .output()
        .unwrap();
    StdCommand::new("git")
        .args(["checkout", "-b", "main"])
        .current_dir(p)
        .output()
        .unwrap();
    StdCommand::new("git")
        .args(["commit", "--allow-empty", "-m", "init"])
        .current_dir(p)
        .output()
        .unwrap();
    StdCommand::new("git")
        .args([
            "remote",
            "add",
            "origin",
            &format!("git@bitbucket.org:{}/{}.git", WORKSPACE, REPO_SLUG),
        ])
        .current_dir(p)
        .output()
        .unwrap();

    dir
}

/// Drop a fake oplog Ship entry so `parsec pr-status` / `parsec ci` resolve
/// the PR number from the log without needing a live workspace.
fn write_oplog_ship_entry(repo: &std::path::Path, ticket: &str, pr_number: u64) {
    let parsec_dir = repo.join(".parsec");
    std::fs::create_dir_all(&parsec_dir).unwrap();
    let body = serde_json::json!({
        "entries": [{
            "id": 1,
            "op": "ship",
            "ticket": ticket,
            "detail": format!(
                "Shipped branch 'feature/{0}' -> https://bitbucket.org/{1}/{2}/pull-requests/{3}",
                ticket, WORKSPACE, REPO_SLUG, pr_number
            ),
            "timestamp": "2024-01-01T00:00:00Z",
            "undo_info": null
        }]
    });
    std::fs::write(
        parsec_dir.join("oplog.json"),
        serde_json::to_string_pretty(&body).unwrap(),
    )
    .unwrap();
}

fn parsec(server: &ServerGuard) -> Command {
    let mut cmd = Command::cargo_bin("parsec").unwrap();
    // Isolate from any user-level config (e.g. existing default_base) so the
    // subprocess sees only the env we provide.
    cmd.env("PARSEC_CONFIG_DIR", "/tmp/parsec-test-nonexistent")
        .env("PARSEC_BITBUCKET_TOKEN", "fake-token-for-test")
        .env("PARSEC_BITBUCKET_API_BASE", server.url())
        // Defensive: don't let an inherited GitHub token cause the dispatcher
        // to pick the GitHub forge for our bitbucket.org-style remote.
        .env_remove("PARSEC_GITHUB_TOKEN")
        .env_remove("GITHUB_TOKEN")
        .env_remove("GH_TOKEN");
    cmd
}

/// Build the API path prefix used in mock URLs.
fn pr_path(pr_id: u64) -> String {
    format!(
        "/repositories/{}/{}/pullrequests/{}",
        WORKSPACE, REPO_SLUG, pr_id
    )
}

fn pipelines_path() -> String {
    format!("/repositories/{}/{}/pipelines/", WORKSPACE, REPO_SLUG)
}

// ---------------------------------------------------------------------------
// pr-status
// ---------------------------------------------------------------------------

#[test]
fn pr_status_bitbucket_maps_ci_and_review_from_api() {
    let repo = setup_bitbucket_repo();
    let repo_path = repo.path().to_str().unwrap();

    let mut server = Server::new();

    // PR JSON is reused for get_pr_status, get_pr_source_branch, and
    // get_pr_participants — they all hit the same endpoint. Two reviewers:
    // one approved, one no-action → review_status == "approved".
    let pr_body = serde_json::json!({
        "id": 42,
        "title": "Add Bitbucket pipelines support",
        "state": "OPEN",
        "links": { "html": { "href": "https://bitbucket.org/fakews/fakerepo/pull-requests/42" } },
        "source": { "branch": { "name": "feature/BB-1" } },
        "participants": [
            { "state": "approved", "approved": true, "role": "REVIEWER" },
            { "state": null, "approved": false, "role": "REVIEWER" }
        ]
    });
    let pr_mock = server
        .mock("GET", pr_path(42).as_str())
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(pr_body.to_string())
        .expect_at_least(2) // status + source-branch + participants
        .create();

    // Pipeline for the source branch: COMPLETED + SUCCESSFUL → ci_status "passing".
    let pipelines_body = serde_json::json!({
        "values": [{
            "uuid": "{abc-123}",
            "state": { "name": "COMPLETED", "result": { "name": "SUCCESSFUL" } },
            "target": { "ref_name": "feature/BB-1" }
        }]
    });
    let pipeline_mock = server
        .mock("GET", pipelines_path().as_str())
        .match_query(Matcher::AllOf(vec![Matcher::UrlEncoded(
            "target.ref_name".into(),
            "feature/BB-1".into(),
        )]))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(pipelines_body.to_string())
        .create();

    write_oplog_ship_entry(repo.path(), "BB-1", 42);

    let output = parsec(&server)
        .args(["--json", "pr-status", "BB-1", "--repo", repo_path])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout.clone()).unwrap();
    let stderr = String::from_utf8(output.stderr.clone()).unwrap();
    assert!(
        output.status.success(),
        "pr-status should succeed.\nstdout:\n{stdout}\nstderr:\n{stderr}",
    );

    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("pr-status --json must produce valid JSON");
    let arr = parsed.as_array().expect("output should be a JSON array");
    assert_eq!(arr.len(), 1);
    let entry = &arr[0];
    assert_eq!(entry["ticket"], "BB-1");
    assert_eq!(entry["pr_number"], 42);
    assert_eq!(entry["state"], "open");
    assert_eq!(
        entry["ci_status"], "passing",
        "ci_status should come from the Bitbucket Pipelines mock"
    );
    assert_eq!(
        entry["review_status"], "approved",
        "review_status should reflect the participants payload"
    );

    pr_mock.assert();
    pipeline_mock.assert();
}

#[test]
fn pr_status_bitbucket_no_pipeline_yet_is_no_checks() {
    let repo = setup_bitbucket_repo();
    let repo_path = repo.path().to_str().unwrap();

    let mut server = Server::new();

    let pr_body = serde_json::json!({
        "id": 7,
        "title": "Edge case PR",
        "state": "OPEN",
        "links": { "html": { "href": "https://bitbucket.org/fakews/fakerepo/pull-requests/7" } },
        "source": { "branch": { "name": "feature/BB-7" } },
        "participants": []
    });
    let _pr_mock = server
        .mock("GET", pr_path(7).as_str())
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(pr_body.to_string())
        .expect_at_least(2)
        .create();

    // No pipeline runs yet for this branch.
    let _pipeline_mock = server
        .mock("GET", pipelines_path().as_str())
        .match_query(Matcher::UrlEncoded(
            "target.ref_name".into(),
            "feature/BB-7".into(),
        ))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"values":[]}"#)
        .create();

    write_oplog_ship_entry(repo.path(), "BB-7", 7);

    let output = parsec(&server)
        .args(["--json", "pr-status", "BB-7", "--repo", repo_path])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let entry = &parsed.as_array().unwrap()[0];
    assert_eq!(
        entry["ci_status"], "no checks",
        "no pipeline runs → ci_status \"no checks\""
    );
    assert_eq!(
        entry["review_status"], "no reviews",
        "no participants → review_status \"no reviews\""
    );
}

#[test]
fn pr_status_bitbucket_changes_requested_review() {
    let repo = setup_bitbucket_repo();
    let repo_path = repo.path().to_str().unwrap();

    let mut server = Server::new();

    let pr_body = serde_json::json!({
        "id": 9,
        "title": "Needs work",
        "state": "OPEN",
        "links": { "html": { "href": "https://bitbucket.org/fakews/fakerepo/pull-requests/9" } },
        "source": { "branch": { "name": "feature/BB-9" } },
        "participants": [
            { "state": "approved", "approved": true, "role": "REVIEWER" },
            { "state": "changes_requested", "approved": false, "role": "REVIEWER" }
        ]
    });
    let _pr_mock = server
        .mock("GET", pr_path(9).as_str())
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(pr_body.to_string())
        .expect_at_least(2)
        .create();

    let pipelines_body = serde_json::json!({
        "values": [{
            "uuid": "{xyz-9}",
            "state": { "name": "COMPLETED", "result": { "name": "FAILED" } },
            "target": { "ref_name": "feature/BB-9" }
        }]
    });
    let _pipeline_mock = server
        .mock("GET", pipelines_path().as_str())
        .match_query(Matcher::UrlEncoded(
            "target.ref_name".into(),
            "feature/BB-9".into(),
        ))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(pipelines_body.to_string())
        .create();

    write_oplog_ship_entry(repo.path(), "BB-9", 9);

    let output = parsec(&server)
        .args(["--json", "pr-status", "BB-9", "--repo", repo_path])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let entry = &parsed.as_array().unwrap()[0];
    assert_eq!(entry["ci_status"], "failing");
    assert_eq!(entry["review_status"], "changes_requested");
}

// ---------------------------------------------------------------------------
// ci
// ---------------------------------------------------------------------------

#[test]
fn ci_bitbucket_uses_pipelines_endpoint() {
    let repo = setup_bitbucket_repo();
    let repo_path = repo.path().to_str().unwrap();

    let mut server = Server::new();

    // PR endpoint must respond so `fetch_bitbucket_ci` can resolve the source branch.
    let pr_body = serde_json::json!({
        "id": 100,
        "title": "CI test",
        "state": "OPEN",
        "links": { "html": { "href": "https://bitbucket.org/fakews/fakerepo/pull-requests/100" } },
        "source": { "branch": { "name": "feature/CI-1" } }
    });
    let _pr_mock = server
        .mock("GET", pr_path(100).as_str())
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(pr_body.to_string())
        .create();

    // Pipeline run that's still in progress.
    let pipelines_body = serde_json::json!({
        "values": [{
            "uuid": "{ci-1}",
            "state": { "name": "IN_PROGRESS", "result": null },
            "target": { "ref_name": "feature/CI-1" }
        }]
    });
    let pipeline_mock = server
        .mock("GET", pipelines_path().as_str())
        .match_query(Matcher::UrlEncoded(
            "target.ref_name".into(),
            "feature/CI-1".into(),
        ))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(pipelines_body.to_string())
        .expect_at_least(1)
        .create();

    // Crucial: assert that the GitHub commit-status / check-runs endpoints are
    // never hit. Mockito returns 501 for unmatched paths by default; that
    // would blow up the request. We use a catch-all for /repos/* to detect
    // accidental GitHub dispatch and fail loudly.
    let github_mock = server
        .mock("GET", Matcher::Regex("^/repos/.*".into()))
        .with_status(500)
        .with_body("github endpoint should not be hit for a Bitbucket remote")
        .expect(0)
        .create();

    write_oplog_ship_entry(repo.path(), "CI-1", 100);

    let output = parsec(&server)
        .args(["--json", "ci", "CI-1", "--repo", repo_path])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout.clone()).unwrap();
    let stderr = String::from_utf8(output.stderr.clone()).unwrap();
    assert!(
        output.status.success(),
        "ci should succeed for an in-progress pipeline.\nstdout:\n{stdout}\nstderr:\n{stderr}",
    );

    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let entry = &parsed.as_array().unwrap()[0];
    assert_eq!(entry["ticket"], "CI-1");
    assert_eq!(entry["pr_number"], 100);
    assert_eq!(
        entry["overall"], "pending",
        "in-progress pipeline → overall \"pending\""
    );

    pipeline_mock.assert();
    github_mock.assert();
}

#[test]
fn ci_bitbucket_failing_pipeline_exits_nonzero() {
    let repo = setup_bitbucket_repo();
    let repo_path = repo.path().to_str().unwrap();

    let mut server = Server::new();

    let pr_body = serde_json::json!({
        "id": 200,
        "title": "Broken build",
        "state": "OPEN",
        "links": { "html": { "href": "https://bitbucket.org/fakews/fakerepo/pull-requests/200" } },
        "source": { "branch": { "name": "feature/CI-2" } }
    });
    let _pr_mock = server
        .mock("GET", pr_path(200).as_str())
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(pr_body.to_string())
        .create();

    let pipelines_body = serde_json::json!({
        "values": [{
            "uuid": "{ci-2}",
            "state": { "name": "COMPLETED", "result": { "name": "FAILED" } },
            "target": { "ref_name": "feature/CI-2" }
        }]
    });
    let _pipeline_mock = server
        .mock("GET", pipelines_path().as_str())
        .match_query(Matcher::UrlEncoded(
            "target.ref_name".into(),
            "feature/CI-2".into(),
        ))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(pipelines_body.to_string())
        .create();

    write_oplog_ship_entry(repo.path(), "CI-2", 200);

    let output = parsec(&server)
        .args(["--json", "ci", "CI-2", "--repo", repo_path])
        .output()
        .unwrap();

    // Failing CI is a hard error (E002) — exit code is non-zero, but the JSON
    // status line is printed to stdout before the error JSON is appended.
    assert!(
        !output.status.success(),
        "failing pipeline should exit non-zero"
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    // First line: the CI status array. Second line: the JSON error envelope.
    let first_line = stdout.lines().next().expect("expected at least one line");
    let parsed: serde_json::Value = serde_json::from_str(first_line).unwrap();
    let entry = &parsed.as_array().unwrap()[0];
    assert_eq!(entry["overall"], "failing");
}
