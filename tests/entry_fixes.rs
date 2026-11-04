//! 항목 손질: citekey 바꾸기(`edit --key`, `--regen-key`)와 `doctor --fix`의 HTML 엔티티 디코드.
//! 실제 바이너리를 격리 HOME에서 돈다. 항목은 `import <file.json>`으로 넣는다(import는 제목을 손대지 않는다).

use std::path::{Path, PathBuf};
use std::process::Command;

struct Home {
    root: PathBuf,
}

impl Home {
    fn new(tag: &str) -> Home {
        let root = std::env::temp_dir().join(format!("bibox-entry-fixes-{}-{}", tag, std::process::id()));
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

    fn import(&self, entries: serde_json::Value) {
        let file = self.root.join("entries.json");
        std::fs::write(&file, entries.to_string()).unwrap();
        let out = self.bibox(&["import", file.to_str().unwrap(), "--json"]);
        assert!(out.status.success(), "import: {}", String::from_utf8_lossy(&out.stderr));
    }

    fn entry(&self, key: &str) -> Option<serde_json::Value> {
        let db: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(self.lib().join("db.json")).unwrap()).unwrap();
        db["entries"].as_array().unwrap().iter().find(|e| e["bibtex_key"] == key).cloned()
    }

    fn pdfs(&self) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(self.lib().join("pdfs"))
            .map(|d| d.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().to_string()).collect())
            .unwrap_or_default();
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

fn stderr(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).to_string()
}

// ── citekey 바꾸기 ──────────────────────────────────────────────────────────

fn kopper(h: &Home) {
    let pdf = write_pdf(&h.root.join("src"), "paper.pdf");
    h.import(serde_json::json!([
        {"bibtex_key": "kopper0000design", "entry_type": "inproceedings",
         "title": "Design and Evaluation of Navigation Techniques for Multiscale Virtual Environments",
         "author": ["Kopper, Regis", "Ni, Tao"], "year": 2006, "file": pdf},
        {"bibtex_key": "ni2006survey", "entry_type": "article", "title": "A Survey", "author": ["Ni, Tao"], "year": 2006}
    ]));
    std::fs::write(h.lib().join("notes").join("kopper0000design.md"), "my note\n").unwrap();
}

#[test]
fn edit_key_renames_the_entry_and_its_note_and_keeps_the_pdf_name_rule() {
    let h = Home::new("rename");
    kopper(&h);
    let pdf_before = h.pdfs();
    let out = h.bibox(&["edit", "kopper0000design", "--key", "kopper2006design"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(h.entry("kopper0000design").is_none());
    let e = h.entry("kopper2006design").expect("renamed entry");
    assert_eq!(e["file_path"], "Kopper_et_al_2006_Design and Evaluation of Navigation Techniques for Multiscale Virtual Environments.pdf");
    assert_eq!(h.pdfs(), pdf_before, "the PDF name comes from metadata, not the key");
    let notes = h.lib().join("notes");
    assert!(!notes.join("kopper0000design.md").exists());
    assert_eq!(std::fs::read_to_string(notes.join("kopper2006design.md")).unwrap(), "my note\n");
}

#[test]
fn edit_key_refuses_a_key_another_entry_has() {
    let h = Home::new("taken");
    kopper(&h);
    let out = h.bibox(&["edit", "kopper0000design", "--key", "ni2006survey"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("ni2006survey"), "{}", stderr(&out));
    assert!(h.entry("kopper0000design").is_some(), "nothing changed");
    assert!(h.lib().join("notes").join("kopper0000design.md").exists());
}

#[test]
fn edit_key_refuses_characters_a_citekey_cannot_hold() {
    let h = Home::new("badchars");
    kopper(&h);
    let out = h.bibox(&["edit", "kopper0000design", "--key", "kopper 2006/x"]);
    assert!(!out.status.success());
    assert!(h.entry("kopper0000design").is_some());
}

#[test]
fn regen_key_rebuilds_the_key_from_current_metadata() {
    let h = Home::new("regen");
    kopper(&h);
    let out = h.bibox(&["edit", "kopper0000design", "--regen-key"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(h.entry("kopper2006design").is_some(), "{}", String::from_utf8_lossy(&out.stdout));
    assert!(h.lib().join("notes").join("kopper2006design.md").exists());
}

#[test]
fn year_edit_and_regen_key_together_use_the_new_year() {
    let h = Home::new("yearregen");
    kopper(&h);
    let out = h.bibox(&["edit", "kopper0000design", "--year", "2007", "--regen-key"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let e = h.entry("kopper2007design").expect("key follows the edited year");
    assert!(e["file_path"].as_str().unwrap().starts_with("Kopper_et_al_2007_"));
}

#[test]
fn key_and_regen_key_cannot_be_combined() {
    let h = Home::new("both");
    kopper(&h);
    let out = h.bibox(&["edit", "kopper0000design", "--key", "x2006", "--regen-key"]);
    assert!(!out.status.success());
}

// ── doctor: HTML 엔티티 ─────────────────────────────────────────────────────

fn bozgeyikli(h: &Home) {
    let pdf = write_pdf(&h.root.join("src"), "paper.pdf");
    h.import(serde_json::json!([
        {"bibtex_key": "bozgeyikli2016point", "entry_type": "inproceedings",
         "title": "Point &amp; Teleport Locomotion Technique for Virtual Reality",
         "author": ["Bozgeyikli, Evren", "Raij, Andrew"], "year": 2016,
         "booktitle": "CHI PLAY &#39;16", "file": pdf}
    ]));
}

#[test]
fn doctor_reports_html_entities_as_fixable() {
    let h = Home::new("doctor-report");
    bozgeyikli(&h);
    let out = h.bibox(&["doctor", "--json"]);
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let issue = v["issues"].as_array().unwrap().iter().find(|i| i["kind"] == "html_entities").expect("html_entities issue");
    assert_eq!(issue["key"], "bozgeyikli2016point");
    assert_eq!(issue["fixable"], true);
    assert!(issue["detail"].as_str().unwrap().contains("title"), "{}", issue);
}

#[test]
fn doctor_lists_html_entities_in_its_human_output() {
    // The footer counted them while no section showed them ("22 fixable" over 20 lines).
    let h = Home::new("doctor-human");
    bozgeyikli(&h);
    let out = String::from_utf8_lossy(&h.bibox(&["doctor"]).stdout).to_string();
    assert!(out.contains("entries with HTML entities (1)"), "{}", out);
    assert!(out.contains("bozgeyikli2016point  title, booktitle"), "{}", out);
}

#[test]
fn doctor_fix_decodes_html_entities_and_renames_the_pdf() {
    let h = Home::new("doctor-fix");
    bozgeyikli(&h);
    assert_eq!(h.pdfs(), vec!["Bozgeyikli_et_al_2016_Point &amp; Teleport Locomotion Technique for Virtual Reality.pdf"]);
    let out = h.bibox(&["doctor", "--fix"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let e = h.entry("bozgeyikli2016point").unwrap();
    assert_eq!(e["title"], "Point & Teleport Locomotion Technique for Virtual Reality");
    assert_eq!(e["booktitle"], "CHI PLAY '16");
    let name = "Bozgeyikli_et_al_2016_Point & Teleport Locomotion Technique for Virtual Reality.pdf";
    assert_eq!(e["file_path"], name);
    assert_eq!(h.pdfs(), vec![name]);
    let again: serde_json::Value = serde_json::from_slice(&h.bibox(&["doctor", "--json"]).stdout).unwrap();
    assert!(!again["issues"].as_array().unwrap().iter().any(|i| i["kind"] == "html_entities"));
}

#[test]
fn doctor_fix_of_a_bad_citekey_keeps_the_pdf_name_rule() {
    let h = Home::new("doctor-badkey");
    let pdf = write_pdf(&h.root.join("src"), "paper.pdf");
    h.import(serde_json::json!([
        {"bibtex_key": "kim2025:deep", "entry_type": "article", "title": "Deep Things", "author": ["Kim, Namil"], "year": 2025, "file": pdf}
    ]));
    std::fs::write(h.lib().join("notes").join("kim2025:deep.md"), "n\n").unwrap();
    let out = h.bibox(&["doctor", "--fix"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let e = h.entry("kim2025deep").expect("sanitized key");
    assert_eq!(e["file_path"], "Kim_2025_Deep Things.pdf");
    assert_eq!(h.pdfs(), vec!["Kim_2025_Deep Things.pdf"]);
    assert!(h.lib().join("notes").join("kim2025deep.md").exists());
}

// ── citekey_format ──────────────────────────────────────────────────────────

fn set_citekey_format(h: &Home, fmt: &str) {
    let out = h.bibox(&["config", "--json"]);
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let path = PathBuf::from(v["config_path"].as_str().unwrap());
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let line = format!("citekey_format = \"{}\"", fmt);
    let text = if text.contains("citekey_format") {
        text.lines().map(|l| if l.trim_start().starts_with("citekey_format") { line.clone() } else { l.to_string() }).collect::<Vec<_>>().join("\n")
    } else {
        format!("{}\n{}\n", line, text)
    };
    std::fs::write(&path, text).unwrap();
}

#[test]
fn add_and_import_use_the_configured_citekey_format() {
    let h = Home::new("keyfmt");
    set_citekey_format(&h, "{author}_{year}");
    let out = h.bibox(&["add", "--title", "Rust Systems", "--author", "Kim, Namil", "--year", "2025", "--json"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(h.entry("kim_2025").is_some(), "add: {}", String::from_utf8_lossy(&out.stdout));
    h.import(serde_json::json!([{"entry_type": "article", "title": "Other Things", "author": ["Lee, Ann"], "year": 2024}]));
    assert!(h.entry("lee_2024").is_some(), "import");
}
