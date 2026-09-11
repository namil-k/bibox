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

/// `auto_commit`과 같지만 경고를 출력하지 않고 돌려준다. TUI는 alternate screen 위라
/// stderr 출력이 화면을 깨뜨리므로 이쪽을 쓴다. git 저장소가 아니면 조용히 `Ok`.
pub fn auto_commit_quiet(db_path: &Path, message: &str) -> Result<(), String> {
    let parent = match db_path.parent() {
        Some(p) => p.to_path_buf(),
        None => return Ok(()),
    };
    let repo_root = match find_git_root(&parent) {
        Some(r) => r,
        None => return Ok(()),
    };
    let db_relative = match db_path.strip_prefix(&repo_root) {
        Ok(rel) => rel.to_string_lossy().to_string(),
        Err(_) => db_path.to_string_lossy().to_string(),
    };
    let root = repo_root.to_string_lossy().to_string();
    let add = std::process::Command::new("git").args(["-C", &root, "add", &db_relative]).output();
    match add {
        Err(e) => return Err(format!("git not available, skipping auto-commit ({})", e)),
        Ok(o) if !o.status.success() => return Err("git add failed, skipping auto-commit".to_string()),
        _ => {}
    }
    let commit = std::process::Command::new("git").args(["-C", &root, "commit", "-q", "-m", message]).output();
    match commit {
        Err(e) => Err(format!("git commit failed: {}", e)),
        Ok(o) if !o.status.success() => Err(format!("git commit failed (exit {})", o.status)),
        _ => Ok(()),
    }
}
