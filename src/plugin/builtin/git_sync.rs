//! git-sync 내장 플러그인. 저장할 때마다 커밋(`after_write`), `g s` 동기화, `g t` 상태.
//! 포터블 홈이 git 저장소의 루트일 때만 동작한다. 그 밖에서는 훅은 조용하고 명령은 이유를 말한다.

use std::path::PathBuf;
use std::process::{Command, Output};

use crate::plugin::builtin::Builtin;
use crate::plugin::protocol::{Final, Request};
use crate::plugin::serve::{serve, Ui};

pub const MANIFEST: &str = r#"api = 1
name = "git-sync"
description = "Commit the library on every write; fetch, pull and push on demand"

[[commands]]
id = "commit"
desc = "Commit db.json and notes"

[[commands]]
id = "sync"
desc = "Commit, pull --rebase and push the library"
key = ["g", "s"]

[[commands]]
id = "status"
desc = "Show the git status of the library"
key = ["g", "t"]

[[settings]]
key = "include_pdfs"
type = "bool"
default = false
desc = "Also commit pdfs/"

[[settings]]
key = "push_on_write"
type = "bool"
default = false
desc = "git push after every hook commit"

[[hooks]]
on = "after_write"
run = "commit"
"#;

pub const BUILTIN: Builtin = Builtin { name: "git-sync", manifest: MANIFEST, run, seeded: true };

fn run() {
    let mut handler = |req: &Request, ui: &mut Ui| handle(req, ui);
    serve(&mut handler);
}

const NEEDS_REPO: &str = "git-sync needs a portable home that is a git repository (bibox init <path>, then git init inside it)";

struct Cfg {
    include_pdfs: bool,
    push_on_write: bool,
}

fn cfg(req: &Request) -> Cfg {
    let c = &req.context.config;
    let flag = |k: &str| c.get(k).and_then(|v| v.as_bool()).unwrap_or(false);
    Cfg { include_pdfs: flag("include_pdfs"), push_on_write: flag("push_on_write") }
}

fn is_hook(req: &Request) -> bool {
    req.trigger.starts_with("hook:")
}

fn err(s: impl Into<String>) -> Final {
    Final { error: Some(s.into()), ..Default::default() }
}

fn msg(s: impl Into<String>) -> Final {
    Final { message: Some(s.into()), ..Default::default() }
}

// ── 순수 함수 ────────────────────────────────────────────────────────────────

/// 기존 auto-commit과 같은 문장. "bibox: add kim2025rust", "bibox: import 12 entries".
pub fn commit_message(reason: &str, keys: &[String]) -> String {
    match (reason, keys.len()) {
        ("undo", _) => "bibox: undo".to_string(),
        ("redo", _) => "bibox: redo".to_string(),
        ("import", n) => format!("bibox: import {} entries", n),
        ("add" | "edit" | "delete", 1) => format!("bibox: {} {}", reason, keys[0]),
        ("add" | "edit" | "delete", n) if n > 1 => format!("bibox: {} {} entries", reason, n),
        _ => "bibox: update".to_string(),
    }
}

/// git stderr에서 사람에게 보여줄 한 줄. `fatal:`/`error:` 우선, 없으면 마지막 줄.
/// 여러 줄 힌트의 마지막 줄을 잡으면 안 된다(git-push 예제에서 배운 것).
pub fn first_error_line(stderr: &str) -> String {
    let lines: Vec<&str> = stderr.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    lines
        .iter()
        .find(|l| l.starts_with("fatal:") || l.starts_with("error:"))
        .or(lines.last())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "unknown git error".to_string())
}

/// 옛 Settings의 Git 행 문자열 그대로.
pub fn format_status(remote: &str, branch: &str, dirty: bool, behind: usize) -> String {
    if dirty {
        format!("{} ● uncommitted changes", remote)
    } else if behind > 0 {
        format!("{}/{} ⚠ {} behind", remote, branch, behind)
    } else {
        format!("{}/{} ✓ up to date", remote, branch)
    }
}

fn hook_commit_message(req: &Request) -> String {
    let h = req.context.hook.as_ref();
    let reason = h.and_then(|h| h.get("reason")).and_then(|v| v.as_str()).unwrap_or("other");
    let keys: Vec<String> = h
        .and_then(|h| h.get("keys"))
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|k| k.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    commit_message(reason, &keys)
}

// ── git ─────────────────────────────────────────────────────────────────────

struct Git {
    home: PathBuf,
}

impl Git {
    fn raw(&self, args: &[&str]) -> Result<Output, String> {
        Command::new("git")
            .arg("-C")
            .arg(&self.home)
            .args(args)
            .output()
            .map_err(|e| format!("git not available: {}", e))
    }

    /// 성공이면 stdout(trim), 실패면 첫 오류 줄.
    fn ok(&self, args: &[&str]) -> Result<String, String> {
        let o = self.raw(args)?;
        if o.status.success() {
            Ok(String::from_utf8_lossy(&o.stdout).trim().to_string())
        } else {
            Err(first_error_line(&String::from_utf8_lossy(&o.stderr)))
        }
    }
}

/// 범위 검사. `paths.home`이 있고 그것이 저장소 루트여야 한다.
fn repo(req: &Request) -> Result<Git, String> {
    let Some(home) = req.context.paths.home.clone() else {
        return Err(NEEDS_REPO.to_string());
    };
    let g = Git { home: home.clone() };
    let top = g.ok(&["rev-parse", "--show-toplevel"]).map_err(|_| NEEDS_REPO.to_string())?;
    let same = std::fs::canonicalize(&top).ok() == std::fs::canonicalize(&home).ok();
    if !same {
        return Err(NEEDS_REPO.to_string());
    }
    Ok(g)
}

/// db.json, notes/(있으면), include_pdfs면 pdfs/(있으면)를 스테이지하고 변경이 있을 때만 커밋한다.
fn stage_and_commit(g: &Git, cfg: &Cfg, message: &str) -> Result<bool, String> {
    let mut paths: Vec<&str> = Vec::new();
    if g.home.join("db.json").is_file() {
        paths.push("db.json");
    }
    if g.home.join("notes").is_dir() {
        paths.push("notes");
    }
    if cfg.include_pdfs && g.home.join("pdfs").is_dir() {
        paths.push("pdfs");
    }
    if paths.is_empty() {
        return Ok(false);
    }
    let mut args = vec!["add", "-A", "--"];
    args.extend(paths);
    g.ok(&args)?;
    let staged = g.raw(&["diff", "--cached", "--quiet"])?;
    if staged.status.success() {
        return Ok(false);
    }
    g.ok(&["commit", "-q", "-m", message])?;
    Ok(true)
}

// ── 명령 ────────────────────────────────────────────────────────────────────

fn handle(req: &Request, ui: &mut Ui) -> Final {
    let c = cfg(req);
    match req.id.as_str() {
        "commit" => cmd_commit(req, &c),
        "sync" => cmd_sync(req, &c, ui),
        "status" => cmd_status(req),
        other => err(format!("unknown command {}", other)),
    }
}

fn cmd_commit(req: &Request, cfg: &Cfg) -> Final {
    let hook = is_hook(req);
    let g = match repo(req) {
        Ok(g) => g,
        Err(e) => return if hook { Final::default() } else { err(e) },
    };
    let message = if hook { hook_commit_message(req) } else { "bibox: update".to_string() };
    let committed = match stage_and_commit(&g, cfg, &message) {
        Ok(c) => c,
        Err(e) => return err(format!("git commit failed: {}", e)),
    };
    if hook && committed && cfg.push_on_write {
        if let Err(e) = g.ok(&["push", "-q"]) {
            return err(format!("git push failed: {}", e));
        }
    }
    if hook {
        Final::default()
    } else {
        msg(if committed { "committed" } else { "nothing to commit" })
    }
}

fn cmd_sync(req: &Request, cfg: &Cfg, ui: &mut Ui) -> Final {
    let g = match repo(req) {
        Ok(g) => g,
        Err(e) => return err(e),
    };
    ui.progress("committing");
    if let Err(e) = stage_and_commit(&g, cfg, "bibox: sync") {
        return err(format!("git commit failed: {}", e));
    }
    ui.progress("pulling");
    // upstream이 없으면 pull도 push도 안 되므로 여기서 한 번에 말한다.
    if g.ok(&["rev-parse", "--abbrev-ref", "@{upstream}"]).is_err() {
        let branch = g.ok(&["branch", "--show-current"]).unwrap_or_else(|_| "master".to_string());
        return err(format!("no upstream branch; run `git push -u origin {}` once", branch));
    }
    // autostash: include_pdfs = false인 채 pdfs/를 지운 트리처럼 unstaged 변경이 남아 있어도 rebase가 거부하지 않는다
    if let Err(e) = g.ok(&["pull", "--rebase", "--autostash", "-q"]) {
        return err(format!("git pull failed: {}", e));
    }
    ui.progress("pushing");
    let ahead: usize = g.ok(&["rev-list", "--count", "@{upstream}..HEAD"]).ok().and_then(|s| s.parse().ok()).unwrap_or(0);
    if let Err(e) = g.ok(&["push", "-q"]) {
        return err(format!("git push failed: {}", e));
    }
    let text = match ahead {
        0 => "up to date".to_string(),
        1 => "pushed 1 commit".to_string(),
        n => format!("pushed {} commits", n),
    };
    Final { message: Some(text), refresh: true, ..Default::default() }
}

fn cmd_status(req: &Request) -> Final {
    let g = match repo(req) {
        Ok(g) => g,
        Err(e) => return err(e),
    };
    match status_text(&g) {
        Ok(s) => msg(s),
        Err(e) => err(e),
    }
}

fn status_text(g: &Git) -> Result<String, String> {
    let remotes = g.ok(&["remote"])?;
    let remote = remotes.lines().next().unwrap_or("").trim().to_string();
    if remote.is_empty() {
        return Ok("no remote".to_string());
    }
    let dirty = !g.ok(&["status", "--porcelain"])?.is_empty();
    let _ = g.raw(&["fetch", "--quiet"]);
    let behind: usize = g.ok(&["rev-list", "--count", "HEAD..@{upstream}"]).ok().and_then(|s| s.parse().ok()).unwrap_or(0);
    let branch = g.ok(&["branch", "--show-current"]).unwrap_or_else(|_| "master".to_string());
    Ok(format_status(&remote, &branch, dirty, behind))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::protocol::{Context, Paths};

    fn k(keys: &[&str]) -> Vec<String> {
        keys.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn commit_messages_match_the_old_auto_commit_texts() {
        assert_eq!(commit_message("add", &k(&["kim2025"])), "bibox: add kim2025");
        assert_eq!(commit_message("edit", &k(&["kim2025"])), "bibox: edit kim2025");
        assert_eq!(commit_message("delete", &k(&["kim2025"])), "bibox: delete kim2025");
        assert_eq!(commit_message("edit", &k(&["a", "b"])), "bibox: edit 2 entries");
        assert_eq!(commit_message("import", &k(&["a", "b", "c"])), "bibox: import 3 entries");
        assert_eq!(commit_message("undo", &k(&[])), "bibox: undo");
        assert_eq!(commit_message("redo", &k(&["x"])), "bibox: redo");
        assert_eq!(commit_message("other", &k(&[])), "bibox: update");
        assert_eq!(commit_message("add", &k(&[])), "bibox: update");
    }

    #[test]
    fn first_error_line_prefers_fatal_then_error_then_the_last_line() {
        let s = "hint: something\nfatal: not a git repository\nhint: more";
        assert_eq!(first_error_line(s), "fatal: not a git repository");
        let s = "hint: a\nerror: failed to push some refs\nhint: b";
        assert_eq!(first_error_line(s), "error: failed to push some refs");
        let s = "To github.com:x/y.git\n ! [rejected] master -> master (fetch first)\n";
        assert_eq!(first_error_line(s), "! [rejected] master -> master (fetch first)");
        assert_eq!(first_error_line(""), "unknown git error");
    }

    #[test]
    fn status_strings_match_the_old_settings_row() {
        assert_eq!(format_status("origin", "master", true, 0), "origin ● uncommitted changes");
        assert_eq!(format_status("origin", "master", false, 2), "origin/master ⚠ 2 behind");
        assert_eq!(format_status("origin", "main", false, 0), "origin/main ✓ up to date");
    }

    fn request(id: &str, trigger: &str, home: Option<&str>, config: serde_json::Value, hook: Option<serde_json::Value>) -> Request {
        Request {
            r#type: "command".into(),
            id: id.into(),
            trigger: trigger.into(),
            context: Context {
                focus: None,
                collection: None,
                entry: None,
                entries: vec![],
                config,
                paths: Paths {
                    config_dir: "/c".into(),
                    db: "/h/db.json".into(),
                    notes: "/h/notes".into(),
                    pdfs: "/h/pdfs".into(),
                    home: home.map(std::path::PathBuf::from),
                },
                hook,
            },
            tab: None,
        }
    }

    #[test]
    fn config_reads_the_two_flags_and_defaults_to_false() {
        let r = request("commit", "key", None, serde_json::json!({}), None);
        let c = cfg(&r);
        assert!(!c.include_pdfs && !c.push_on_write);
        let r = request("commit", "key", None, serde_json::json!({"include_pdfs": true, "push_on_write": true, "zzz": 1}), None);
        let c = cfg(&r);
        assert!(c.include_pdfs && c.push_on_write);
    }

    #[test]
    fn a_hook_outside_a_repo_is_silent_and_a_command_explains() {
        let hook = request("commit", "hook:after_write", None, serde_json::json!({}), Some(serde_json::json!({"reason": "add", "keys": ["a"]})));
        assert_eq!(cmd_commit(&hook, &cfg(&hook)), Final::default());
        let manual = request("commit", "key", None, serde_json::json!({}), None);
        assert!(cmd_commit(&manual, &cfg(&manual)).error.unwrap().contains("portable home"));
        let status = request("status", "key", None, serde_json::json!({}), None);
        assert!(cmd_status(&status).error.unwrap().contains("portable home"));
    }

    #[test]
    fn hook_message_is_built_from_reason_and_keys() {
        let hook = serde_json::json!({"reason": "import", "keys": ["a", "b"]});
        let r = request("commit", "hook:after_write", None, serde_json::json!({}), Some(hook));
        assert_eq!(hook_commit_message(&r), "bibox: import 2 entries");
        let r = request("commit", "hook:after_write", None, serde_json::json!({}), None);
        assert_eq!(hook_commit_message(&r), "bibox: update");
    }

    #[test]
    fn the_manifest_declares_three_commands_and_one_hook() {
        let mut problems = vec![];
        let m = crate::plugin::manifest::parse_manifest_with(
            std::path::Path::new("/tmp/git-sync"),
            "api = 1\nname = \"git-sync\"\nbuiltin = \"git-sync\"\n",
            &mut problems,
            &[BUILTIN],
        )
        .unwrap();
        assert_eq!(m.commands.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), vec!["commit", "sync", "status"]);
        assert_eq!(m.hooks.len(), 1);
        assert_eq!(m.commands[1].key.as_ref().unwrap().len(), 2, "g s");
        assert_eq!(m.commands[2].key.as_ref().unwrap().len(), 2, "g t");
        assert!(m.commands[0].key.is_none(), "commit has no key");
    }
}
