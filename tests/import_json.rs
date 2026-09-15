//! `bibox import <file.json>`을 실제 바이너리로 돈다. 격리 HOME에 `bibox init`으로 홈을 만든다.

use std::path::{Path, PathBuf};
use std::process::Command;

struct Home {
    root: PathBuf,
}

impl Home {
    fn new(tag: &str) -> Home {
        let root = std::env::temp_dir().join(format!("bibox-import-json-{}-{}", tag, std::process::id()));
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

    fn bibox(&self, args: &[&str]) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_bibox"))
            .args(args)
            .env("HOME", &self.root)
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("XDG_DATA_HOME")
            .output()
            .expect("run bibox")
    }

    fn db(&self) -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(self.lib().join("db.json")).unwrap()).unwrap()
    }

    fn pdfs(&self) -> Vec<String> {
        let dir = self.lib().join("pdfs");
        let mut v: Vec<String> = std::fs::read_dir(&dir).map(|d| d.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().to_string()).collect()).unwrap_or_default();
        v.sort();
        v
    }
}

fn write_pdf(dir: &Path, name: &str) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let p = dir.join(name);
    std::fs::write(&p, b"%PDF-1.4\n%%EOF\n").unwrap();
    p
}

fn entries_json(pdf: &Path) -> String {
    serde_json::json!([
        {"bibtex_key": "kim2025", "entry_type": "article", "title": "Fast Bibliographies", "author": ["Kim, Namil"], "year": 2025,
         "journal": "J. Refs", "doi": "10.1000/abc", "abstract": "Abs", "tags": ["fast"], "collections": ["ML/NLP"], "file": pdf},
        {"bibtex_key": "lee2024", "entry_type": "misc", "title": "Slow Citations", "author": ["Lee, Ann"], "year": 2024,
         "file": "/nonexistent/missing.pdf"}
    ])
    .to_string()
}

fn outcomes(out: &std::process::Output) -> Vec<serde_json::Value> {
    let text = String::from_utf8_lossy(&out.stdout);
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("stdout is not a JSON array ({}):\n{}\nstderr: {}", e, text, String::from_utf8_lossy(&out.stderr)))
}

#[test]
fn json_import_adds_entries_copies_pdfs_and_reports_per_entry() {
    let h = Home::new("adds");
    let pdf = write_pdf(&h.root.join("src"), "paper.pdf");
    let file = h.root.join("entries.json");
    std::fs::write(&file, entries_json(&pdf)).unwrap();
    let out = h.bibox(&["import", file.to_str().unwrap(), "--to", "zotero", "--json"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let o = outcomes(&out);
    assert_eq!(o.len(), 2);
    assert_eq!((o[0]["status"].as_str(), o[0]["key"].as_str()), (Some("added"), Some("kim2025")));
    assert_eq!(o[1]["status"], "added");
    assert!(o[1]["reason"].as_str().unwrap().contains("file not found"), "{}", o[1]);
    let db = h.db();
    let entries = db["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    let kim = entries.iter().find(|e| e["bibtex_key"] == "kim2025").unwrap();
    assert_eq!(kim["collections"], serde_json::json!(["ML/NLP", "zotero"]));
    assert_eq!(kim["tags"], serde_json::json!(["fast"]));
    assert_eq!(kim["abstract"], "Abs");
    assert_eq!(kim["file_path"], "Kim_2025_Fast Bibliographies.pdf");
    assert_eq!(h.pdfs(), vec!["Kim_2025_Fast Bibliographies.pdf"]);
    assert!(pdf.is_file(), "copied, not moved");
    let lee = entries.iter().find(|e| e["bibtex_key"] == "lee2024").unwrap();
    assert!(lee["file_path"].is_null());
}

#[test]
fn rerun_merges_by_doi_and_is_idempotent() {
    let h = Home::new("rerun");
    let pdf = write_pdf(&h.root.join("src"), "paper.pdf");
    let file = h.root.join("entries.json");
    std::fs::write(&file, entries_json(&pdf)).unwrap();
    assert!(h.bibox(&["import", file.to_str().unwrap(), "--json"]).status.success());
    let before = std::fs::read_to_string(h.lib().join("db.json")).unwrap();
    let out = h.bibox(&["import", file.to_str().unwrap(), "--json"]);
    let o = outcomes(&out);
    assert_eq!(o[0]["status"], "skipped", "same DOI, same PDF already attached: {}", o[0]);
    assert_eq!(o[0]["key"], "kim2025");
    assert_eq!(o[1]["status"], "skipped", "no DOI: matched by title and year: {}", o[1]);
    assert_eq!(h.db()["entries"].as_array().unwrap().len(), 2);
    assert_eq!(h.pdfs().len(), 1, "no second copy");
    assert_eq!(std::fs::read_to_string(h.lib().join("db.json")).unwrap(), before, "nothing rewritten");
}

#[test]
fn dry_run_writes_nothing_but_still_reports() {
    let h = Home::new("dry");
    let pdf = write_pdf(&h.root.join("src"), "paper.pdf");
    let file = h.root.join("entries.json");
    std::fs::write(&file, entries_json(&pdf)).unwrap();
    let before = std::fs::read_to_string(h.lib().join("db.json")).unwrap();
    let out = h.bibox(&["import", file.to_str().unwrap(), "--json", "--dry-run"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let o = outcomes(&out);
    assert_eq!(o.len(), 2);
    assert_eq!(o[0]["status"], "added");
    assert_eq!(std::fs::read_to_string(h.lib().join("db.json")).unwrap(), before);
    assert!(h.pdfs().is_empty());
    // 사람용 출력도 dry run을 말한다
    let out = h.bibox(&["import", file.to_str().unwrap(), "--dry-run"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.to_lowercase().contains("dry run"), "{}", text);
    assert_eq!(std::fs::read_to_string(h.lib().join("db.json")).unwrap(), before);
}

#[test]
fn bib_import_supports_json_output_too() {
    let h = Home::new("bib");
    let file = h.root.join("refs.bib");
    std::fs::write(&file, "@article{park2023,\n title={Braces},\n author={Park, Min},\n year={2023},\n journal={J},\n doi={10.1/p}\n}\n").unwrap();
    let out = h.bibox(&["import", file.to_str().unwrap(), "--json"]);
    let o = outcomes(&out);
    assert_eq!(o.len(), 1);
    assert_eq!((o[0]["status"].as_str(), o[0]["key"].as_str()), (Some("added"), Some("park2023")));
}

#[test]
fn a_non_array_json_file_is_an_error() {
    let h = Home::new("bad");
    let file = h.root.join("bad.json");
    std::fs::write(&file, "{\"title\": \"x\"}").unwrap();
    let out = h.bibox(&["import", file.to_str().unwrap()]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("array"));
}
