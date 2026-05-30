use std::path::Path;

use crate::config::CacheStrategy;

/// Share build-cache directories from the main repo into a freshly created
/// worktree. Each entry is processed independently; failures are logged but
/// never propagated, so a flaky cache share never breaks `parsec start`.
///
/// - Source path is `<repo_root>/<entry>`. Missing → skip.
/// - Destination path is `<worktree_path>/<entry>`. Already exists → skip.
/// - `Symlink` creates a symlink to the absolute source path; `Copy` does a
///   recursive copy using stdlib only (no extra dependency).
pub fn share_cache(
    repo_root: &Path,
    worktree_path: &Path,
    entries: &[String],
    strategy: CacheStrategy,
) {
    if entries.is_empty() {
        return;
    }

    for entry in entries {
        if entry.is_empty() || entry.contains("..") {
            eprintln!("warning: skipping invalid shared_cache entry {:?}", entry);
            continue;
        }

        let src = repo_root.join(entry);
        let dest = worktree_path.join(entry);

        if !src.exists() {
            eprintln!(
                "info: shared_cache: source '{}' does not exist in main repo, skipping",
                entry
            );
            continue;
        }

        if dest.exists() || dest.symlink_metadata().is_ok() {
            eprintln!(
                "info: shared_cache: destination '{}' already exists in worktree, skipping",
                entry
            );
            continue;
        }

        // Ensure dest's parent exists (for nested entries like "a/b/target").
        if let Some(parent) = dest.parent() {
            if !parent.exists() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    eprintln!(
                        "warning: shared_cache: failed to create parent for '{}': {e}",
                        entry
                    );
                    continue;
                }
            }
        }

        let abs_src = match dunce::canonicalize(&src) {
            Ok(p) => p,
            Err(e) => {
                eprintln!(
                    "warning: shared_cache: cannot resolve source '{}': {e}",
                    entry
                );
                continue;
            }
        };

        let result = match strategy {
            CacheStrategy::Symlink => create_symlink(&abs_src, &dest),
            CacheStrategy::Copy => copy_recursive(&abs_src, &dest),
        };

        match result {
            Ok(()) => {
                eprintln!(
                    "info: shared_cache: {} '{}' from {} -> {}",
                    strategy,
                    entry,
                    abs_src.display(),
                    dest.display()
                );
            }
            Err(e) => {
                eprintln!(
                    "warning: shared_cache: failed to share '{}' ({}): {e}",
                    entry, strategy
                );
            }
        }
    }
}

#[cfg(unix)]
fn create_symlink(src: &Path, dest: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(src, dest)
}

#[cfg(windows)]
fn create_symlink(src: &Path, dest: &Path) -> std::io::Result<()> {
    if src.is_dir() {
        std::os::windows::fs::symlink_dir(src, dest)
    } else {
        std::os::windows::fs::symlink_file(src, dest)
    }
}

fn copy_recursive(src: &Path, dest: &Path) -> std::io::Result<()> {
    let metadata = std::fs::symlink_metadata(src)?;
    let file_type = metadata.file_type();

    if file_type.is_symlink() {
        // Follow symlinks during copy (resolving once); fall back to plain copy.
        let target = std::fs::read_link(src)?;
        let resolved = if target.is_absolute() {
            target
        } else {
            src.parent().unwrap_or(Path::new(".")).join(target)
        };
        return copy_recursive(&resolved, dest);
    }

    if file_type.is_dir() {
        std::fs::create_dir_all(dest)?;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            let child_src = entry.path();
            let child_dest = dest.join(entry.file_name());
            copy_recursive(&child_src, &child_dest)?;
        }
        Ok(())
    } else {
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(src, dest).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn read_file(p: &Path) -> String {
        fs::read_to_string(p).unwrap()
    }

    fn make_dirs() -> (TempDir, std::path::PathBuf, std::path::PathBuf) {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path().join("repo");
        let wt = tmp.path().join("worktree");
        fs::create_dir_all(&repo).unwrap();
        fs::create_dir_all(&wt).unwrap();
        (tmp, repo, wt)
    }

    #[test]
    fn symlink_strategy_links_existing_dir() {
        let (_tmp, repo, wt) = make_dirs();
        fs::create_dir_all(repo.join("target")).unwrap();
        fs::write(repo.join("target/build.txt"), "hello").unwrap();

        share_cache(&repo, &wt, &["target".to_string()], CacheStrategy::Symlink);

        let dest = wt.join("target");
        assert!(dest.exists());
        let meta = fs::symlink_metadata(&dest).unwrap();
        assert!(meta.file_type().is_symlink(), "should be a symlink");
        assert_eq!(read_file(&dest.join("build.txt")), "hello");
    }

    #[test]
    fn copy_strategy_creates_real_dir() {
        let (_tmp, repo, wt) = make_dirs();
        fs::create_dir_all(repo.join("target/nested")).unwrap();
        fs::write(repo.join("target/a.txt"), "alpha").unwrap();
        fs::write(repo.join("target/nested/b.txt"), "beta").unwrap();

        share_cache(&repo, &wt, &["target".to_string()], CacheStrategy::Copy);

        let dest = wt.join("target");
        assert!(dest.exists());
        let meta = fs::symlink_metadata(&dest).unwrap();
        assert!(!meta.file_type().is_symlink(), "must not be a symlink");
        assert!(meta.is_dir());
        assert_eq!(read_file(&dest.join("a.txt")), "alpha");
        assert_eq!(read_file(&dest.join("nested/b.txt")), "beta");
    }

    #[test]
    fn missing_entry_is_skipped_silently() {
        let (_tmp, repo, wt) = make_dirs();

        share_cache(
            &repo,
            &wt,
            &["does-not-exist".to_string()],
            CacheStrategy::Symlink,
        );

        assert!(!wt.join("does-not-exist").exists());
    }

    #[test]
    fn existing_dest_is_not_overwritten() {
        let (_tmp, repo, wt) = make_dirs();
        fs::create_dir_all(repo.join("target")).unwrap();
        fs::write(repo.join("target/from_repo.txt"), "repo").unwrap();
        fs::create_dir_all(wt.join("target")).unwrap();
        fs::write(wt.join("target/preexisting.txt"), "keep").unwrap();

        share_cache(&repo, &wt, &["target".to_string()], CacheStrategy::Copy);

        // Pre-existing content untouched, repo content not copied in.
        assert!(wt.join("target/preexisting.txt").exists());
        assert!(!wt.join("target/from_repo.txt").exists());
    }

    #[test]
    fn empty_list_is_noop() {
        let (_tmp, repo, wt) = make_dirs();
        share_cache(&repo, &wt, &[], CacheStrategy::Symlink);
        // Just verify nothing was created in the worktree.
        let entries: Vec<_> = fs::read_dir(&wt).unwrap().collect();
        assert!(entries.is_empty());
    }

    #[test]
    fn path_traversal_entries_rejected() {
        let (_tmp, repo, wt) = make_dirs();
        fs::create_dir_all(repo.join("evil")).unwrap();

        share_cache(&repo, &wt, &["../evil".to_string()], CacheStrategy::Symlink);

        // Nothing should have been created.
        let entries: Vec<_> = fs::read_dir(&wt).unwrap().collect();
        assert!(entries.is_empty());
    }
}
