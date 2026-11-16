//! 서재를 citers의 입력으로 바꾼다. 항목 하나가 `Work` 하나.

use std::collections::HashSet;
use std::path::Path;

use citers::{Input, Work};

use super::summary::{note_summary, template_lines};
use crate::models::Database;

/// 모든 항목을 `Work`로 만든다. 노트(`notes_dir/<key>.md`)가 있으면 한 줄 요약을 붙이고,
/// 템플릿 문구는 `templates_dir`의 양식에서 걸러 낸다. DOI가 없는 항목은 그래프에 못 오르므로
/// focus로 지정되면 focus 대신 안내 문구를 남긴다.
pub fn build_input(db: &Database, notes_dir: &Path, templates_dir: Option<&Path>, focus: Option<&str>) -> Input {
    let boiler: HashSet<String> = templates_dir.map(template_lines).unwrap_or_default();
    let has_doi = |doi: &Option<String>| doi.as_deref().is_some_and(|d| !d.trim().is_empty());
    let works = db
        .entries
        .iter()
        .map(|e| {
            let summary = note_text(notes_dir, &e.bibtex_key).and_then(|t| note_summary(&t, &boiler));
            let mut labels = e.collections.clone();
            labels.extend(e.tags.iter().map(|t| format!("#{}", t)));
            Work {
                id: e.bibtex_key.clone(),
                doi: e.doi.clone().filter(|d| !d.trim().is_empty()),
                title: e.title.clone(),
                authors: e.author.clone(),
                year: e.year.map(|y| y as i32),
                summary: summary.as_ref().map(|(_, s)| s.clone()),
                summary_label: summary.map(|(l, _)| l.to_string()),
                labels,
            }
        })
        .collect();

    let mut input = Input { works, focus: None, add_command: Some("bibox add --doi {doi}".to_string()), notice: None };
    if let Some(key) = focus {
        match db.entries.iter().find(|e| e.bibtex_key == key) {
            Some(e) if !has_doi(&e.doi) => {
                input.notice = Some(format!("{} has no DOI, so it is not in the citation data", key));
            }
            // 없는 key도 그대로 넘긴다. "not in the graph" 처리는 citers가 한다
            _ => input.focus = Some(key.to_string()),
        }
    }
    input
}

fn note_text(notes_dir: &Path, key: &str) -> Option<String> {
    if key.is_empty() || key.contains(['/', '\\']) || key.contains("..") {
        return None;
    }
    std::fs::read_to_string(notes_dir.join(format!("{}.md", key))).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Entry, EntryType};

    fn entry(key: &str, doi: Option<&str>) -> Entry {
        Entry {
            id: key.into(), bibtex_key: key.into(), entry_type: EntryType::Article,
            title: Some(format!("Title {}", key)), author: vec!["Kim, J.".into()], year: Some(2006),
            journal: None, volume: None, number: None, pages: None, publisher: None, editor: None,
            edition: None, isbn: None, booktitle: None, doi: doi.map(str::to_string), url: None, abstract_text: None,
            tags: vec![], howpublished: None, month: None, note: None, collections: vec![],
            file_path: None, created_at: "2026-01-01 00:00:00".into(), updated_at: None,
        }
    }

    fn db(entries: Vec<Entry>) -> Database {
        Database { entries, merge_log: vec![] }
    }

    #[test]
    fn entries_become_works_with_labels_summary_and_add_command() {
        let dir = tempfile_dir("works");
        std::fs::write(dir.join("a.md"), "## Summary\nS.\n").unwrap();
        let mut a = entry("a", Some("10.1/a"));
        a.collections = vec!["ilda".into()];
        a.tags = vec!["vr".into()];
        let input = build_input(&db(vec![a, entry("b", None)]), &dir, None, None);
        assert_eq!(input.works.len(), 2);
        let w = &input.works[0];
        assert_eq!(w.id, "a");
        assert_eq!(w.doi.as_deref(), Some("10.1/a"));
        assert_eq!(w.title.as_deref(), Some("Title a"));
        assert_eq!(w.authors, vec!["Kim, J."]);
        assert_eq!(w.year, Some(2006));
        assert_eq!(w.labels, vec!["ilda", "#vr"]);
        assert_eq!(w.summary.as_deref(), Some("S."));
        assert_eq!(w.summary_label.as_deref(), Some("Summary"));
        assert_eq!(input.works[1].summary, None);
        assert_eq!(input.add_command.as_deref(), Some("bibox add --doi {doi}"));
        assert_eq!(input.focus, None);
        assert_eq!(input.notice, None);
    }

    #[test]
    fn entry_without_doi_gets_a_notice() {
        let dir = tempfile_dir("nodoi");
        let input = build_input(&db(vec![entry("a", Some("10.1/a")), entry("b", None)]), &dir, None, Some("b"));
        assert_eq!(input.focus, None);
        assert_eq!(input.notice.as_deref(), Some("b has no DOI, so it is not in the citation data"));
        let input = build_input(&db(vec![entry("a", Some("10.1/a"))]), &dir, None, Some("a"));
        assert_eq!(input.focus.as_deref(), Some("a"));
        let input = build_input(&db(vec![entry("a", Some("10.1/a"))]), &dir, None, Some("zzz"));
        assert_eq!(input.focus.as_deref(), Some("zzz"));
    }

    #[test]
    fn missing_notes_dir_is_fine() {
        let input = build_input(&db(vec![entry("a", Some("10.1/a"))]), Path::new("/definitely/not/here"), Some(Path::new("/nor/here")), None);
        assert_eq!(input.works.len(), 1);
        assert!(input.works.iter().all(|w| w.summary.is_none() && w.summary_label.is_none()));
    }

    fn tempfile_dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("bibox-graph-input-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }
}
