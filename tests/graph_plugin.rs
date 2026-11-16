//! graph 내장 플러그인을 실제 바이너리로 끝까지 돈다. 격리 HOME, 네트워크 없음.
//! 캐시 fixture는 citers 자신의 타입으로 만들어 써서 직렬화 모양이 어긋나지 않는다.
//! 라이브러리의 모든 DOI가 신선한 캐시(또는 기록된 miss)로 덮여 있어서 citers는 요청을 보내지 않는다.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

use citers::cache::{Cache, Cached};
use citers::openalex::OaWork;

struct Home {
    root: PathBuf,
}

impl Home {
    fn new(tag: &str) -> Home {
        let root = std::env::temp_dir().join(format!("bibox-graph-plugin-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let h = Home { root };
        let out = h.bibox(&["init", h.lib().to_str().unwrap()]);
        assert!(out.status.success(), "init: {}", String::from_utf8_lossy(&out.stderr));
        h
    }

    fn lib(&self) -> PathBuf {
        self.root.join("lib")
    }

    /// 플러그인 폴더. 설정 디렉터리는 HOME을 따라간다(macOS는 Library/Application Support).
    fn plugin_dir(&self) -> PathBuf {
        let base = if cfg!(target_os = "macos") { self.root.join("Library/Application Support") } else { self.root.join(".config") };
        base.join("bibox/plugins/graph")
    }

    fn bibox(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_bibox"))
            .args(args)
            .env("HOME", &self.root)
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("XDG_DATA_HOME")
            .env_remove("BIBOX_HOME")
            .output()
            .expect("run bibox")
    }

    fn ok(&self, args: &[&str]) -> String {
        let out = self.bibox(args);
        assert!(out.status.success(), "{:?}: {}", args, String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).into()
    }
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64
}

fn oa(id: &str, doi: &str, refs: &[&str]) -> OaWork {
    OaWork { id: id.into(), doi: Some(doi.into()), title: Some(format!("OA {}", id)), year: Some(2020), authors: vec!["Kim".into()], cited_by: 1, refs: refs.iter().map(|s| s.to_string()).collect() }
}

/// a(10.1/a)는 b(서재에는 `10.1/B`, 캐시 키는 소문자 `10.1/b`)를 인용한다. e는 OpenAlex가 b와 같은 논문(W2)으로 합친 프리프린트라, citers가 중복으로 보고 노드를 만들지 않는다. 그래서 e를 focus로 주면 "그래프에 없음"이 된다. c는 DOI가 없다. a, b, d는 서재 밖의 W9(10.1/x)도 인용해서 min_shared 3을
/// 채우므로 W9는 "bibox add --doi" 명령이 달린 서재 밖 노드로 그려진다. W9도 캐시에 있어 요청이 나가지 않는다.
fn prepare(tag: &str) -> Home {
    let h = Home::new(tag);
    let entries = serde_json::json!([
        {"bibtex_key": "a", "entry_type": "article", "title": "Paper A", "author": ["Kim, Namil"], "year": 2024, "doi": "10.1/a", "collections": ["ilda"]},
        {"bibtex_key": "d", "entry_type": "article", "title": "Paper D", "author": ["Choi, Ji"], "year": 2021, "doi": "10.1/d"},
        {"bibtex_key": "b", "entry_type": "article", "title": "Paper B", "author": ["Lee, Ann"], "year": 2020, "doi": "10.1/B"},
        {"bibtex_key": "e", "entry_type": "article", "title": "Paper E", "author": ["Seo, Hana"], "year": 2019, "doi": "10.1/e"},
        {"bibtex_key": "c", "entry_type": "misc", "title": "Paper C", "author": ["Park, Bo"], "year": 2022}
    ]);
    let file = h.root.join("entries.json");
    std::fs::write(&file, entries.to_string()).unwrap();
    h.ok(&["import", file.to_str().unwrap(), "--json"]);
    std::fs::create_dir_all(h.lib().join("notes")).unwrap();
    std::fs::write(h.lib().join("notes/a.md"), "## Summary\nS.\n").unwrap();

    h.ok(&["plugin", "install", "graph"]);
    let cache = Cache {
        works: HashMap::from([
        ("10.1/a".to_string(), Cached { work: oa("W1", "10.1/a", &["W2", "W9"]), fetched_at: now() }),
        ("10.1/b".to_string(), Cached { work: oa("W2", "10.1/b", &["W9"]), fetched_at: now() }),
        ("10.1/d".to_string(), Cached { work: oa("W3", "10.1/d", &["W9"]), fetched_at: now() }),
        ("10.1/e".to_string(), Cached { work: oa("W2", "10.1/b", &["W9"]), fetched_at: now() }),
        ]),
        outside: HashMap::from([("W9".to_string(), Cached { work: oa("W9", "10.1/x", &[]), fetched_at: now() })]),
        ..Cache::default()
    };
    cache.save(&h.plugin_dir().join("cache")).unwrap();
    h
}

fn snapshot(h: &Home) -> (String, String) {
    (std::fs::read_to_string(h.lib().join("db.json")).unwrap(), std::fs::read_to_string(h.lib().join("notes/a.md")).unwrap())
}

#[test]
fn graph_cli_opens_from_a_prefilled_cache() {
    let h = prepare("prefilled");
    let before = snapshot(&h);
    let out = h.bibox(&["graph", "a", "--no-open"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 1, "{}", stdout);
    let html_path = PathBuf::from(lines[0]);
    assert!(html_path.exists(), "{}", lines[0]);
    let html = std::fs::read_to_string(&html_path).unwrap();
    assert!(html.contains("\"summary\":\"S.\""), "no summary");
    assert!(html.contains("\"labels\":[\"ilda\"]"), "no labels");
    assert!(html.contains("bibox add --doi"), "no add command");
    assert!(!out.stderr.contains(&b'\r'), "no carriage returns on a non-terminal stderr");
    assert_eq!(snapshot(&h), before, "the library must not change");
    // 요청이 나갔다면 캐시에 새 항목이나 miss가 생겼을 것이다
    let cache = Cache::load(&h.plugin_dir().join("cache"));
    assert_eq!((cache.works.len(), cache.outside.len(), cache.misses.len()), (4, 1, 0));
}

#[test]
fn graph_cli_on_entry_without_doi_shows_the_notice() {
    let h = prepare("nodoi");
    let out = h.bibox(&["graph", "c", "--no-open"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let html = std::fs::read_to_string(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert!(html.contains("c has no DOI, so it is not in the citation data"));
}

#[test]
fn graph_is_not_installed_by_default() {
    let h = Home::new("default");
    let list = h.ok(&["plugin", "list"]);
    assert!(!list.contains("graph"), "{}", list);
    assert!(!h.plugin_dir().exists());
    let out = h.bibox(&["graph", "--no-open"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("No plugin named \"graph\""));
}

#[test]
fn graph_cli_focus_that_misses_the_data_warns() {
    let h = prepare("nofocus");
    let out = h.bibox(&["graph", "e", "--no-open"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Warning: e is not in the citation data"), "{}", String::from_utf8_lossy(&out.stderr));
    let out = h.bibox(&["graph", "a", "--no-open"]);
    assert!(!String::from_utf8_lossy(&out.stderr).contains("not in the citation data"));
}

#[test]
fn graph_cli_focus_on_a_key_not_in_the_library_shows_the_notice() {
    let h = prepare("nokey");
    let out = h.bibox(&["graph", "zzz", "--no-open"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let html = std::fs::read_to_string(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert!(html.contains("zzz is not in the library"));
}
