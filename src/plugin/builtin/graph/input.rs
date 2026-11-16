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
    let works = db
        .entries
        .iter()
        .map(|e| {
            let summary = note_text(notes_dir, &e.bibtex_key).and_then(|t| note_summary(&t, &boiler));
            let mut labels = e.collections.clone();
            labels.extend(e.tags.iter().map(|t| format!("#{}", t)));
            Work {
                id: e.bibtex_key.clone(),
                doi: entry_doi(e),
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
            Some(e) if entry_doi(e).is_none() => {
                input.notice = Some(format!("{} has no DOI, so it is not in the citation data", key));
            }
            None => input.notice = Some(format!("{} is not in the library", key)),
            // DOI가 있는 항목. 그래프에 실제로 있는지는 citers가 알려 준다(`focus_found`)
            Some(_) => input.focus = Some(key.to_string()),
        }
    }
    input
}

/// citers는 `finish()`에서만 DOI를 정규화한다. 라이브러리로 `Input`을 직접 만드는 쪽이 정규화해서 넘겨야
/// 캐시 키와 버전 병합이 맞는다. DOI가 아닌 문자열(`arXiv:...`)은 DOI 없음으로 본다.
fn entry_doi(e: &crate::models::Entry) -> Option<String> {
    e.doi.as_deref().and_then(citers::input::normalize_doi)
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
        assert_eq!(input.focus, None);
        assert_eq!(input.notice.as_deref(), Some("zzz is not in the library"));
    }

    #[test]
    fn dois_are_normalized_and_non_dois_dropped() {
        let dir = tempfile_dir("doinorm");
        let es = vec![
            entry("a", Some("10.1109/TVCG.2019.1")),
            entry("b", Some(" https://doi.org/10.1/B ")),
            entry("c", Some("doi:10.2/C")),
            entry("d", Some("arXiv:2101.00001")),
            entry("e", Some("  ")),
        ];
        let input = build_input(&db(es), &dir, None, None);
        let dois: Vec<Option<&str>> = input.works.iter().map(|w| w.doi.as_deref()).collect();
        assert_eq!(dois, vec![Some("10.1109/tvcg.2019.1"), Some("10.1/b"), Some("10.2/c"), None, None]);
        let input = build_input(&db(vec![entry("d", Some("arXiv:2101.00001"))]), &dir, None, Some("d"));
        assert_eq!(input.focus, None);
        assert_eq!(input.notice.as_deref(), Some("d has no DOI, so it is not in the citation data"));
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
