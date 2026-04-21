use std::path::Path;

use anyhow::Result;
use colored::Colorize;

use crate::config::ParsecConfig;
use crate::git;
use crate::github;
use crate::output::Mode;

pub async fn release(
    repo: &Path,
    version: &str,
    from: Option<&str>,
    no_github_release: bool,
    dry_run: bool,
    mode: Mode,
) -> Result<()> {
    let config = ParsecConfig::load()?;
    let repo_root = git::get_repo_root(repo)?;

    // Resolve source branch: --from > "develop" > git default branch
    let source_branch = if let Some(f) = from {
        f.to_string()
    } else {
        // Try "develop" first, fall back to default branch
        let has_develop =
            git::run_output(&repo_root, &["rev-parse", "--verify", "refs/heads/develop"]).is_ok();
        if has_develop {
            "develop".to_string()
        } else {
            git::get_default_branch(&repo_root)?
        }
    };

    // Resolve target branch from config (default: "main")
    let target_branch = config.release.branch.clone();
    let tag_prefix = config.release.tag_prefix.clone();
    let tag = format!("{}{}", tag_prefix, version);

    let step = |msg: &str| {
        if mode != Mode::Quiet {
            println!("  {}", msg);
        }
    };

    if dry_run && mode != Mode::Quiet {
        println!("Dry run — no changes will be made.\n");
    }

    // Step a: git fetch origin
    step("Fetching from origin...");
    if !dry_run {
        git::run(&repo_root, &["fetch", "origin"])?;
    }

    // Step b: Verify source branch is up to date with origin
    step(&format!(
        "Verifying '{}' is up to date with origin...",
        source_branch
    ));
    if !dry_run {
        // Get local and remote SHAs
        let local_sha = git::run_output(&repo_root, &["rev-parse", &source_branch])?;
        let remote_ref = format!("origin/{}", source_branch);
        let remote_sha =
            git::run_output(&repo_root, &["rev-parse", &remote_ref]).unwrap_or_default();
        if !remote_sha.is_empty() && local_sha != remote_sha {
            // Check if local is behind remote
            let behind = git::run_output(
                &repo_root,
                &[
                    "rev-list",
                    "--count",
                    &format!("{}..{}", source_branch, remote_ref),
                ],
            )
            .unwrap_or_default();
            let behind_n: u32 = behind.trim().parse().unwrap_or(0);
            if behind_n > 0 {
                anyhow::bail!(
                    "branch '{}' is {} commit(s) behind origin/{}. Pull first.",
                    source_branch,
                    behind_n,
                    source_branch
                );
            }
        }
    }

    // Step c: checkout target branch and pull
    step(&format!("Checking out '{}' and pulling...", target_branch));
    if !dry_run {
        git::run(&repo_root, &["checkout", &target_branch])?;
        git::run(&repo_root, &["pull", "origin", &target_branch])?;
    }

    // Step d: merge source branch with --no-ff
    let merge_msg = format!("Release {}", version);
    step(&format!(
        "Merging '{}' into '{}' (no-ff)...",
        source_branch, target_branch
    ));
    if !dry_run {
        git::run(
            &repo_root,
            &["merge", &source_branch, "--no-ff", "-m", &merge_msg],
        )?;
    }

    // Step e: create annotated tag
    step(&format!("Creating tag '{}'...", tag));
    if !dry_run {
        git::run(&repo_root, &["tag", "-a", &tag, "-m", &merge_msg])?;
    }

    // Step f: push with tags
    step(&format!("Pushing '{}' with tags...", target_branch));
    if !dry_run {
        git::run(
            &repo_root,
            &["push", "origin", &target_branch, "--follow-tags"],
        )?;
    }

    // Step g: GitHub Release
    let release_url = if !no_github_release {
        // Check for GitHub remote
        let remote_url = git::run_output(&repo_root, &["remote", "get-url", "origin"]).ok();

        if let Some(ref remote) = remote_url {
            if github::parse_github_remote(remote).is_some() {
                // Generate changelog from git log between previous tag and new tag
                let changelog = if config.release.changelog {
                    // Find the previous tag
                    let prev_tag = git::run_output(
                        &repo_root,
                        &["describe", "--tags", "--abbrev=0", &format!("{}^", tag)],
                    )
                    .ok();

                    let range = if let Some(ref pt) = prev_tag {
                        format!("{}..{}", pt, tag)
                    } else {
                        tag.clone()
                    };

                    let log = if dry_run {
                        // In dry run, use HEAD instead of the new tag (not yet created)
                        let dry_range = if let Some(ref pt) = prev_tag {
                            format!("{}..HEAD", pt)
                        } else {
                            "HEAD".to_string()
                        };
                        git::run_output(
                            &repo_root,
                            &["log", &dry_range, "--pretty=format:- %s (%h)"],
                        )
                        .unwrap_or_default()
                    } else {
                        git::run_output(&repo_root, &["log", &range, "--pretty=format:- %s (%h)"])
                            .unwrap_or_default()
                    };
                    log
                } else {
                    String::new()
                };

                let release_name = format!("{}{}", tag_prefix, version);
                step(&format!("Creating GitHub Release '{}'...", release_name));

                if !dry_run {
                    if let Some(gh) = github::GitHubClient::new(remote, &config)? {
                        match gh.create_release(&tag, &release_name, &changelog).await {
                            Ok(url) => Some(url),
                            Err(e) => {
                                eprintln!("warning: GitHub Release creation failed: {e}");
                                None
                            }
                        }
                    } else {
                        eprintln!(
                            "warning: no GitHub token found, skipping GitHub Release creation."
                        );
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    // Step h: Print summary
    if mode == Mode::Json {
        let summary = serde_json::json!({
            "version": version,
            "tag": tag,
            "source_branch": source_branch,
            "target_branch": target_branch,
            "dry_run": dry_run,
            "github_release_url": release_url,
        });
        println!("{}", serde_json::to_string_pretty(&summary)?);
    } else if mode == Mode::Human {
        println!();
        println!("{}", "Release complete!".green().bold());
        println!("  Version : {}", version.bold());
        println!("  Tag     : {}", tag.bold());
        println!("  Branch  : {} -> {}", source_branch, target_branch);
        if dry_run {
            println!("  (dry run — no changes were made)");
        }
        if let Some(url) = &release_url {
            println!("  Release : {}", url);
        }
    }

    Ok(())
}
