//! Settings 화면의 항목 자료. 코어 설정과 플러그인 설정을 같은 `Item`으로 표현한다.
//! 화면은 이 목록을 절로 걸러 그리고 검색은 글자로 거른다. TUI 없이 테스트된다.

use std::path::PathBuf;

use crate::config::{expand_tilde, Config, LineNumbers, CITEKEY_PRESETS};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    General,
    Appearance,
    Export,
    Plugins,
}

impl Section {
    pub const ALL: [Section; 4] = [Section::General, Section::Appearance, Section::Export, Section::Plugins];

    pub fn label(self) -> &'static str {
        match self {
            Section::General => "General",
            Section::Appearance => "Appearance",
            Section::Export => "Export",
            Section::Plugins => "Plugins",
        }
    }
}

/// 행의 종류. 표시와 h/l/Enter 동작을 정한다.
#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Bool { on: &'static str, off: &'static str },
    Choice(Vec<String>),
    Int,
    Str,
    /// h/l은 프리셋 순환(비어 있으면 무동작), Enter는 디렉토리 선택기.
    /// `optional`이면 "없음"이 순환의 첫 자리다.
    Path { presets: Vec<PathBuf>, optional: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Core {
    Home,
    PdfStorage,
    CitekeyFormat,
    Language,
    LineNumbers,
    PanelRatio,
    ScrollDirection,
    StatusBar,
    BibExportDir,
    ExportDir,
}

impl Core {
    pub const ALL: [Core; 10] = [
        Core::Home,
        Core::PdfStorage,
        Core::CitekeyFormat,
        Core::Language,
        Core::LineNumbers,
        Core::PanelRatio,
        Core::ScrollDirection,
        Core::StatusBar,
        Core::BibExportDir,
        Core::ExportDir,
    ];

    fn section(self) -> Section {
        match self {
            Core::Home | Core::PdfStorage | Core::CitekeyFormat | Core::Language => Section::General,
            Core::LineNumbers | Core::PanelRatio | Core::ScrollDirection | Core::StatusBar => Section::Appearance,
            Core::BibExportDir | Core::ExportDir => Section::Export,
        }
    }

    fn id(self) -> &'static str {
        match self {
            Core::Home => "general.home",
            Core::PdfStorage => "general.pdf_storage",
            Core::CitekeyFormat => "general.citekey_format",
            Core::Language => "general.language",
            Core::LineNumbers => "appearance.line_numbers",
            Core::PanelRatio => "appearance.panel_ratio",
            Core::ScrollDirection => "appearance.scroll_direction",
            Core::StatusBar => "appearance.status_bar",
            Core::BibExportDir => "export.bib_export_dir",
            Core::ExportDir => "export.export_dir",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Core::Home => "Home",
            Core::PdfStorage => "PDF storage",
            Core::CitekeyFormat => "Citekey format",
            Core::Language => "Language",
            Core::LineNumbers => "Line numbers",
            Core::PanelRatio => "Panel ratio",
            Core::ScrollDirection => "Scroll direction",
            Core::StatusBar => "Status bar",
            Core::BibExportDir => "Bib export dir",
            Core::ExportDir => "Export dir",
        }
    }

    fn desc(self) -> &'static str {
        match self {
            Core::Home => "Portable home: db.json, pdfs/, notes/ (set by bibox init)",
            Core::PdfStorage => "Where attached PDFs are stored (iCloud, Google Drive, ...)",
            Core::CitekeyFormat => "Template for new citation keys",
            Core::Language => "UI language",
            Core::LineNumbers => "Entry list line numbers",
            Core::PanelRatio => "Width of collections : entries : preview",
            Core::ScrollDirection => "Mouse wheel direction",
            Core::StatusBar => "Hint bar at the bottom",
            Core::BibExportDir => "Where .bib exports go",
            Core::ExportDir => "Where other exports go",
        }
    }

    fn kind(self) -> Kind {
        match self {
            Core::Home => Kind::Path { presets: vec![], optional: true },
            Core::PdfStorage => Kind::Path { presets: pdf_dir_presets(), optional: true },
            Core::CitekeyFormat => Kind::Choice(CITEKEY_PRESETS.iter().map(|s| s.to_string()).collect()),
            Core::Language => Kind::Choice(vec!["en".into(), "ko".into()]),
            Core::LineNumbers => Kind::Choice(vec!["absolute".into(), "relative".into(), "none".into()]),
            Core::PanelRatio => Kind::Choice(PANEL_RATIO_PRESETS.iter().map(ratio_label).collect()),
            Core::ScrollDirection => Kind::Choice(vec!["natural".into(), "standard".into()]),
            Core::StatusBar => Kind::Bool { on: "shown", off: "hidden" },
            Core::BibExportDir | Core::ExportDir => Kind::Path { presets: dir_presets(), optional: false },
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    Core(Core),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    /// 검색과 선택기 되돌아오기의 열쇠. `appearance.line_numbers`, `plugins.git-sync.push_on_write`.
    pub id: String,
    pub section: Section,
    pub plugin: Option<String>,
    pub label: String,
    pub desc: String,
    pub kind: Kind,
    pub target: Target,
}

pub const PANEL_RATIO_PRESETS: [[u16; 3]; 5] = [[2, 4, 4], [1, 5, 4], [2, 3, 5], [1, 4, 5], [3, 4, 3]];

fn ratio_label(r: &[u16; 3]) -> String {
    format!("{}, {}, {}", r[0], r[1], r[2])
}

pub fn pdf_dir_presets() -> Vec<PathBuf> {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    vec![
        home.join("Library/Mobile Documents/com~apple~CloudDocs/bibox-pdfs"),
        home.join("Google Drive/bibox-pdfs"),
        home.join("Dropbox/bibox-pdfs"),
        home.join("Documents/bibox-pdfs"),
    ]
}

pub fn dir_presets() -> Vec<PathBuf> {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let download = dirs::download_dir().unwrap_or_else(|| home.clone());
    vec![PathBuf::from("."), download, home.join("Documents"), home.join("Desktop")]
}

pub fn core_items() -> Vec<Item> {
    Core::ALL
        .iter()
        .map(|c| Item {
            id: c.id().to_string(),
            section: c.section(),
            plugin: None,
            label: c.label().to_string(),
            desc: c.desc().to_string(),
            kind: c.kind(),
            target: Target::Core(*c),
        })
        .collect()
}

// ── 값 읽기 ─────────────────────────────────────────────────────────────────

pub fn value(item: &Item, config: &Config) -> String {
    match &item.target {
        Target::Core(c) => core_value(*c, config),
    }
}

fn core_value(core: Core, config: &Config) -> String {
    match core {
        Core::Home => match &config.home {
            Some(h) => h.display().to_string(),
            None => "(not set. use `bibox init <path>`)".to_string(),
        },
        Core::PdfStorage => match &config.pdf_dir {
            Some(p) => p.display().to_string(),
            None => "(default: home/pdfs/)".to_string(),
        },
        Core::CitekeyFormat => config.citekey_format.clone(),
        Core::Language => config.language.clone(),
        Core::LineNumbers => match config.line_numbers {
            LineNumbers::Absolute => "absolute",
            LineNumbers::Relative => "relative",
            LineNumbers::None => "none",
        }
        .to_string(),
        Core::PanelRatio => ratio_label(&config.panel_ratio),
        Core::ScrollDirection => (if config.natural_scroll { "natural" } else { "standard" }).to_string(),
        Core::StatusBar => (if config.status_bar { "shown" } else { "hidden" }).to_string(),
        Core::BibExportDir => config.bib_export_dir.display().to_string(),
        Core::ExportDir => config.export_dir.display().to_string(),
    }
}

// ── 값 쓰기 ─────────────────────────────────────────────────────────────────

/// choice/bool 값을 문자열로 적용한다. 호출자가 선택지 안의 값만 넘긴다.
fn apply_core(core: Core, config: &mut Config, chosen: &str) {
    match core {
        Core::LineNumbers => {
            config.line_numbers = match chosen {
                "relative" => LineNumbers::Relative,
                "none" => LineNumbers::None,
                _ => LineNumbers::Absolute,
            }
        }
        Core::PanelRatio => {
            if let Some(r) = PANEL_RATIO_PRESETS.iter().find(|r| ratio_label(r) == chosen) {
                config.panel_ratio = *r;
            }
        }
        Core::ScrollDirection => config.natural_scroll = chosen == "natural",
        Core::StatusBar => config.status_bar = chosen == "shown",
        Core::CitekeyFormat => config.citekey_format = chosen.to_string(),
        Core::Language => {
            config.language = chosen.to_string();
            config.msgs = crate::i18n::Msgs::new(chosen);
        }
        Core::Home | Core::PdfStorage | Core::BibExportDir | Core::ExportDir => {}
    }
}

/// 순환 인덱스. 현재 값이 목록 밖이면 +는 첫째, -는 마지막으로 간다.
fn cycle(len: usize, current: Option<usize>, delta: i32) -> usize {
    match (current, delta > 0) {
        (None, true) => 0,
        (None, false) => len - 1,
        (Some(i), true) => (i + 1) % len,
        (Some(i), false) => (i + len - 1) % len,
    }
}

/// h/l. config를 고쳤으면 true.
pub fn step(item: &Item, config: &mut Config, delta: i32) -> bool {
    match (&item.target, &item.kind) {
        (Target::Core(c), Kind::Bool { on, off }) => {
            let cur = core_value(*c, config);
            apply_core(*c, config, if cur == *on { off } else { on });
            true
        }
        (Target::Core(c), Kind::Choice(choices)) => {
            if choices.is_empty() {
                return false;
            }
            let cur = core_value(*c, config);
            let pos = choices.iter().position(|x| *x == cur);
            let next = cycle(choices.len(), pos, delta);
            apply_core(*c, config, &choices[next]);
            true
        }
        (Target::Core(c), Kind::Path { presets, optional }) => {
            if presets.is_empty() {
                return false;
            }
            // optional이면 "없음"이 0번 자리, 프리셋은 1..=n
            let current: Option<PathBuf> = match c {
                Core::PdfStorage => config.pdf_dir.clone(),
                Core::BibExportDir => Some(config.bib_export_dir.clone()),
                Core::ExportDir => Some(config.export_dir.clone()),
                Core::Home => config.home.clone(),
                _ => None,
            };
            let offset = if *optional { 1 } else { 0 };
            let len = presets.len() + offset;
            let pos = match &current {
                None if *optional => Some(0),
                None => None,
                Some(p) => presets.iter().position(|x| x == p).map(|i| i + offset),
            };
            let next = cycle(len, pos, delta);
            if *optional && next == 0 {
                if *c == Core::PdfStorage {
                    config.pdf_dir = None;
                }
            } else {
                set_path(item, config, presets[next - offset].clone());
            }
            true
        }
        (Target::Core(_), Kind::Int) | (Target::Core(_), Kind::Str) => false,
    }
}

/// 입력 팝업의 결과. 파싱 실패면 Err(문구).
pub fn set(item: &Item, config: &mut Config, text: &str) -> Result<(), String> {
    let text = text.trim();
    match (&item.target, &item.kind) {
        (Target::Core(c), Kind::Bool { on, off }) => {
            let v = match text {
                t if t == *on || t == "on" || t == "true" => on,
                t if t == *off || t == "off" || t == "false" => off,
                other => return Err(format!("expected {} or {}, got \"{}\"", on, off, other)),
            };
            apply_core(*c, config, v);
            Ok(())
        }
        (Target::Core(c), Kind::Choice(choices)) => {
            if !choices.iter().any(|x| x == text) {
                return Err(format!("\"{}\" is not one of: {}", text, choices.join(", ")));
            }
            apply_core(*c, config, text);
            Ok(())
        }
        (Target::Core(_), Kind::Path { .. }) => {
            if text.is_empty() {
                return Err("path is empty".to_string());
            }
            set_path(item, config, PathBuf::from(text));
            Ok(())
        }
        (Target::Core(_), Kind::Int) | (Target::Core(_), Kind::Str) => Err("not editable".to_string()),
    }
}

/// 디렉토리 선택기의 결과. Home은 지금처럼 pdfs/와 notes/도 따라간다.
pub fn set_path(item: &Item, config: &mut Config, path: PathBuf) {
    if let Target::Core(c) = &item.target {
        match c {
            Core::Home => {
                let expanded = expand_tilde(&path);
                config.bibox_dir = expanded.join("pdfs");
                config.notes_dir = expanded.join("notes");
                config.home = Some(path);
            }
            Core::PdfStorage => {
                config.bibox_dir = expand_tilde(&path);
                config.pdf_dir = Some(path);
            }
            Core::BibExportDir => config.bib_export_dir = path,
            Core::ExportDir => config.export_dir = path,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find<'a>(items: &'a [Item], id: &str) -> &'a Item {
        items.iter().find(|i| i.id == id).unwrap_or_else(|| panic!("no item {}", id))
    }

    #[test]
    fn core_items_are_ten_in_section_order_with_stable_ids() {
        assert_eq!(
            core_items().iter().map(|i| i.id.clone()).collect::<Vec<_>>(),
            vec![
                "general.home", "general.pdf_storage", "general.citekey_format", "general.language",
                "appearance.line_numbers", "appearance.panel_ratio", "appearance.scroll_direction", "appearance.status_bar",
                "export.bib_export_dir", "export.export_dir",
            ]
        );
        let sections: Vec<Section> = core_items().iter().map(|i| i.section).collect();
        assert!(sections.windows(2).all(|w| Section::ALL.iter().position(|s| *s == w[0]) <= Section::ALL.iter().position(|s| *s == w[1])));
        assert!(core_items().iter().all(|i| i.plugin.is_none()));
    }

    #[test]
    fn choice_steps_wrap_both_ways() {
        let items = core_items();
        let ln = find(&items, "appearance.line_numbers");
        let mut c = Config::default();
        assert_eq!(value(ln, &c), "absolute");
        assert!(step(ln, &mut c, 1));
        assert_eq!(value(ln, &c), "relative");
        assert!(step(ln, &mut c, 1));
        assert_eq!(value(ln, &c), "none");
        assert!(step(ln, &mut c, 1));
        assert_eq!(value(ln, &c), "absolute");
        assert!(step(ln, &mut c, -1));
        assert_eq!(value(ln, &c), "none");
    }

    #[test]
    fn bool_toggles_and_shows_its_labels() {
        let items = core_items();
        let sb = find(&items, "appearance.status_bar");
        let mut c = Config::default();
        assert_eq!(value(sb, &c), "shown");
        assert!(step(sb, &mut c, 1));
        assert_eq!(value(sb, &c), "hidden");
        assert!(!c.status_bar);
        assert!(step(sb, &mut c, -1));
        assert!(c.status_bar);
    }

    #[test]
    fn a_panel_ratio_off_the_presets_steps_to_the_first_preset() {
        let items = core_items();
        let pr = find(&items, "appearance.panel_ratio");
        let mut c = Config::default();
        c.panel_ratio = [9, 9, 9];
        assert_eq!(value(pr, &c), "9, 9, 9");
        assert!(step(pr, &mut c, 1));
        assert_eq!(c.panel_ratio, [2, 4, 4]);
        assert!(step(pr, &mut c, -1));
        assert_eq!(c.panel_ratio, [3, 4, 3], "last preset");
    }

    #[test]
    fn pdf_storage_cycles_none_then_the_presets() {
        let items = core_items();
        let pdf = find(&items, "general.pdf_storage");
        let mut c = Config::default();
        assert_eq!(c.pdf_dir, None);
        assert!(step(pdf, &mut c, 1));
        assert_eq!(c.pdf_dir, Some(pdf_dir_presets()[0].clone()));
        let n = pdf_dir_presets().len();
        for _ in 1..n {
            step(pdf, &mut c, 1);
        }
        assert_eq!(c.pdf_dir, Some(pdf_dir_presets()[n - 1].clone()));
        assert!(step(pdf, &mut c, 1));
        assert_eq!(c.pdf_dir, None, "wraps back to none");
        assert!(step(pdf, &mut c, -1));
        assert_eq!(c.pdf_dir, Some(pdf_dir_presets()[n - 1].clone()));
    }

    #[test]
    fn home_has_no_presets_so_step_does_nothing() {
        let items = core_items();
        let home = find(&items, "general.home");
        let mut c = Config::default();
        assert!(!step(home, &mut c, 1));
        assert_eq!(c.home, None);
        assert!(value(home, &c).contains("not set"));
    }

    #[test]
    fn set_rejects_an_unknown_choice_and_accepts_bool_words() {
        let items = core_items();
        let mut c = Config::default();
        let ln = find(&items, "appearance.line_numbers");
        assert!(set(ln, &mut c, "diagonal").is_err());
        assert!(set(ln, &mut c, "relative").is_ok());
        assert_eq!(c.line_numbers, LineNumbers::Relative);
        let sb = find(&items, "appearance.status_bar");
        assert!(set(sb, &mut c, "hidden").is_ok());
        assert!(!c.status_bar);
        assert!(set(sb, &mut c, "on").is_ok());
        assert!(c.status_bar);
        assert!(set(sb, &mut c, "maybe").is_err());
    }

    #[test]
    fn set_path_on_home_moves_pdfs_and_notes_with_it() {
        let items = core_items();
        let mut c = Config::default();
        set_path(find(&items, "general.home"), &mut c, PathBuf::from("/tmp/lib"));
        assert_eq!(c.home, Some(PathBuf::from("/tmp/lib")));
        assert_eq!(c.bibox_dir, PathBuf::from("/tmp/lib/pdfs"));
        assert_eq!(c.notes_dir, PathBuf::from("/tmp/lib/notes"));
        set_path(find(&items, "general.pdf_storage"), &mut c, PathBuf::from("/tmp/cloud"));
        assert_eq!(c.pdf_dir, Some(PathBuf::from("/tmp/cloud")));
        assert_eq!(c.bibox_dir, PathBuf::from("/tmp/cloud"));
        set_path(find(&items, "export.export_dir"), &mut c, PathBuf::from("/tmp/out"));
        assert_eq!(c.export_dir, PathBuf::from("/tmp/out"));
    }

    #[test]
    fn language_step_switches_the_messages_immediately() {
        let items = core_items();
        let lang = find(&items, "general.language");
        let mut c = Config::default();
        assert_eq!(value(lang, &c), "en");
        assert!(step(lang, &mut c, 1));
        assert_eq!(c.language, "ko");
        assert_eq!(c.msgs.no_entries(), "항목이 없습니다.");
    }
}
