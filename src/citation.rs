//! 인용 문자열. 서지 관리 프로그램의 기본 기능이라 플러그인이 아니라 코어다.
//! 평문이며 이탤릭 같은 서식은 없다. 없는 필드는 그 조각을 통째로 뺀다.

use crate::models::Entry;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    Apa,
    Ieee,
    Chicago,
}

impl Style {
    pub fn all() -> [Style; 3] {
        [Style::Apa, Style::Ieee, Style::Chicago]
    }

    pub fn label(&self) -> &'static str {
        match self {
            Style::Apa => "APA",
            Style::Ieee => "IEEE",
            Style::Chicago => "Chicago",
        }
    }

    pub fn parse(s: &str) -> Option<Style> {
        match s.trim().to_ascii_lowercase().as_str() {
            "apa" => Some(Style::Apa),
            "ieee" => Some(Style::Ieee),
            "chicago" => Some(Style::Chicago),
            _ => None,
        }
    }
}

// ── 저자명 ──────────────────────────────────────────────────────────────────

struct Name {
    last: String,
    first: String,
}

/// 쉼표가 있으면 `Last, First`, 없으면 마지막 토큰이 성. "Plato"는 성만.
fn parse_name(s: &str) -> Name {
    let s = s.trim();
    if let Some((last, first)) = s.split_once(',') {
        return Name { last: last.trim().to_string(), first: first.trim().to_string() };
    }
    let parts: Vec<&str> = s.split_whitespace().collect();
    match parts.split_last() {
        Some((last, rest)) => Name { last: last.to_string(), first: rest.join(" ") },
        None => Name { last: String::new(), first: String::new() },
    }
}

/// "Jinho" -> "J.", "Jin Ho" -> "J. H.", "J.-H." -> "J.-H." (이미 이니셜이면 그대로).
fn initials(first: &str) -> String {
    first
        .split_whitespace()
        .map(|tok| {
            if tok.contains('.') {
                tok.to_string()
            } else {
                tok.chars().next().map(|c| format!("{}.", c)).unwrap_or_default()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn apa_name(n: &Name) -> String {
    if n.first.is_empty() { n.last.clone() } else { format!("{}, {}", n.last, initials(&n.first)) }
}

fn ieee_name(n: &Name) -> String {
    if n.first.is_empty() { n.last.clone() } else { format!("{} {}", initials(&n.first), n.last) }
}

fn chicago_first(n: &Name) -> String {
    if n.first.is_empty() { n.last.clone() } else { format!("{}, {}", n.last, n.first) }
}

fn chicago_rest(n: &Name) -> String {
    if n.first.is_empty() { n.last.clone() } else { format!("{} {}", n.first, n.last) }
}

/// 전부. 21명 이상이면 19명 + ... + 마지막.
fn authors_apa(names: &[Name]) -> String {
    let v: Vec<String> = names.iter().map(apa_name).collect();
    match v.len() {
        0 => String::new(),
        1 => v[0].clone(),
        2..=20 => format!("{}, & {}", v[..v.len() - 1].join(", "), v[v.len() - 1]),
        n => format!("{}, ... {}", v[..19].join(", "), v[n - 1]),
    }
}

/// 6명까지 전부, 7명부터 첫 저자 + et al.
fn authors_ieee(names: &[Name]) -> String {
    let v: Vec<String> = names.iter().map(ieee_name).collect();
    match v.len() {
        0 => String::new(),
        1 => v[0].clone(),
        2 => format!("{} and {}", v[0], v[1]),
        3..=6 => format!("{}, and {}", v[..v.len() - 1].join(", "), v[v.len() - 1]),
        _ => format!("{} et al.", v[0]),
    }
}

/// 첫 저자만 `Last, First`, 나머지는 `First Last`. 10명까지 전부, 11명부터 7명 + et al.
fn authors_chicago(names: &[Name]) -> String {
    let v: Vec<String> = names.iter().enumerate().map(|(i, n)| if i == 0 { chicago_first(n) } else { chicago_rest(n) }).collect();
    match v.len() {
        0 => String::new(),
        1 => v[0].clone(),
        2..=10 => format!("{}, and {}", v[..v.len() - 1].join(", "), v[v.len() - 1]),
        _ => format!("{}, et al.", v[..7].join(", ")),
    }
}

// ── 필드 ────────────────────────────────────────────────────────────────────

fn nonempty(o: &Option<String>) -> Option<&str> {
    o.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

fn pages(e: &Entry) -> Option<String> {
    nonempty(&e.pages).map(|p| p.replace("--", "-"))
}

fn year(e: &Entry) -> Option<String> {
    e.year.map(|y| y.to_string())
}

fn is_bare(e: &Entry) -> bool {
    e.author.is_empty() && nonempty(&e.title).is_none() && e.year.is_none()
}

// ── 형식 ────────────────────────────────────────────────────────────────────

pub fn format(entry: &Entry, style: Style) -> String {
    if is_bare(entry) {
        return entry.bibtex_key.clone();
    }
    let names: Vec<Name> = entry.author.iter().map(|a| parse_name(a)).collect();
    let raw = match style {
        Style::Apa => apa(entry, &names),
        Style::Ieee => ieee(entry, &names),
        Style::Chicago => chicago(entry, &names),
    };
    clean(&raw)
}

/// BibTeX의 대소문자 보호 중괄호(`{XR}`)와 LaTeX 표기(`Schr\"{o}dinger`)는 .bib 안의 것이다.
/// 붙여 넣을 인용에는 남지 않아야 한다. 이스케이프를 먼저 풀어야 `{\l}` 같은 것이 잡힌다.
fn clean(s: &str) -> String {
    crate::commands::strip_bibtex_braces(&crate::commands::decode_latex(s))
}

/// 여러 항목은 줄바꿈으로. IEEE만 `[n] ` 번호를 붙인다.
pub fn format_many(entries: &[&Entry], style: Style) -> String {
    entries
        .iter()
        .enumerate()
        .map(|(i, e)| match style {
            Style::Ieee => format!("[{}] {}", i + 1, format(e, style)),
            _ => format(e, style),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `Kim, J., & Park, S. (2025). Title. Journal, 51(3), 1-10. https://doi.org/x`
fn apa(e: &Entry, names: &[Name]) -> String {
    let mut s = String::new();
    let who = authors_apa(names);
    if !who.is_empty() {
        s.push_str(&who);
        s.push(' ');
    }
    s.push_str(&format!("({}).", year(e).unwrap_or_else(|| "n.d.".to_string())));
    if let Some(t) = nonempty(&e.title) {
        s.push_str(&format!(" {}.", t));
    }
    if let Some(j) = nonempty(&e.journal) {
        s.push_str(&format!(" {}", j));
        if let Some(v) = nonempty(&e.volume) {
            s.push_str(&format!(", {}", v));
        }
        if let Some(n) = nonempty(&e.number) {
            s.push_str(&format!("({})", n));
        }
        if let Some(p) = pages(e) {
            s.push_str(&format!(", {}", p));
        }
        s.push('.');
    } else if let Some(b) = nonempty(&e.booktitle) {
        s.push_str(&format!(" In {}.", b));
    } else if let Some(p) = nonempty(&e.publisher) {
        s.push_str(&format!(" {}.", p));
    }
    if let Some(d) = nonempty(&e.doi) {
        s.push_str(&format!(" https://doi.org/{}", d));
    }
    s
}

/// `J. Kim and S. Park, "Title," Journal, vol. 51, no. 3, pp. 1-10, 2025. doi: x`
fn ieee(e: &Entry, names: &[Name]) -> String {
    let mut parts: Vec<String> = Vec::new();
    let who = authors_ieee(names);
    if !who.is_empty() {
        parts.push(who);
    }
    if let Some(t) = nonempty(&e.title) {
        parts.push(format!("\"{},\"", t)); // IEEE는 쉼표가 따옴표 안에 들어간다
    }
    if let Some(j) = nonempty(&e.journal) {
        parts.push(j.to_string());
        if let Some(v) = nonempty(&e.volume) {
            parts.push(format!("vol. {}", v));
        }
        if let Some(n) = nonempty(&e.number) {
            parts.push(format!("no. {}", n));
        }
        if let Some(p) = pages(e) {
            parts.push(format!("pp. {}", p));
        }
    } else if let Some(b) = nonempty(&e.booktitle) {
        parts.push(format!("in {}", b));
    } else if let Some(p) = nonempty(&e.publisher) {
        parts.push(p.to_string());
    }
    if let Some(y) = year(e) {
        parts.push(y);
    }
    let mut s = String::new();
    for (i, p) in parts.iter().enumerate() {
        if i > 0 {
            if parts[i - 1].ends_with(",\"") {
                s.push(' ');
            } else {
                s.push_str(", ");
            }
        }
        s.push_str(p);
    }
    if s.ends_with(",\"") {
        // 제목이 마지막이면 쉼표 대신 마침표를 따옴표 안에
        s.truncate(s.len() - 2);
        s.push_str(".\"");
    } else {
        s.push('.');
    }
    if let Some(d) = nonempty(&e.doi) {
        s.push_str(&format!(" doi: {}", d));
    }
    s
}

/// `Kim, Jinho, and Sun Park. 2025. "Title." Journal 51 (3): 1-10. https://doi.org/x`
fn chicago(e: &Entry, names: &[Name]) -> String {
    let mut s = String::new();
    let who = authors_chicago(names);
    if !who.is_empty() {
        s.push_str(&who);
        // "et al."이나 이니셜로 끝나면 마침표가 이미 있다
        if !who.ends_with('.') {
            s.push('.');
        }
        s.push(' ');
    }
    s.push_str(&year(e).map(|y| format!("{}.", y)).unwrap_or_else(|| "n.d.".to_string()));
    if let Some(t) = nonempty(&e.title) {
        s.push_str(&format!(" \"{}.\"", t));
    }
    if let Some(j) = nonempty(&e.journal) {
        s.push_str(&format!(" {}", j));
        if let Some(v) = nonempty(&e.volume) {
            s.push_str(&format!(" {}", v));
        }
        if let Some(n) = nonempty(&e.number) {
            s.push_str(&format!(" ({})", n));
        }
        if let Some(p) = pages(e) {
            s.push_str(&format!(": {}", p));
        }
        s.push('.');
    } else if let Some(b) = nonempty(&e.booktitle) {
        s.push_str(&format!(" In {}.", b));
    } else if let Some(p) = nonempty(&e.publisher) {
        s.push_str(&format!(" {}.", p));
    }
    if let Some(d) = nonempty(&e.doi) {
        s.push_str(&format!(" https://doi.org/{}", d));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::EntryType;

    fn base(key: &str) -> Entry {
        Entry {
            id: "1".into(), bibtex_key: key.into(), entry_type: EntryType::Article,
            title: None, author: vec![], year: None, journal: None, volume: None, number: None,
            pages: None, publisher: None, editor: None, edition: None, isbn: None, booktitle: None,
            doi: None, url: None, abstract_text: None, tags: vec![], howpublished: None, month: None,
            note: None, collections: vec![], file_path: None, created_at: "2026-01-01 00:00:00".into(),
            updated_at: None,
        }
    }

    fn article() -> Entry {
        let mut e = base("kim2025rust");
        e.author = vec!["Kim, Jinho".into(), "Park, Sun".into()];
        e.year = Some(2025);
        e.title = Some("Rust systems programming".into());
        e.journal = Some("J. Sys".into());
        e.volume = Some("51".into());
        e.number = Some("3".into());
        e.pages = Some("1--10".into());
        e.doi = Some("10.1/x".into());
        e
    }

    fn book() -> Entry {
        let mut e = base("lee2020book");
        e.entry_type = EntryType::Book;
        e.author = vec!["Lee, Min".into()];
        e.year = Some(2020);
        e.title = Some("Book".into());
        e.publisher = Some("Pub".into());
        e
    }

    fn proceedings() -> Entry {
        let mut e = base("kim2024paper");
        e.entry_type = EntryType::InProceedings;
        e.author = vec!["Kim, Jinho".into()];
        e.year = Some(2024);
        e.title = Some("Paper".into());
        e.booktitle = Some("Proc. X".into());
        e.pages = Some("5--9".into());
        e
    }

    #[test]
    fn bibtex_braces_and_latex_escapes_never_reach_the_citation() {
        // 실제 라이브러리의 chen2024peapods 모양. 중괄호는 BibTeX의 대소문자 보호일 뿐이다
        let mut e = article();
        e.title = Some("{PEA-PODs}: Perceptual Evaluation in {XR} Displays".into());
        e.journal = Some("{ACM} Transactions".into());
        e.author = vec!["Schr\\\"{o}dinger, Erwin".into(), "{van der Berg}, Jan".into()];
        let apa = format(&e, Style::Apa);
        assert!(!apa.contains('{') && !apa.contains('}'), "{}", apa);
        assert!(apa.contains("PEA-PODs: Perceptual Evaluation in XR Displays"), "{}", apa);
        assert!(apa.contains("ACM Transactions"), "{}", apa);
        assert!(apa.starts_with("Schrödinger, E., & van der Berg, J."), "{}", apa);
        let ieee = format(&e, Style::Ieee);
        assert!(!ieee.contains('{'), "{}", ieee);
    }

    #[test]
    fn apa_article_book_and_proceedings() {
        assert_eq!(format(&article(), Style::Apa), "Kim, J., & Park, S. (2025). Rust systems programming. J. Sys, 51(3), 1-10. https://doi.org/10.1/x");
        assert_eq!(format(&book(), Style::Apa), "Lee, M. (2020). Book. Pub.");
        assert_eq!(format(&proceedings(), Style::Apa), "Kim, J. (2024). Paper. In Proc. X.");
    }

    #[test]
    fn ieee_article_book_and_proceedings() {
        assert_eq!(format(&article(), Style::Ieee), "J. Kim and S. Park, \"Rust systems programming,\" J. Sys, vol. 51, no. 3, pp. 1-10, 2025. doi: 10.1/x");
        assert_eq!(format(&book(), Style::Ieee), "M. Lee, \"Book,\" Pub, 2020.");
        assert_eq!(format(&proceedings(), Style::Ieee), "J. Kim, \"Paper,\" in Proc. X, 2024.");
    }

    #[test]
    fn chicago_article_book_and_proceedings() {
        assert_eq!(format(&article(), Style::Chicago), "Kim, Jinho, and Sun Park. 2025. \"Rust systems programming.\" J. Sys 51 (3): 1-10. https://doi.org/10.1/x");
        assert_eq!(format(&book(), Style::Chicago), "Lee, Min. 2020. \"Book.\" Pub.");
        assert_eq!(format(&proceedings(), Style::Chicago), "Kim, Jinho. 2024. \"Paper.\" In Proc. X.");
    }

    fn with_authors(n: usize) -> Entry {
        let mut e = base("many");
        e.author = (0..n).map(|i| format!("Last{}, First{}", i, i)).collect();
        e.year = Some(2025);
        e.title = Some("T".into());
        e
    }

    #[test]
    fn author_lists_follow_each_style_cutoff() {
        // APA: 21명 이상이면 19명 + ... + 마지막
        let s = format(&with_authors(21), Style::Apa);
        assert!(s.starts_with("Last0, F., Last1, F.,"));
        assert!(s.contains("Last18, F., ... Last20, F. (2025)"), "{}", s);
        assert!(!s.contains("Last19"), "{}", s);
        // APA: 3명은 전부, 마지막 앞에 &
        assert!(format(&with_authors(3), Style::Apa).starts_with("Last0, F., Last1, F., & Last2, F. (2025)"));
        // IEEE: 3~6명은 전부, 7명부터 et al.
        assert!(format(&with_authors(3), Style::Ieee).starts_with("F. Last0, F. Last1, and F. Last2, \"T,\""));
        assert!(format(&with_authors(7), Style::Ieee).starts_with("F. Last0 et al., \"T,\""));
        // Chicago: 2명은 "A, and B", 11명부터 7명 + et al.
        assert!(format(&with_authors(2), Style::Chicago).starts_with("Last0, First0, and First1 Last1. 2025."));
        let s = format(&with_authors(11), Style::Chicago);
        assert!(s.starts_with("Last0, First0, First1 Last1,"), "{}", s);
        assert!(s.contains("First6 Last6, et al. 2025."), "{}", s);
        assert!(!s.contains("Last7"), "{}", s);
    }

    #[test]
    fn names_in_first_last_order_and_bare_surnames_are_understood() {
        let mut e = base("x");
        e.author = vec!["Jinho Kim".into(), "Plato".into(), "Kim, J.-H.".into()];
        e.year = Some(2025);
        e.title = Some("T".into());
        assert!(format(&e, Style::Apa).starts_with("Kim, J., Plato, & Kim, J.-H. (2025)."), "{}", format(&e, Style::Apa));
        assert!(format(&e, Style::Ieee).starts_with("J. Kim, Plato, and J.-H. Kim, \"T,\""), "{}", format(&e, Style::Ieee));
    }

    #[test]
    fn missing_pieces_are_dropped_and_a_bare_entry_falls_back_to_the_citekey() {
        let mut e = base("solo");
        e.title = Some("Only a title".into());
        assert_eq!(format(&e, Style::Apa), "(n.d.). Only a title.");
        assert_eq!(format(&e, Style::Ieee), "\"Only a title.\"");
        assert_eq!(format(&e, Style::Chicago), "n.d. \"Only a title.\"");
        assert_eq!(format(&base("bare2026"), Style::Apa), "bare2026");
        assert_eq!(format(&base("bare2026"), Style::Ieee), "bare2026");
    }

    #[test]
    fn many_entries_join_with_newlines_and_ieee_numbers_them() {
        let a = article();
        let b = book();
        let ieee = format_many(&[&a, &b], Style::Ieee);
        assert!(ieee.starts_with("[1] J. Kim and S. Park,"), "{}", ieee);
        assert!(ieee.contains("\n[2] M. Lee,"), "{}", ieee);
        let apa = format_many(&[&a, &b], Style::Apa);
        assert_eq!(apa.lines().count(), 2);
        assert!(!apa.contains("[1]"));
    }

    #[test]
    fn styles_parse_case_insensitively() {
        assert_eq!(Style::parse("APA"), Some(Style::Apa));
        assert_eq!(Style::parse("ieee"), Some(Style::Ieee));
        assert_eq!(Style::parse("Chicago"), Some(Style::Chicago));
        assert_eq!(Style::parse("mla"), None);
        assert_eq!(Style::all().iter().map(|s| s.label()).collect::<Vec<_>>(), vec!["APA", "IEEE", "Chicago"]);
    }
}
