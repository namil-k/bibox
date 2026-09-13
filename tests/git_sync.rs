//! git-sync 내장 플러그인을 실제로 띄운다. 바이너리 전용 크레이트라 모듈은 못 가져오지만
//! `CARGO_BIN_EXE_bibox`로 자기 실행 파일은 띄울 수 있다. git이 PATH에 있어야 한다.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

fn git(home: &Path, args: &[&str]) -> String {
    let o = Command::new("git").arg("-C").arg(home).args(args).output().expect("git");
    assert!(o.status.success(), "git {:?} failed: {}", args, String::from_utf8_lossy(&o.stderr));
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

/// `git init`된 포터블 홈. db.json 하나와 notes/ 디렉토리.
fn fresh_home(tag: &str) -> PathBuf {
    let home = std::env::temp_dir().join(format!("bibox-git-sync-{}-{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join("notes")).unwrap();
    std::fs::write(home.join("db.json"), "{\"entries\":[]}\n").unwrap();
    git(&home, &["init", "-q"]);
    git(&home, &["config", "user.email", "test@example.com"]);
    git(&home, &["config", "user.name", "bibox test"]);
    git(&home, &["config", "commit.gpgsign", "false"]);
    home
}

struct Plugin {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Plugin {
    fn start() -> Plugin {
        let mut child = Command::new(env!("CARGO_BIN_EXE_bibox"))
            .args(["plugin", "run", "git-sync"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn bibox plugin run git-sync");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Plugin { child, stdin, stdout }
    }

    fn write(&mut self, line: &str) {
        self.stdin.write_all(line.as_bytes()).unwrap();
        self.stdin.write_all(b"\n").unwrap();
        self.stdin.flush().unwrap();
    }

    fn read(&mut self) -> serde_json::Value {
        let mut buf = String::new();
        let n = self.stdout.read_line(&mut buf).unwrap();
        assert!(n > 0, "plugin closed stdout");
        serde_json::from_str(buf.trim_end()).unwrap_or_else(|e| panic!("not json ({}): {}", e, buf))
    }

    /// 요청 하나를 보내고 최종 응답을 받는다. 그 사이의 ui 요청은 progress면 `{}`, 아니면 취소로 답한다.
    fn call(&mut self, request: &str) -> (Vec<String>, serde_json::Value) {
        self.write(request);
        let mut progress = Vec::new();
        loop {
            let v = self.read();
            match v.get("ui").and_then(|u| u.as_str()) {
                Some("progress") => {
                    progress.push(v["text"].as_str().unwrap_or("").to_string());
                    self.write("{}");
                }
                Some("pick") => self.write("{\"index\":null}"),
                Some("prompt") => self.write("{\"text\":null}"),
                Some("confirm") => self.write("{\"yes\":false}"),
                Some(other) => panic!("unexpected ui {}", other),
                None => return (progress, v),
            }
        }
    }
}

impl Drop for Plugin {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn request(id: &str, trigger: &str, home: Option<&Path>, config: serde_json::Value, hook: serde_json::Value) -> String {
    let h = home.map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from("/nonexistent"));
    serde_json::json!({
        "type": "command", "id": id, "trigger": trigger,
        "context": {
            "focus": null, "collection": null, "entry": null, "entries": [],
            "config": config,
            "paths": { "config_dir": "/tmp", "db": h.join("db.json"), "notes": h.join("notes"), "pdfs": h.join("pdfs"), "home": home },
            "hook": hook
        }
    })
    .to_string()
}

#[test]
fn a_write_hook_commits_db_and_notes_with_the_reason_message() {
    let home = fresh_home("hook");
    let mut p = Plugin::start();
    std::fs::write(home.join("notes/kim2025rust.md"), "# note\n").unwrap();
    let (_, f) = p.call(&request("commit", "hook:after_write", Some(&home), serde_json::json!({}), serde_json::json!({"reason": "add", "keys": ["kim2025rust"]})));
    assert_eq!(f, serde_json::json!({}), "hooks are silent on success");
    assert_eq!(git(&home, &["log", "-1", "--format=%s"]), "bibox: add kim2025rust");
    let files = git(&home, &["show", "--name-only", "--format=", "HEAD"]);
    assert!(files.contains("db.json") && files.contains("notes/kim2025rust.md"), "{}", files);
}

#[test]
fn a_hook_without_a_home_is_silent() {
    let mut p = Plugin::start();
    let (_, f) = p.call(&request("commit", "hook:after_write", None, serde_json::json!({}), serde_json::json!({"reason": "edit", "keys": []})));
    assert_eq!(f, serde_json::json!({}));
}

#[test]
fn a_manual_commit_in_a_plain_directory_explains_what_it_needs() {
    let dir = std::env::temp_dir().join(format!("bibox-git-sync-plain-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut p = Plugin::start();
    let (_, f) = p.call(&request("commit", "key", Some(&dir), serde_json::json!({}), serde_json::Value::Null));
    assert!(f["error"].as_str().unwrap().contains("portable home"), "{}", f);
}

#[test]
fn status_reports_no_remote_on_a_fresh_repo() {
    let home = fresh_home("status");
    let mut p = Plugin::start();
    let (_, f) = p.call(&request("status", "key", Some(&home), serde_json::json!({}), serde_json::Value::Null));
    assert_eq!(f["message"], "no remote");
}

#[test]
fn sync_without_an_upstream_fails_after_committing() {
    let home = fresh_home("sync");
    let mut p = Plugin::start();
    let (progress, f) = p.call(&request("sync", "key", Some(&home), serde_json::json!({}), serde_json::Value::Null));
    assert_eq!(progress, vec!["committing", "pulling"]);
    assert!(f["error"].as_str().unwrap().contains("upstream"), "{}", f);
    assert_eq!(git(&home, &["log", "-1", "--format=%s"]), "bibox: sync");
}

/// 0.3.5 사용자의 흔한 상태: pdfs/가 추적되는데 `include_pdfs = false`라 지운 PDF가 unstaged로 남는다.
/// `git pull --rebase`는 그런 트리를 거부하므로 autostash로 넘긴다.
#[test]
fn sync_survives_unstaged_changes_to_tracked_files() {
    let home = fresh_home("dirty");
    std::fs::create_dir_all(home.join("pdfs")).unwrap();
    std::fs::write(home.join("pdfs/old.pdf"), b"%PDF").unwrap();
    git(&home, &["add", "."]);
    git(&home, &["commit", "-qm", "init"]);
    let remote = home.with_extension("remote.git");
    let _ = std::fs::remove_dir_all(&remote);
    git(&home, &["init", "-q", "--bare", remote.to_str().unwrap()]);
    git(&home, &["remote", "add", "origin", remote.to_str().unwrap()]);
    git(&home, &["push", "-q", "-u", "origin", "HEAD"]);
    std::fs::remove_file(home.join("pdfs/old.pdf")).unwrap();
    std::fs::write(home.join("db.json"), "{\"entries\":[{}]}\n").unwrap();
    let mut p = Plugin::start();
    let (progress, f) = p.call(&request("sync", "key", Some(&home), serde_json::json!({}), serde_json::Value::Null));
    assert_eq!(progress, vec!["committing", "pulling", "pushing"], "{}", f);
    assert_eq!(f["message"], "pushed 1 commit", "{}", f);
    assert!(git(&home, &["status", "--short"]).contains("D pdfs/old.pdf"), "the deletion stays local and unstaged");
}

#[test]
fn pdfs_are_committed_only_when_include_pdfs_is_set() {
    let home = fresh_home("pdfs");
    std::fs::create_dir_all(home.join("pdfs")).unwrap();
    std::fs::write(home.join("pdfs/x.pdf"), b"%PDF").unwrap();
    let mut p = Plugin::start();
    // db.json도 새 파일이므로 첫 커밋은 있다. pdfs는 빠진다.
    let (_, f) = p.call(&request("commit", "key", Some(&home), serde_json::json!({}), serde_json::Value::Null));
    assert_eq!(f["message"], "committed");
    assert!(!git(&home, &["show", "--name-only", "--format=", "HEAD"]).contains("pdfs/x.pdf"));
    // 다시 부르면 커밋할 것이 없다
    let (_, f) = p.call(&request("commit", "key", Some(&home), serde_json::json!({}), serde_json::Value::Null));
    assert_eq!(f["message"], "nothing to commit");
    // include_pdfs면 들어간다
    let (_, f) = p.call(&request("commit", "key", Some(&home), serde_json::json!({"include_pdfs": true}), serde_json::Value::Null));
    assert_eq!(f["message"], "committed");
    assert!(git(&home, &["show", "--name-only", "--format=", "HEAD"]).contains("pdfs/x.pdf"));
}

#[test]
fn a_bad_line_gets_an_error_and_the_next_request_still_works() {
    let home = fresh_home("badline");
    let mut p = Plugin::start();
    p.write("this is not json");
    let v = p.read();
    assert!(v["error"].as_str().unwrap().contains("bad request"), "{}", v);
    let (_, f) = p.call(&request("status", "key", Some(&home), serde_json::json!({}), serde_json::Value::Null));
    assert_eq!(f["message"], "no remote");
}
