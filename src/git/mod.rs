use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Run a git command in `repo`, returning an error if the exit code is non-zero.
pub fn run(repo: &Path, args: &[&str]) -> Result<()> {
    let status = Command::new("git")
        .args(args)
        .current_dir(repo)
        .status()
        .with_context(|| format!("failed to spawn git {:?}", args))?;

    if status.success() {
        Ok(())
    } else {
        Err(anyhow!(
            "git {:?} exited with status {}",
            args,
            status.code().unwrap_or(-1)
        ))
    }
}

/// Run a git command in `repo`, returning stdout trimmed on success.
pub fn run_output(repo: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .with_context(|| format!("failed to spawn git {:?}", args))?;

    if output.status.success() {
        let stdout = String::from_utf8(output.stdout)
            .with_context(|| format!("git {:?} produced non-UTF-8 output", args))?;
        Ok(stdout.trim().to_owned())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(anyhow!(
            "git {:?} exited with status {}: {}",
            args,
            output.status.code().unwrap_or(-1),
            stderr.trim()
        ))
    }
}

/// Detect the default branch: checks for `refs/heads/main`, falls back to `master`.
pub fn get_default_branch(repo: &Path) -> Result<String> {
    // Try to resolve refs/heads/main
    let result = Command::new("git")
        .args(["rev-parse", "--verify", "refs/heads/main"])
        .current_dir(repo)
        .output()
        .context("failed to spawn git rev-parse")?;

    if result.status.success() {
        return Ok("main".to_owned());
    }

    // Fall back to master
    let result = Command::new("git")
        .args(["rev-parse", "--verify", "refs/heads/master"])
        .current_dir(repo)
        .output()
        .context("failed to spawn git rev-parse")?;

    if result.status.success() {
        return Ok("master".to_owned());
    }

    Err(anyhow!(
        "could not detect default branch: neither 'main' nor 'master' exists in {:?}",
        repo
    ))
}

/// Return the repository root by running `git rev-parse --show-toplevel`.
/// Note: in a worktree, this returns the worktree root, not the main repo.
pub fn get_repo_root(path: &Path) -> Result<PathBuf> {
    let out = run_output(path, &["rev-parse", "--show-toplevel"])?;
    Ok(PathBuf::from(out))
}

/// Return the main repository root, even when called from a worktree.
/// Uses `git rev-parse --git-common-dir` to find the shared `.git` directory,
/// then derives the main repo root from it.
pub fn get_main_repo_root(path: &Path) -> Result<PathBuf> {
    let common_dir = run_output(path, &["rev-parse", "--git-common-dir"])?;
    let common_path = PathBuf::from(&common_dir);

    // If common_dir is ".git", we're in the main repo — use --show-toplevel
    // If it's an absolute path like "/repo/.git", parent is the main repo root
    // If it's a relative path like "../../repo/.git", resolve it
    let abs_common = if common_path.is_absolute() {
        common_path
    } else {
        let toplevel = get_repo_root(path)?;
        toplevel.join(&common_path)
    };

    // The main repo root is the parent of the .git directory
    let canonical = abs_common
        .canonicalize()
        .with_context(|| format!("failed to canonicalize git common dir: {:?}", abs_common))?;

    canonical
        .parent()
        .map(|p| p.to_path_buf())
        .ok_or_else(|| anyhow!("git common dir has no parent: {:?}", canonical))
}

/// Get files with uncommitted changes (staged + unstaged) in the working tree.
pub fn get_uncommitted_files(repo: &Path) -> Result<Vec<String>> {
    // Staged changes
    let staged = run_output(repo, &["diff", "--name-only", "--cached"])?;
    // Unstaged changes
    let unstaged = run_output(repo, &["diff", "--name-only"])?;

    let mut files: std::collections::HashSet<String> = std::collections::HashSet::new();
    for line in staged.lines().chain(unstaged.lines()) {
        let line = line.trim();
        if !line.is_empty() {
            files.insert(line.to_owned());
        }
    }
    Ok(files.into_iter().collect())
}

/// Return `true` if `branch` has been fully merged into `base`.
pub fn is_branch_merged(repo: &Path, branch: &str, base: &str) -> Result<bool> {
    // `git branch --merged <base>` lists branches merged into base.
    let output = run_output(repo, &["branch", "--merged", base])?;
    Ok(output
        .lines()
        .any(|line| line.trim().trim_start_matches('*').trim() == branch))
}

/// Return the list of files changed between `base` and `head` (`git diff --name-only base..head`).
pub fn get_changed_files(repo: &Path, base: &str, head: &str) -> Result<Vec<String>> {
    let range = format!("{}..{}", base, head);
    let output = run_output(repo, &["diff", "--name-only", &range])?;
    Ok(output
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| l.to_owned())
        .collect())
}

/// Return the URL of the `origin` remote.
pub fn get_remote_url(repo: &Path) -> Result<String> {
    run_output(repo, &["remote", "get-url", "origin"])
}

/// Push `branch` to origin and set the upstream tracking reference.
pub fn push_branch(repo: &Path, branch: &str) -> Result<()> {
    run(repo, &["push", "--force-with-lease", "-u", "origin", branch])
}

/// Return the name of the currently checked-out branch.
#[allow(dead_code)]
pub fn get_current_branch(repo: &Path) -> Result<String> {
    run_output(repo, &["rev-parse", "--abbrev-ref", "HEAD"])
}

/// Create a new worktree at `path` on a new branch `branch` based on `base`.
pub fn worktree_add(repo: &Path, path: &Path, branch: &str, base: &str) -> Result<()> {
    let path_str = path
        .to_str()
        .ok_or_else(|| anyhow!("worktree path is not valid UTF-8: {:?}", path))?;
    run(repo, &["worktree", "add", "-b", branch, path_str, base])
}

/// Create a worktree at `path` for an already-existing branch.
/// If the branch only exists on the remote, it is fetched and tracked locally first.
pub fn worktree_add_existing(repo: &Path, path: &Path, branch: &str) -> Result<()> {
    let path_str = path
        .to_str()
        .ok_or_else(|| anyhow!("worktree path is not valid UTF-8: {:?}", path))?;

    // Check if branch exists locally
    let local_exists = run_output(
        repo,
        &["rev-parse", "--verify", &format!("refs/heads/{}", branch)],
    )
    .is_ok();

    if local_exists {
        return run(repo, &["worktree", "add", path_str, branch]);
    }

    // Check if it exists on remote (handle "origin/branch" syntax too)
    let remote_ref = if branch.starts_with("origin/") {
        branch.to_owned()
    } else {
        format!("origin/{}", branch)
    };

    let remote_exists = run_output(
        repo,
        &[
            "rev-parse",
            "--verify",
            &format!("refs/remotes/{}", remote_ref),
        ],
    )
    .is_ok();

    if remote_exists {
        // Strip "origin/" prefix for the local branch name
        let local_name = branch.strip_prefix("origin/").unwrap_or(branch);
        // Create local tracking branch and worktree in one step
        run(
            repo,
            &["worktree", "add", "-b", local_name, path_str, &remote_ref],
        )
    } else {
        Err(anyhow!(
            "branch '{}' not found locally or on remote. Use `parsec start {}` without --branch to create a new branch.",
            branch,
            branch.strip_prefix("origin/").unwrap_or(branch)
        ))
    }
}

/// Remove the worktree rooted at `path`.
pub fn worktree_remove(repo: &Path, path: &Path) -> Result<()> {
    let path_str = path
        .to_str()
        .ok_or_else(|| anyhow!("worktree path is not valid UTF-8: {:?}", path))?;
    run(repo, &["worktree", "remove", path_str])
}

/// List worktrees, returning `(path, branch, HEAD-sha)` tuples parsed from
/// `git worktree list --porcelain`.
#[allow(dead_code)]
pub fn worktree_list(repo: &Path) -> Result<Vec<(String, String, String)>> {
    let output = run_output(repo, &["worktree", "list", "--porcelain"])?;

    let mut result = Vec::new();
    let mut wt_path = String::new();
    let mut wt_head = String::new();
    let mut wt_branch = String::new();

    for line in output.lines() {
        if let Some(val) = line.strip_prefix("worktree ") {
            // Start of a new worktree block — flush the previous one.
            if !wt_path.is_empty() {
                result.push((wt_path.clone(), wt_branch.clone(), wt_head.clone()));
            }
            wt_path = val.to_owned();
            wt_head.clear();
            wt_branch.clear();
        } else if let Some(val) = line.strip_prefix("HEAD ") {
            wt_head = val.to_owned();
        } else if let Some(val) = line.strip_prefix("branch ") {
            // Porcelain format: "branch refs/heads/<name>"
            wt_branch = val.strip_prefix("refs/heads/").unwrap_or(val).to_owned();
        } else if line == "detached" {
            wt_branch = "(detached)".to_owned();
        }
    }

    // Flush the last block.
    if !wt_path.is_empty() {
        result.push((wt_path, wt_branch, wt_head));
    }

    Ok(result)
}

/// Return the best common ancestor of commits `a` and `b`.
pub fn get_merge_base(repo: &Path, a: &str, b: &str) -> Result<String> {
    run_output(repo, &["merge-base", a, b])
}

/// Force-delete a local branch.
pub fn delete_branch(repo: &Path, branch: &str) -> Result<()> {
    run(repo, &["branch", "-D", branch])
}

/// Fetch all refs from `origin`.
pub fn fetch(repo: &Path) -> Result<()> {
    run(repo, &["fetch", "origin"])
}

/// Fetch from origin if a remote exists. Non-fatal if no remote configured.
pub fn fetch_if_remote(repo: &Path) -> Result<()> {
    // Check if remote exists first
    let has_remote = std::process::Command::new("git")
        .args(["remote"])
        .current_dir(repo)
        .output()
        .map(|o| !String::from_utf8_lossy(&o.stdout).trim().is_empty())
        .unwrap_or(false);

    if has_remote {
        // Fetch but don't fail hard - just warn
        if let Err(e) = fetch(repo) {
            eprintln!("warning: fetch failed (continuing anyway): {}", e);
        }
    }
    Ok(())
}
