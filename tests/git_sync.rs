//! git-sync 내장 플러그인을 실제로 띄워 JSON-RPC로 말을 건다. 바이너리 전용 크레이트라 모듈은 못 가져오지만
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
    next_id: u64,
}

/// 요청 하나의 결과: 응답(result 또는 error 객체)과 그 사이에 온 알림들.
struct Reply {
    result: Option<serde_json::Value>,
    error: Option<serde_json::Value>,
    progress: Vec<String>,
    status: Vec<(String, String)>,
    messages: Vec<String>,
}

impl Plugin {
    /// 띄우고 initialize까지. `home`이 None이면 paths.home도 null.
    fn start(home: Option<&Path>, config: serde_json::Value) -> Plugin {
        let mut child = Command::new(env!("CARGO_BIN_EXE_bibox"))
            .args(["plugin", "run", "git-sync"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn bibox plugin run git-sync");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let mut p = Plugin { child, stdin, stdout, next_id: 1 };
        let h = home.map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from("/nonexistent"));
        let params = serde_json::json!({
            "protocol": 2, "bibox": "test",
            "paths": { "config_dir": "/tmp", "db": h.join("db.json"), "notes": h.join("notes"), "pdfs": h.join("pdfs"), "home": home },
            "config": config,
            "capabilities": { "images": false, "status_bar": true }
        });
        let r = p.call("initialize", params);
        assert_eq!(r.result.unwrap()["protocol"], 2);
        p
    }

    fn write(&mut self, v: &serde_json::Value) {
        self.stdin.write_all(v.to_string().as_bytes()).unwrap();
        self.stdin.write_all(b"\n").unwrap();
        self.stdin.flush().unwrap();
    }

    fn write_raw(&mut self, line: &str) {
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

    fn notify(&mut self, method: &str, params: serde_json::Value) {
        self.write(&serde_json::json!({"jsonrpc": "2.0", "method": method, "params": params}));
    }

    /// 요청 하나를 보내고 그 id의 응답을 받는다. 그 사이의 알림은 모으고, 플러그인의 요청(팝업)은 취소로 답한다.
    fn call(&mut self, method: &str, params: serde_json::Value) -> Reply {
        let id = self.next_id;
        self.next_id += 1;
        self.write(&serde_json::json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        let mut reply = Reply { result: None, error: None, progress: vec![], status: vec![], messages: vec![] };
        loop {
            let v = self.read();
            match (v.get("method").and_then(|m| m.as_str()), v.get("id")) {
                (Some("window/progress"), None) => reply.progress.push(v["params"]["text"].as_str().unwrap_or("").to_string()),
                (Some("status/set"), None) => reply.status.push((v["params"]["field"].as_str().unwrap_or("").to_string(), v["params"]["text"].as_str().unwrap_or("").to_string())),
                (Some("window/message"), None) => reply.messages.push(v["params"]["text"].as_str().unwrap_or("").to_string()),
                (Some("library/refresh"), None) => {}
                (Some("window/pick"), Some(rid)) => self.write(&serde_json::json!({"jsonrpc": "2.0", "id": rid, "result": {"index": null}})),
                (Some("window/prompt"), Some(rid)) => self.write(&serde_json::json!({"jsonrpc": "2.0", "id": rid, "result": {"text": null}})),
                (Some("window/confirm"), Some(rid)) => self.write(&serde_json::json!({"jsonrpc": "2.0", "id": rid, "result": {"yes": false}})),
                (Some(other), _) => panic!("unexpected method {}", other),
                (None, Some(rid)) => {
                    assert_eq!(rid.as_u64(), Some(id), "answers come back in order: {}", v);
                    reply.result = v.get("result").cloned();
                    reply.error = v.get("error").cloned();
                    return reply;
                }
                (None, None) => panic!("neither method nor id: {}", v),
            }
        }
    }

    /// 명령 하나. `commands/run`의 짧은 꼴.
    fn run(&mut self, command: &str) -> Reply {
        self.call("commands/run", serde_json::json!({"command": command, "trigger": "key", "entries": []}))
    }
}

impl Drop for Plugin {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn message(r: &Reply) -> String {
    r.result.as_ref().and_then(|v| v["message"].as_str()).map(str::to_string).unwrap_or_else(|| panic!("no message: {:?}", r.error))
}

fn error(r: &Reply) -> String {
    r.error.as_ref().and_then(|e| e["message"].as_str()).map(str::to_string).unwrap_or_else(|| panic!("no error: {:?}", r.result))
}

#[test]
fn a_written_event_commits_db_and_notes_with_the_reason_message() {
    let home = fresh_home("written");
    let mut p = Plugin::start(Some(&home), serde_json::json!({}));
    std::fs::write(home.join("notes/kim2025rust.md"), "# note\n").unwrap();
    p.notify("library/written", serde_json::json!({"reason": "add", "entries": [{"bibtex_key": "kim2025rust"}]}));
    // 알림은 답이 없다. 뒤에 보낸 요청의 답이 오면 앞의 알림은 처리된 것이다
    let r = p.run("status");
    assert_eq!(message(&r), "no remote");
    assert_eq!(git(&home, &["log", "-1", "--format=%s"]), "bibox: add kim2025rust");
    let files = git(&home, &["show", "--name-only", "--format=", "HEAD"]);
    assert!(files.contains("db.json") && files.contains("notes/kim2025rust.md"), "{}", files);
    assert!(r.status.iter().any(|(f, t)| f == "ahead" && t.is_empty()), "no upstream: the ahead segment is cleared: {:?}", r.status);
    assert!(r.messages.is_empty(), "silent on success: {:?}", r.messages);
}

#[test]
fn an_event_without_a_home_is_silent() {
    let mut p = Plugin::start(None, serde_json::json!({}));
    p.notify("library/written", serde_json::json!({"reason": "edit", "entries": []}));
    let r = p.run("status");
    assert!(error(&r).contains("portable home"));
    assert!(r.messages.is_empty() && r.status.is_empty(), "{:?} {:?}", r.messages, r.status);
}

#[test]
fn a_manual_commit_in_a_plain_directory_explains_what_it_needs() {
    let dir = std::env::temp_dir().join(format!("bibox-git-sync-plain-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut p = Plugin::start(Some(&dir), serde_json::json!({}));
    let r = p.run("commit");
    assert!(error(&r).contains("portable home"), "{:?}", r.error);
    assert_eq!(r.error.unwrap()["code"], -32000);
}

#[test]
fn status_reports_no_remote_on_a_fresh_repo() {
    let home = fresh_home("status");
    let mut p = Plugin::start(Some(&home), serde_json::json!({}));
    assert_eq!(message(&p.run("status")), "no remote");
}

#[test]
fn sync_without_an_upstream_fails_after_committing() {
    let home = fresh_home("sync");
    let mut p = Plugin::start(Some(&home), serde_json::json!({}));
    let r = p.run("sync");
    assert_eq!(r.progress, vec!["committing", "pulling"]);
    assert!(error(&r).contains("upstream"), "{:?}", r.error);
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
    let mut p = Plugin::start(Some(&home), serde_json::json!({}));
    // 커밋만 하면 ahead가 1이 되어 상태 조각에 보인다
    let r = p.run("commit");
    assert_eq!(message(&r), "committed");
    assert!(r.status.contains(&("ahead".to_string(), "↑1 unpushed".to_string())), "{:?}", r.status);
    let r = p.run("sync");
    assert_eq!(r.progress, vec!["committing", "pulling", "pushing"], "{:?}", r.error);
    assert_eq!(message(&r), "pushed 1 commit");
    assert!(r.status.contains(&("ahead".to_string(), String::new())), "pushed: the segment clears: {:?}", r.status);
    assert!(git(&home, &["status", "--short"]).contains("D pdfs/old.pdf"), "the deletion stays local and unstaged");
}

#[test]
fn pdfs_are_committed_only_when_include_pdfs_is_set() {
    let home = fresh_home("pdfs");
    std::fs::create_dir_all(home.join("pdfs")).unwrap();
    std::fs::write(home.join("pdfs/x.pdf"), b"%PDF").unwrap();
    let mut p = Plugin::start(Some(&home), serde_json::json!({}));
    // db.json도 새 파일이므로 첫 커밋은 있다. pdfs는 빠진다.
    assert_eq!(message(&p.run("commit")), "committed");
    assert!(!git(&home, &["show", "--name-only", "--format=", "HEAD"]).contains("pdfs/x.pdf"));
    // 다시 부르면 커밋할 것이 없다
    assert_eq!(message(&p.run("commit")), "nothing to commit");
    // include_pdfs가 config/changed로 켜지면 들어간다
    p.notify("config/changed", serde_json::json!({"config": {"include_pdfs": true}}));
    assert_eq!(message(&p.run("commit")), "committed");
    assert!(git(&home, &["show", "--name-only", "--format=", "HEAD"]).contains("pdfs/x.pdf"));
}

#[test]
fn a_bad_line_gets_a_parse_error_and_the_next_request_still_works() {
    let home = fresh_home("badline");
    let mut p = Plugin::start(Some(&home), serde_json::json!({}));
    p.write_raw("this is not json");
    let v = p.read();
    assert_eq!(v["error"]["code"], -32700, "{}", v);
    assert!(v["id"].is_null());
    assert_eq!(message(&p.run("status")), "no remote");
    let r = p.call("nope/x", serde_json::json!({}));
    assert_eq!(r.error.unwrap()["code"], -32601);
}

#[test]
fn shutdown_is_answered_and_ends_the_process() {
    let mut p = Plugin::start(None, serde_json::json!({}));
    let r = p.call("shutdown", serde_json::json!({}));
    assert!(r.result.is_some());
    let status = p.child.wait().unwrap();
    assert!(status.success());
}
