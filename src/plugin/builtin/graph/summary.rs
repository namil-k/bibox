//! 노트 한 줄 요약. 논문 노트에서 "이 논문이 무엇인가"를 한 문장으로 뽑는다.

use std::collections::HashSet;
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;

static META: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z_][A-Za-z_ -]{0,20}:\s").unwrap());
static CITE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)DOI[:\s]|\(\d{4}[a-z]?\)\.").unwrap());
static BOLD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\*\*(.+?)\*\*").unwrap());
static CODE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"`(.+?)`").unwrap());
static END_HEAD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^#{1,2}\s").unwrap());
static ONE_LINE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^#{2,}\s*(\d+\.\s*)?(한 줄 정의)\b").unwrap());
static SUMMARY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^#{2,}\s*(\d+\.\s*)?(Summary|요약)\b").unwrap());

/// 템플릿 폴더의 `*.md`에서 비어 있지 않고 `#`로 시작하지 않는 줄(trim)을 모은다.
/// 노트에 그대로 남은 템플릿 문구를 걸러내는 데 쓴다. 폴더가 없으면 빈 집합.
#[cfg_attr(not(test), allow(dead_code))]
pub fn template_lines(templates_dir: &Path) -> HashSet<String> {
    let mut out = HashSet::new();
    let Ok(rd) = std::fs::read_dir(templates_dir) else { return out };
    for entry in rd.flatten() {
        let p = entry.path();
        if p.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&p) else { continue };
        for l in text.lines() {
            let t = l.trim();
            if !t.is_empty() && !t.starts_with('#') {
                out.insert(t.to_string());
            }
        }
    }
    out
}

fn clean(t: &str) -> String {
    let t = BOLD.replace_all(t, "$1");
    CODE.replace_all(&t, "$1").trim().to_string()
}

fn is_prose(s: &str) -> bool {
    // `*`는 글머리표(`* x`)일 때만 거른다. `**굵게**`로 시작하는 문장은 산문이다.
    let bullet = s == "*" || s.starts_with("* ");
    !s.is_empty() && !bullet && !s.starts_with(['#', '-', '>', '|']) && !META.is_match(s)
}

fn paragraphs(lines: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    let mut para: Vec<&str> = Vec::new();
    for l in lines {
        let s = l.trim();
        if is_prose(s) {
            para.push(s);
        } else if !para.is_empty() {
            out.push(para.join(" "));
            para.clear();
        }
    }
    if !para.is_empty() {
        out.push(para.join(" "));
    }
    out
}

fn first_para(lines: &[&str]) -> Option<String> {
    paragraphs(lines).into_iter().find(|p| !CITE.is_match(p)).map(|p| clean(&p))
}

fn section(lines: &[&str], head: &Regex) -> Option<String> {
    let i = lines.iter().position(|l| head.is_match(l))?;
    let rest: Vec<&str> = lines[i + 1..].iter().copied().take_while(|m| !END_HEAD.is_match(m)).collect();
    first_para(&rest)
}

/// (라벨, 문장). 한 줄 정의 절 -> Summary/요약 절 -> 노트 전체의 첫 산문 문단 순.
#[cfg_attr(not(test), allow(dead_code))]
pub fn note_summary(note: &str, boiler: &HashSet<String>) -> Option<(&'static str, String)> {
    let lines: Vec<&str> = note.lines().filter(|l| !boiler.contains(l.trim())).collect();
    if let Some(s) = section(&lines, &ONE_LINE) {
        return Some(("In one line", s));
    }
    if let Some(s) = section(&lines, &SUMMARY) {
        return Some(("Summary", s));
    }
    first_para(&lines).map(|s| ("Note", s))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none() -> HashSet<String> {
        HashSet::new()
    }

    #[test]
    fn one_line_section_wins() {
        let n = "# T\ncitekey: x\n\n## 1. 한 줄 정의\n\n**선호**는 A.\n\n## 2. 기여\n본문";
        assert_eq!(note_summary(n, &none()), Some(("In one line", "선호는 A.".to_string())));
    }

    #[test]
    fn summary_section_then_first_paragraph() {
        let r = note_summary("## Summary\nComprehensive review.", &none()).unwrap();
        assert_eq!(r, ("Summary", "Comprehensive review.".to_string()));
        let r = note_summary("doi: 10.1/x\nyear: 2020\n\nReal prose here.", &none()).unwrap();
        assert_eq!(r, ("Note", "Real prose here.".to_string()));
    }

    #[test]
    fn citation_paragraphs_and_metadata_are_skipped() {
        let n = "Curcio, C. A. (1990). J. DOI: 10.1/x\n\ncone 분포 근거.";
        assert_eq!(note_summary(n, &none()), Some(("Note", "cone 분포 근거.".to_string())));
    }

    #[test]
    fn template_only_note_has_no_summary() {
        let line = "이 논문이 한 일을 한 문장으로. 저자의 Abstract를 옮기지 말고 내 말로.";
        let boiler: HashSet<String> = [line.to_string()].into();
        let n = format!("# T\n\n## 1. 한 줄 정의\n{line}\n");
        assert_eq!(note_summary(&n, &boiler), None);
    }

    #[test]
    fn template_lines_ignores_headings_and_missing_dir() {
        let d = std::env::temp_dir().join(format!("bibox-graph-tpl-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("a.md"), "# H\n\n- x\n본문 줄\n").unwrap();
        let got = template_lines(&d);
        let _ = std::fs::remove_dir_all(&d);
        let want: HashSet<String> = ["- x".to_string(), "본문 줄".to_string()].into();
        assert_eq!(got, want);
        assert!(template_lines(&d).is_empty());
    }
}
