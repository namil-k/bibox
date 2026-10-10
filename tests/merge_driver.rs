//! 실제 git 두 클론으로 pull해서 `bibox merge-db`가 db.json을 합치는지 본다. git이 PATH에 있어야 한다.

use std::path::{Path, PathBuf};
use std::process::Command;

fn git(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new("git").arg("-C").arg(dir).args(args).output().expect("git")
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
        let exe = env!("CARGO_BIN_EXE_bibox");
        ok(dir, &["config", "merge.bibox.name", "bibox entry-level merge"]);
        ok(dir, &["config", "merge.bibox.driver", &format!("'{}' merge-db %O %A %B", exe)]);
    }
}

/// bare 원격, 초기 커밋(항목 둘 + .gitattributes), 두 클론 mac과 server.
fn setup(tag: &str, register_mac: bool) -> (PathBuf, PathBuf) {
    let root = std::env::temp_dir().join(format!("bibox-merge-driver-{}-{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let remote = root.join("remote.git");
    ok(&root, &["init", "-q", "--bare", remote.to_str().unwrap()]);
    let seed = root.join("seed");
    clone_as(&remote, &seed, "seed", false);
    write_db(&seed, vec![entry("1", "kim2020a", "Alpha", 2020, None), entry("2", "lee2021b", "Beta", 2021, None)]);
    std::fs::write(seed.join(".gitattributes"), "db.json merge=bibox\n").unwrap();
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
