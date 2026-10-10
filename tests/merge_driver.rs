//! 실제 git 두 클론으로 pull해서 `bibox merge-db`가 db.json을 합치는지 본다. git이 PATH에 있어야 한다.

use std::path::{Path, PathBuf};
use std::process::Command;

/// 사용자의 전역·시스템 git 설정이 결과를 바꾸지 못하게 끊는다. 신원은 저장소별 설정으로 준다.
fn isolated(mut c: Command) -> Command {
    c.env("GIT_CONFIG_GLOBAL", "/dev/null").env("GIT_CONFIG_NOSYSTEM", "1");
    c
}

fn git(dir: &Path, args: &[&str]) -> std::process::Output {
    let mut c = isolated(Command::new("git"));
    c.arg("-C").arg(dir).args(args).output().expect("git")
}

fn ok(dir: &Path, args: &[&str]) -> String {
    let o = git(dir, args);
    assert!(o.status.success(), "git {:?} failed: {}", args, String::from_utf8_lossy(&o.stderr));
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

fn entry(id: &str, key: &str, title: &str, year: u32, at: Option<&str>) -> serde_json::Value {
    serde_json::json!({
        "id": id, "bibtex_key": key, "entry_type": "article", "title": title, "author": ["Kim, J."], "year": year,
        "journal": null, "volume": null, "number": null, "pages": null, "publisher": null, "editor": null, "edition": null,
        "isbn": null, "booktitle": null, "doi": null, "url": null, "abstract": null, "tags": [], "howpublished": null,
        "month": null, "note": null, "collections": [], "file_path": null, "created_at": "2026-01-01 00:00:00", "updated_at": at
    })
}

fn write_db(dir: &Path, entries: Vec<serde_json::Value>) {
    let s = serde_json::to_string_pretty(&serde_json::json!({ "entries": entries })).unwrap();
    std::fs::write(dir.join("db.json"), s).unwrap();
}

fn read_db(dir: &Path) -> serde_json::Value {
    let s = std::fs::read_to_string(dir.join("db.json")).unwrap();
    serde_json::from_str(&s).unwrap_or_else(|e| panic!("db.json is not JSON ({}):\n{}", e, s))
}

fn clone_as(remote: &Path, dir: &Path, name: &str, register: bool) {
    ok(remote.parent().unwrap(), &["clone", "-q", remote.to_str().unwrap(), dir.to_str().unwrap()]);
    ok(dir, &["config", "user.email", &format!("{}@example.com", name)]);
    ok(dir, &["config", "user.name", name]);
    ok(dir, &["config", "commit.gpgsign", "false"]);
    if register {
        register_by_hand(dir);
    }
}

/// bibox가 하는 등록과 같은 것: 이 클론만 읽는 info/attributes 줄과 git 설정. 커밋되는 파일은 없다.
fn register_by_hand(dir: &Path) {
    let attributes = dir.join(ok(dir, &["rev-parse", "--git-path", "info/attributes"]));
    std::fs::create_dir_all(attributes.parent().unwrap()).unwrap();
    std::fs::write(&attributes, "db.json merge=bibox\n").unwrap();
    let exe = env!("CARGO_BIN_EXE_bibox");
    ok(dir, &["config", "merge.bibox.name", "bibox entry-level merge"]);
    ok(dir, &["config", "merge.bibox.driver", &format!("'{}' merge-db %O %A %B", exe)]);
}

/// bare 원격, 초기 커밋(항목 둘, .gitattributes는 없다), 두 클론 mac과 server.
fn setup(tag: &str, register_mac: bool) -> (PathBuf, PathBuf) {
    let root = std::env::temp_dir().join(format!("bibox-merge-driver-{}-{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let remote = root.join("remote.git");
    ok(&root, &["init", "-q", "--bare", remote.to_str().unwrap()]);
    let seed = root.join("seed");
    clone_as(&remote, &seed, "seed", false);
    write_db(&seed, vec![entry("1", "kim2020a", "Alpha", 2020, None), entry("2", "lee2021b", "Beta", 2021, None)]);
    ok(&seed, &["add", "."]);
    ok(&seed, &["commit", "-qm", "init"]);
    ok(&seed, &["push", "-q", "origin", "HEAD"]);
    let (mac, server) = (root.join("mac"), root.join("server"));
    clone_as(&remote, &mac, "mac", register_mac);
    clone_as(&remote, &server, "server", true);
    (mac, server)
}

fn commit_all(dir: &Path, msg: &str) {
    ok(dir, &["commit", "-qam", msg]);
}

#[test]
fn different_entries_merge_cleanly_with_pull_rebase() {
    let (mac, server) = setup("rebase", true);
    write_db(&server, vec![entry("1", "kim2020a", "Alpha", 2020, None), entry("2", "lee2021b", "Beta edited on server", 2021, Some("2026-10-10T08:05:00+00:00"))]);
    commit_all(&server, "server edit");
    ok(&server, &["push", "-q"]);
    write_db(&mac, vec![entry("1", "kim2020a", "Alpha edited on mac", 2020, Some("2026-10-10T10:00:00+02:00")), entry("2", "lee2021b", "Beta", 2021, None)]);
    commit_all(&mac, "mac edit");
    ok(&mac, &["pull", "--rebase", "-q"]);
    let db = read_db(&mac);
    assert_eq!(db["entries"][0]["title"], "Alpha edited on mac");
    assert_eq!(db["entries"][1]["title"], "Beta edited on server");
    assert!(db.get("merge_log").is_none(), "no conflicts, no log: {}", db);
}

#[test]
fn the_same_field_takes_the_newer_value_with_a_plain_pull_too() {
    let (mac, server) = setup("merge", true);
    write_db(&server, vec![entry("1", "kim2020a", "Deep learning.", 2020, Some("2026-10-10T08:05:00+00:00")), entry("2", "lee2021b", "Beta", 2021, None)]);
    commit_all(&server, "server edit");
    ok(&server, &["push", "-q"]);
    write_db(&mac, vec![entry("1", "kim2020a", "Deep Learning", 2020, Some("2026-10-10T10:00:00+02:00")), entry("2", "lee2021b", "Beta", 2021, None)]);
    commit_all(&mac, "mac edit");
    ok(&mac, &["pull", "--no-rebase", "-q", "--no-edit"]);
    let db = read_db(&mac);
    assert_eq!(db["entries"][0]["title"], "Deep learning.");
    assert_eq!(db["merge_log"][0]["dropped"], "Deep Learning");
    assert_eq!(db["merge_log"][0]["rule"], "newer");
}

#[test]
fn an_unregistered_clone_still_gets_a_plain_text_conflict() {
    let (mac, server) = setup("unregistered", false);
    write_db(&server, vec![entry("1", "kim2020a", "Server", 2020, None), entry("2", "lee2021b", "Beta", 2021, None)]);
    commit_all(&server, "server edit");
    ok(&server, &["push", "-q"]);
    write_db(&mac, vec![entry("1", "kim2020a", "Mac", 2020, None), entry("2", "lee2021b", "Beta", 2021, None)]);
    commit_all(&mac, "mac edit");
    let o = git(&mac, &["pull", "--rebase", "-q"]);
    assert!(!o.status.success(), "no driver on this machine: git's own text merge conflicts as before");
    let _ = git(&mac, &["rebase", "--abort"]);
}

fn no_gitattributes_anywhere(dirs: &[&Path]) {
    for d in dirs {
        assert!(!d.join(".gitattributes").exists(), "{} has a .gitattributes", d.display());
        assert!(!ok(d, &["ls-files"]).contains(".gitattributes"));
        assert!(ok(d, &["log", "--all", "--format=", "--name-only"]).lines().all(|l| l != ".gitattributes"));
    }
}

/// 둘 다 info/attributes로만 등록한 채 같은 칸을 고쳤다. 커밋된 .gitattributes가 없어도 pull --rebase가 도우미로 합친다.
#[test]
fn info_attributes_alone_lets_pull_rebase_merge_the_same_field() {
    let (mac, server) = setup("infoattr", true);
    no_gitattributes_anywhere(&[&mac, &server]);
    write_db(&server, vec![entry("1", "kim2020a", "Server title", 2020, Some("2026-10-10T08:05:00+00:00")), entry("2", "lee2021b", "Beta", 2021, None)]);
    commit_all(&server, "server edit");
    ok(&server, &["push", "-q"]);
    write_db(&mac, vec![entry("1", "kim2020a", "Mac title", 2020, Some("2026-10-10T10:10:00+02:00")), entry("2", "lee2021b", "Beta", 2021, None)]);
    commit_all(&mac, "mac edit");
    ok(&mac, &["pull", "--rebase", "-q"]);
    let db = read_db(&mac);
    assert_eq!(db["entries"][0]["title"], "Mac title", "08:10Z is newer than 08:05Z");
    assert_eq!(db["merge_log"][0]["dropped"], "Server title");
    no_gitattributes_anywhere(&[&mac]);
}

/// 등록된 기계에서 도우미가 실패하면(한쪽 항목이 깨짐) 등록 안 된 기계처럼 충돌 표시가 남고 pull이 멈춘다.
#[test]
fn a_failed_driver_leaves_conflict_markers_like_an_unregistered_clone() {
    let (mac, server) = setup("failed", true);
    let mut broken = entry("1", "kim2020a", "Server", 2020, None);
    broken.as_object_mut().unwrap().remove("author");
    write_db(&server, vec![broken, entry("2", "lee2021b", "Beta", 2021, None)]);
    commit_all(&server, "server edit with a broken entry");
    ok(&server, &["push", "-q"]);
    write_db(&mac, vec![entry("1", "kim2020a", "Mac", 2020, None), entry("2", "lee2021b", "Beta", 2021, None)]);
    commit_all(&mac, "mac edit");
    let o = git(&mac, &["pull", "--no-rebase", "-q", "--no-edit"]);
    assert!(!o.status.success(), "the merge must stop");
    assert!(String::from_utf8_lossy(&o.stderr).contains("bibox merge-db"), "the driver ran: {}", String::from_utf8_lossy(&o.stderr));
    let s = std::fs::read_to_string(mac.join("db.json")).unwrap();
    assert!(s.contains("<<<<<<< ours") && s.contains(">>>>>>> theirs"), "{}", s);
    assert!(ok(&mac, &["diff", "--name-only", "--diff-filter=U"]).contains("db.json"));
}

/// git hook처럼 GIT_DIR가 다른 저장소를 가리킨 채 bibox가 돌아도, 등록은 라이브러리 저장소에만 한다.
#[test]
fn an_exported_git_dir_does_not_redirect_registration() {
    let root = std::env::temp_dir().join(format!("bibox-merge-driver-gitdir-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (home, lib, other) = (root.join("home"), root.join("lib"), root.join("other"));
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&other).unwrap();
    let bibox = |args: &[&str], git_dir: Option<&Path>| {
        let mut c = isolated(Command::new(env!("CARGO_BIN_EXE_bibox")));
        c.args(args).env("HOME", &home).env_remove("XDG_CONFIG_HOME").env_remove("XDG_DATA_HOME");
        if let Some(g) = git_dir {
            c.env("GIT_DIR", g);
        }
        let o = c.output().unwrap();
        assert!(o.status.success(), "bibox {:?}: {}", args, String::from_utf8_lossy(&o.stderr));
    };
    bibox(&["init", lib.to_str().unwrap()], None);
    ok(&lib, &["init", "-q"]);
    ok(&other, &["init", "-q"]);
    bibox(&["list"], Some(&other.join(".git")));
    assert!(git(&other, &["config", "--get", "merge.bibox.driver"]).stdout.is_empty(), "the other repository is untouched");
    assert!(!other.join(".git/info/attributes").exists() || !std::fs::read_to_string(other.join(".git/info/attributes")).unwrap().contains("merge=bibox"));
    assert!(ok(&lib, &["config", "--get", "merge.bibox.driver"]).contains("merge-db"), "the library is registered");
}

/// 읽기만 하는 명령 하나로 이 기계에 등록된다. 작업 트리에는 아무것도 생기지 않는다.
#[test]
fn any_bibox_command_registers_the_driver_on_this_machine() {
    let root = std::env::temp_dir().join(format!("bibox-merge-driver-register-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (home, lib) = (root.join("home"), root.join("lib"));
    std::fs::create_dir_all(&home).unwrap();
    let bibox = |args: &[&str]| {
        let mut c = isolated(Command::new(env!("CARGO_BIN_EXE_bibox")));
        let o = c.args(args).env("HOME", &home).env_remove("XDG_CONFIG_HOME").env_remove("XDG_DATA_HOME").output().unwrap();
        assert!(o.status.success(), "bibox {:?}: {}", args, String::from_utf8_lossy(&o.stderr));
    };
    bibox(&["init", lib.to_str().unwrap()]);
    ok(&lib, &["init", "-q"]);
    assert!(git(&lib, &["config", "--get", "merge.bibox.driver"]).stdout.is_empty(), "not registered yet");
    bibox(&["list"]);
    let attributes = std::fs::read_to_string(lib.join(".git/info/attributes")).unwrap();
    assert!(attributes.lines().any(|l| l == "db.json merge=bibox"), "{}", attributes);
    assert_eq!(ok(&lib, &["config", "--get", "merge.bibox.driver"]), format!("'{}' merge-db %O %A %B", env!("CARGO_BIN_EXE_bibox")));
    assert_eq!(ok(&lib, &["config", "--get", "merge.bibox.name"]), "bibox entry-level merge");
    assert!(ok(&lib, &["status", "--porcelain", "--ignored"]).lines().all(|l| !l.contains(".gitattributes")));
    assert!(!lib.join(".gitattributes").exists());
}
