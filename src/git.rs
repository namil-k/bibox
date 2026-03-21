use anyhow::Result;
use std::path::Path;

/// Walk up from `start` to find a directory containing `.git`.
fn find_git_root(start: &Path) -> Option<std::path::PathBuf> {
    let mut current = start.to_path_buf();
    loop {
        if current.join(".git").exists() {
            return Some(current);
        }
        if !current.pop() {
            return None;
        }
    }
}

/// Auto-commit `db_path` into the git repo it lives in (if any).
/// Silently returns Ok(()) if not in a git repo.
/// Prints a warning (but still returns Ok) if git is unavailable or commit fails.
pub fn auto_commit(db_path: &Path, message: &str) -> Result<()> {
    let parent = match db_path.parent() {
        Some(p) => p.to_path_buf(),
        None => return Ok(()),
    };

    let repo_root = match find_git_root(&parent) {
        Some(r) => r,
        None => return Ok(()), // not in a git repo
    };

    // Compute path of db relative to repo root
    let db_relative = match db_path.strip_prefix(&repo_root) {
        Ok(rel) => rel.to_string_lossy().to_string(),
        Err(_) => db_path.to_string_lossy().to_string(),
    };

    // git add
    let add_status = std::process::Command::new("git")
        .args(["-C", &repo_root.to_string_lossy(), "add", &db_relative])
        .status();

    match add_status {
        Err(e) => {
            eprintln!("bibox: git not available, skipping auto-commit ({})", e);
            return Ok(());
        }
        Ok(s) if !s.success() => {
            eprintln!("bibox: git add failed, skipping auto-commit");
            return Ok(());
        }
        _ => {}
    }

    // git commit
    let commit_status = std::process::Command::new("git")
        .args(["-C", &repo_root.to_string_lossy(), "commit", "-m", message])
        .status();

    match commit_status {
        Err(e) => eprintln!("bibox: git commit failed: {}", e),
        Ok(s) if !s.success() => eprintln!("bibox: git commit failed (exit {})", s),
        _ => {}
    }

    Ok(())
}
