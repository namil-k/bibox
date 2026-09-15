//! git-sync 내장 플러그인. 저장할 때마다 커밋(`library/written`, `note/saved`), `g s` 동기화, `g t` 상태.
//! 포터블 홈이 git 저장소의 루트일 때만 동작한다. 그 밖에서는 이벤트는 조용하고 명령은 이유를 말한다.
//! 안 올린 커밋 수는 상태 바 조각 `ahead`로 민다.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

use crate::plugin::builtin::Builtin;
use crate::plugin::rpc::RpcError;
use crate::plugin::serve::{serve, Ui};

pub const MANIFEST: &str = r#"api = 2
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

[[fields]]
id = "ahead"
place = "status"
align = "right"
width = 12
desc = "Commits not yet pushed"

[[settings]]
key = "include_pdfs"
type = "bool"
default = false
desc = "Also commit pdfs/"

[[settings]]
key = "push_on_write"
type = "bool"
default = false
desc = "git push after every write"

[events]
subscribe = ["library/written", "note/saved"]
"#;

pub const GUIDE: &str = "No CLI. When the bibox home (`bibox config --json` -> `home`) is a git repository, every write through the CLI or the TUI is committed right away (`db.json` and notes; PDFs only if `include_pdfs = true`). To publish, run `git push` in the home directory, or set `push_on_write = true` under `[plugins.git-sync]` in config.toml. In the TUI, `g s` fetches, pulls with rebase and pushes; `g t` shows status; the status bar shows how many commits are not pushed yet. Nothing is committed when the home is not a git repository.";

pub const BUILTIN: Builtin = Builtin { name: "git-sync", manifest: MANIFEST, run, seeded: true, guide: GUIDE };

fn run() {
    let mut st = State::default();
    let mut handler = |method: &str, params: Value, ui: &mut Ui| -> Result<Value, RpcError> {
        if method == "initialize" {
            st.on_initialize(&params);
            return Ok(Value::Null);
        }
        st.handle(method, params, ui)
    };
    serve("git-sync", &mut handler);
}

const NEEDS_REPO: &str = "git-sync needs a portable home that is a git repository (bibox init <path>, then git init inside it)";

/// initialize에서 받은 paths.home과 config. `config/changed`가 config만 바꾼다.
#[derive(Default)]
struct State {
    home: Option<PathBuf>,
    include_pdfs: bool,
    push_on_write: bool,
}

impl State {
    fn on_initialize(&mut self, params: &Value) {
        self.home = params.pointer("/paths/home").and_then(Value::as_str).map(PathBuf::from);
        self.set_config(params.get("config").unwrap_or(&Value::Null));
    }

    fn set_config(&mut self, c: &Value) {
        let flag = |k: &str| c.get(k).and_then(Value::as_bool).unwrap_or(false);
        self.include_pdfs = flag("include_pdfs");
        self.push_on_write = flag("push_on_write");
    }

    fn handle(&mut self, method: &str, params: Value, ui: &mut Ui) -> Result<Value, RpcError> {
        match method {
            "config/changed" => {
                self.set_config(params.get("config").unwrap_or(&Value::Null));
                Ok(Value::Null)
            }
            "library/written" | "note/saved" => {
                // 저장소가 아니면 조용히. 실패는 메시지로만 알리고 응답은 없다(알림이라).
                let Ok(g) = repo(self.home.as_deref()) else { return Ok(Value::Null) };
                let message = written_message(method, &params);
                match stage_and_commit(&g, self.include_pdfs, &message) {
                    Ok(true) if self.push_on_write => {
                        if let Err(e) = g.ok(&["push", "-q"]) {
                            ui.message(&format!("git-sync: git push failed: {}", e));
                        }
                    }
                    Ok(_) => {}
                    Err(e) => ui.message(&format!("git-sync: git commit failed: {}", e)),
                }
                push_ahead(&g, ui);
                Ok(Value::Null)
            }
            "commands/run" => {
                let command = params.get("command").and_then(Value::as_str).unwrap_or("");
                let g = repo(self.home.as_deref()).map_err(RpcError::plugin)?;
                let message = match command {
                    "commit" => {
                        let committed = stage_and_commit(&g, self.include_pdfs, "bibox: update").map_err(|e| RpcError::plugin(format!("git commit failed: {}", e)))?;
                        if committed { "committed" } else { "nothing to commit" }.to_string()
                    }
                    "sync" => cmd_sync(&g, self.include_pdfs, ui).map_err(RpcError::plugin)?,
                    "status" => status_text(&g).map_err(RpcError::plugin)?,
                    other => return Err(RpcError::method_not_found(&format!("commands/run {}", other))),
                };
                push_ahead(&g, ui);
                Ok(serde_json::json!({"message": message}))
            }
            other => Err(RpcError::method_not_found(other)),
        }
    }
}

// ── 순수 함수 ────────────────────────────────────────────────────────────────

/// 이벤트 params에서 커밋 문장. `library/written`은 reason과 항목 키로, `note/saved`는 항목 키로.
fn written_message(method: &str, params: &Value) -> String {
    if method == "note/saved" {
        let key = params.pointer("/entry/bibtex_key").and_then(Value::as_str).unwrap_or("");
        return if key.is_empty() { "bibox: update".to_string() } else { format!("bibox: note {}", key) };
    }
    let reason = params.get("reason").and_then(Value::as_str).unwrap_or("other");
    let keys: Vec<String> = params
        .get("entries")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|e| e.get("bibtex_key").and_then(Value::as_str).map(str::to_string)).collect())
        .unwrap_or_default();
    commit_message(reason, &keys)
}

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

/// 범위 검사. home이 있고 그것이 저장소 루트여야 한다.
fn repo(home: Option<&Path>) -> Result<Git, String> {
    let Some(home) = home else {
        return Err(NEEDS_REPO.to_string());
    };
    let g = Git { home: home.to_path_buf() };
    let top = g.ok(&["rev-parse", "--show-toplevel"]).map_err(|_| NEEDS_REPO.to_string())?;
    let same = std::fs::canonicalize(&top).ok() == std::fs::canonicalize(home).ok();
    if !same {
        return Err(NEEDS_REPO.to_string());
    }
    Ok(g)
}

/// db.json, notes/(있으면), include_pdfs면 pdfs/(있으면)를 스테이지하고 변경이 있을 때만 커밋한다.
fn stage_and_commit(g: &Git, include_pdfs: bool, message: &str) -> Result<bool, String> {
    let mut paths: Vec<&str> = Vec::new();
    if g.home.join("db.json").is_file() {
        paths.push("db.json");
    }
    if g.home.join("notes").is_dir() {
        paths.push("notes");
    }
    if include_pdfs && g.home.join("pdfs").is_dir() {
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

/// upstream보다 앞선 커밋 수. 원격이 없으면 None.
fn ahead(g: &Git) -> Option<usize> {
    g.ok(&["rev-list", "--count", "@{upstream}..HEAD"]).ok().and_then(|s| s.parse().ok())
}

/// 상태 바 조각 `ahead`. 0이거나 원격이 없으면 빈칸(조각이 사라진다).
fn push_ahead(g: &Git, ui: &mut Ui) {
    let text = match ahead(g) {
        Some(n) if n > 0 => format!("↑{} unpushed", n),
        _ => String::new(),
    };
    ui.status("ahead", &text, None);
}

// ── 명령 ────────────────────────────────────────────────────────────────────

fn cmd_sync(g: &Git, include_pdfs: bool, ui: &mut Ui) -> Result<String, String> {
    ui.progress("committing");
    stage_and_commit(g, include_pdfs, "bibox: sync").map_err(|e| format!("git commit failed: {}", e))?;
    ui.progress("pulling");
    // upstream이 없으면 pull도 push도 안 되므로 여기서 한 번에 말한다.
    if g.ok(&["rev-parse", "--abbrev-ref", "@{upstream}"]).is_err() {
        let branch = g.ok(&["branch", "--show-current"]).unwrap_or_else(|_| "master".to_string());
        return Err(format!("no upstream branch; run `git push -u origin {}` once", branch));
    }
    // autostash: include_pdfs = false인 채 pdfs/를 지운 트리처럼 unstaged 변경이 남아 있어도 rebase가 거부하지 않는다
    g.ok(&["pull", "--rebase", "--autostash", "-q"]).map_err(|e| format!("git pull failed: {}", e))?;
    ui.progress("pushing");
    let n = ahead(g).unwrap_or(0);
    g.ok(&["push", "-q"]).map_err(|e| format!("git push failed: {}", e))?;
    ui.refresh();
    Ok(match n {
        0 => "up to date".to_string(),
        1 => "pushed 1 commit".to_string(),
        n => format!("pushed {} commits", n),
    })
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
    use std::collections::VecDeque;
    use std::io::Cursor;

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

    /// 핸들러를 부르고 (결과, 플러그인이 쓴 줄들)을 돌려준다. 읽을 줄은 없다(팝업을 안 쓰는 경로만).
    fn call(st: &mut State, method: &str, params: Value) -> (Result<Value, RpcError>, String) {
        let mut cur = Cursor::new(String::new());
        let mut out: Vec<u8> = Vec::new();
        let mut deferred = VecDeque::new();
        let mut next = 1;
        let r = {
            let mut ui = Ui::for_test(&mut cur, &mut out, &mut deferred, &mut next);
            st.handle(method, params, &mut ui)
        };
        (r, String::from_utf8(out).unwrap())
    }

    fn init(home: Option<&str>, config: Value) -> State {
        let mut st = State::default();
        st.on_initialize(&serde_json::json!({"paths": {"config_dir": "/c", "db": "/h/db.json", "notes": "/h/n", "pdfs": "/h/p", "home": home}, "config": config}));
        st
    }

    /// 임시 디렉토리에 git 저장소. 커밋할 수 있게 user를 준다.
    fn repo_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bibox-git-sync-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for args in [vec!["init", "-q"], vec!["config", "user.email", "t@t"], vec!["config", "user.name", "t"]] {
            assert!(Command::new("git").arg("-C").arg(&dir).args(&args).status().unwrap().success());
        }
        dir
    }

    #[test]
    fn initialize_reads_home_and_the_two_flags_and_config_changed_updates_them() {
        let st = init(Some("/h"), serde_json::json!({}));
        assert_eq!(st.home.as_deref(), Some(Path::new("/h")));
        assert!(!st.include_pdfs && !st.push_on_write);
        let mut st = init(None, serde_json::json!({"include_pdfs": true, "push_on_write": true, "zzz": 1}));
        assert!(st.home.is_none() && st.include_pdfs && st.push_on_write);
        let (r, out) = call(&mut st, "config/changed", serde_json::json!({"config": {"include_pdfs": false}}));
        assert!(r.is_ok() && out.is_empty());
        assert!(!st.include_pdfs && !st.push_on_write);
    }

    #[test]
    fn an_event_outside_a_repo_is_silent_and_a_command_explains() {
        let mut st = init(None, serde_json::json!({}));
        let (r, out) = call(&mut st, "library/written", serde_json::json!({"reason": "add", "entries": [{"bibtex_key": "a"}]}));
        assert_eq!(r.unwrap(), Value::Null);
        assert!(out.is_empty(), "no message, no status: {}", out);
        for cmd in ["commit", "status", "sync"] {
            let (r, _) = call(&mut st, "commands/run", serde_json::json!({"command": cmd, "trigger": "key"}));
            assert!(r.unwrap_err().message.contains("portable home"), "{}", cmd);
        }
        let (r, _) = call(&mut st, "nope/x", serde_json::json!({}));
        assert_eq!(r.unwrap_err().code, crate::plugin::rpc::METHOD_NOT_FOUND);
    }

    #[test]
    fn written_messages_come_from_reason_and_keys_and_notes_name_the_entry() {
        assert_eq!(written_message("library/written", &serde_json::json!({"reason": "import", "entries": [{"bibtex_key": "a"}, {"bibtex_key": "b"}]})), "bibox: import 2 entries");
        assert_eq!(written_message("library/written", &serde_json::json!({})), "bibox: update");
        assert_eq!(written_message("note/saved", &serde_json::json!({"entry": {"bibtex_key": "kim2025"}, "path": "/n/kim2025.md"})), "bibox: note kim2025");
    }

    /// 실제 저장소: written이 커밋하고, 원격이 없으니 ahead 조각은 빈칸으로 민다. 두 번째 written은 변경이 없어 커밋하지 않는다.
    #[test]
    fn a_written_event_commits_the_library_and_pushes_the_ahead_segment() {
        let dir = repo_dir("written");
        std::fs::write(dir.join("db.json"), "{}").unwrap();
        let mut st = init(dir.to_str(), serde_json::json!({}));
        let (r, out) = call(&mut st, "library/written", serde_json::json!({"reason": "add", "entries": [{"bibtex_key": "kim2025"}]}));
        assert!(r.is_ok());
        let g = Git { home: dir.clone() };
        assert_eq!(g.ok(&["log", "--format=%s"]).unwrap(), "bibox: add kim2025");
        assert!(out.contains("\"method\":\"status/set\"") && out.contains("\"field\":\"ahead\"") && out.contains("\"text\":\"\""), "{}", out);
        let (_, _) = call(&mut st, "library/written", serde_json::json!({"reason": "edit", "entries": []}));
        assert_eq!(g.ok(&["rev-list", "--count", "HEAD"]).unwrap(), "1", "nothing new to commit");
        let (r, _) = call(&mut st, "commands/run", serde_json::json!({"command": "commit", "trigger": "key"}));
        assert_eq!(r.unwrap()["message"], "nothing to commit");
        let (r, _) = call(&mut st, "commands/run", serde_json::json!({"command": "status", "trigger": "key"}));
        assert_eq!(r.unwrap()["message"], "no remote");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_manifest_declares_three_commands_a_status_field_and_two_events() {
        let mut problems = vec![];
        let m = crate::plugin::manifest::parse_manifest_with(
            std::path::Path::new("/tmp/git-sync"),
            "api = 2\nname = \"git-sync\"\nbuiltin = \"git-sync\"\n",
            &mut problems,
            &[BUILTIN],
        )
        .unwrap();
        assert!(problems.is_empty(), "{:?}", problems);
        assert_eq!(m.commands.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), vec!["commit", "sync", "status"]);
        assert_eq!(m.events, vec!["library/written", "note/saved"]);
        assert_eq!((m.fields[0].id.as_str(), m.fields[0].place.as_str()), ("ahead", "status"));
        assert_eq!(m.commands[1].key.as_ref().unwrap().len(), 2, "g s");
        assert_eq!(m.commands[2].key.as_ref().unwrap().len(), 2, "g t");
        assert!(m.commands[0].key.is_none(), "commit has no key");
    }
}
