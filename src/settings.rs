//! Settings 화면의 항목 자료. 코어 설정과 플러그인 설정을 같은 `Item`으로 표현한다.
//! 화면은 이 목록을 절로 걸러 그리고 검색은 글자로 거른다. TUI 없이 테스트된다.

use std::path::PathBuf;

use crate::config::{expand_tilde, Config, Images, LineNumbers, CITEKEY_PRESETS};
use crate::plugin::{Manifest, SettingDecl, SettingKind};

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
    Theme,
    Images,
    BibExportDir,
    ExportDir,
}

impl Core {
    pub const ALL: [Core; 12] = [
        Core::Home,
        Core::PdfStorage,
        Core::CitekeyFormat,
        Core::Language,
        Core::LineNumbers,
        Core::PanelRatio,
        Core::ScrollDirection,
        Core::StatusBar,
        Core::Theme,
        Core::Images,
        Core::BibExportDir,
        Core::ExportDir,
    ];

    fn section(self) -> Section {
        match self {
            Core::Home | Core::PdfStorage | Core::CitekeyFormat | Core::Language => Section::General,
            Core::LineNumbers | Core::PanelRatio | Core::ScrollDirection | Core::StatusBar | Core::Theme | Core::Images => Section::Appearance,
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
            Core::Theme => "appearance.theme",
            Core::Images => "appearance.images",
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
            Core::Theme => "Theme",
            Core::Images => "Images",
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
            Core::Theme => "Colors from a VS Code theme file in themes/, or the terminal's own",
            Core::Images => "Page images in preview tabs (kitty, iTerm2, sixel). Takes effect on next start",
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
            Core::Theme => Kind::Choice(crate::theme::list(&crate::config::themes_dir())),
            Core::Images => Kind::Choice(Images::ALL.iter().map(|i| i.name().to_string()).collect()),
            Core::BibExportDir | Core::ExportDir => Kind::Path { presets: dir_presets(), optional: false },
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    Core(Core),
    Plugin { name: String, decl: SettingDecl },
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

fn kind_of(k: &SettingKind) -> Kind {
    match k {
        SettingKind::Bool => Kind::Bool { on: "on", off: "off" },
        SettingKind::Int => Kind::Int,
        SettingKind::Str => Kind::Str,
        SettingKind::Choice(cs) => Kind::Choice(cs.clone()),
    }
}

/// 설치되어 로드된 플러그인의 `[[settings]]`. 매니페스트 순서, 선언 순서.
pub fn plugin_items(manifests: &[Manifest]) -> Vec<Item> {
    let mut out = Vec::new();
    for m in manifests {
        for d in &m.settings {
            out.push(Item {
                id: format!("plugins.{}.{}", m.name, d.key),
                section: Section::Plugins,
                plugin: Some(m.name.clone()),
                label: d.key.clone(),
                desc: d.desc.clone().unwrap_or_default(),
                kind: kind_of(&d.kind),
                target: Target::Plugin { name: m.name.clone(), decl: d.clone() },
            });
        }
    }
    out
}

pub fn items(manifests: &[Manifest]) -> Vec<Item> {
    let mut all = core_items();
    all.extend(plugin_items(manifests));
    all
}

// ── 값 읽기 ─────────────────────────────────────────────────────────────────

/// `[plugins.<name>]`의 값. 없거나 타입이 다르면 선언의 default.
fn plugin_value(config: &Config, name: &str, decl: &SettingDecl) -> toml::Value {
    config
        .plugins
        .get(name)
        .and_then(|t| t.get(&decl.key))
        .filter(|v| decl.kind.accepts(v))
        .cloned()
        .unwrap_or_else(|| decl.default.clone())
}

fn plugin_write(config: &mut Config, name: &str, key: &str, v: toml::Value) {
    config.plugins.entry(name.to_string()).or_default().insert(key.to_string(), v);
}

fn toml_display(v: &toml::Value) -> String {
    match v {
        toml::Value::String(s) => s.clone(),
        toml::Value::Boolean(b) => (if *b { "on" } else { "off" }).to_string(),
        other => other.to_string(),
    }
}

pub fn value(item: &Item, config: &Config) -> String {
    match &item.target {
        Target::Core(c) => core_value(*c, config),
        Target::Plugin { name, decl } => toml_display(&plugin_value(config, name, decl)),
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
        Core::Theme => config.theme.clone(),
        Core::Images => config.images.name().to_string(),
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
        Core::Images => {
            if let Some(i) = Images::parse(chosen) {
                config.images = i;
            }
        }
        Core::Theme => {
            config.theme = chosen.to_string();
            // 즉시 적용. 파일이 깨졌으면 terminal로(doctor가 이유를 말한다)
            let t = crate::theme::load(chosen, &crate::config::themes_dir()).unwrap_or_else(|_| crate::theme::Theme::terminal());
            crate::theme::set_theme(t);
        }
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
        (Target::Plugin { name, decl }, Kind::Bool { .. }) => {
            let cur = plugin_value(config, name, decl).as_bool().unwrap_or(false);
            plugin_write(config, name, &decl.key, toml::Value::Boolean(!cur));
            true
        }
        (Target::Plugin { name, decl }, Kind::Int) => {
            let cur = plugin_value(config, name, decl).as_integer().unwrap_or(0);
            plugin_write(config, name, &decl.key, toml::Value::Integer(cur + delta as i64));
            true
        }
        (Target::Plugin { name, decl }, Kind::Choice(choices)) => {
            let cur = plugin_value(config, name, decl);
            let pos = cur.as_str().and_then(|s| choices.iter().position(|c| c == s));
            let next = cycle(choices.len(), pos, delta);
            plugin_write(config, name, &decl.key, toml::Value::String(choices[next].clone()));
            true
        }
        (Target::Plugin { .. }, Kind::Str) | (Target::Plugin { .. }, Kind::Path { .. }) => false,
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
        (Target::Plugin { name, decl }, Kind::Bool { on, off }) => {
            let v = match text {
                t if t == *on || t == "true" => true,
                t if t == *off || t == "false" => false,
                other => return Err(format!("expected {} or {}, got \"{}\"", on, off, other)),
            };
            plugin_write(config, name, &decl.key, toml::Value::Boolean(v));
            Ok(())
        }
        (Target::Plugin { name, decl }, Kind::Int) => {
            let n: i64 = text.parse().map_err(|_| format!("not a number: \"{}\"", text))?;
            plugin_write(config, name, &decl.key, toml::Value::Integer(n));
            Ok(())
        }
        (Target::Plugin { name, decl }, Kind::Str) => {
            plugin_write(config, name, &decl.key, toml::Value::String(text.to_string()));
            Ok(())
        }
        (Target::Plugin { name, decl }, Kind::Choice(choices)) => {
            if !choices.iter().any(|x| x == text) {
                return Err(format!("\"{}\" is not one of: {}", text, choices.join(", ")));
            }
            plugin_write(config, name, &decl.key, toml::Value::String(text.to_string()));
            Ok(())
        }
        (Target::Plugin { .. }, Kind::Path { .. }) => Err("not editable".to_string()),
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

// ── 설명 상자 ────────────────────────────────────────────────────────────────

/// 단어 단위 줄바꿈. 한 단어가 폭보다 길면 그대로 한 줄.
pub fn wrap_words(text: &str, width: usize) -> Vec<String> {
    let width = width.max(8);
    let mut out = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > width {
            out.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() || out.is_empty() {
        out.push(line);
    }
    out
}

/// 설명 상자의 줄 수: 목록에 보이는 설명 중 가장 긴 것(줄바꿈 뒤)에 맞춘다. 1..=max.
/// 커서를 옮겨도 목록이 위아래로 움직이지 않도록 절 단위로 한 번 정한다.
pub fn desc_height(descs: &[&str], width: usize, max: usize) -> usize {
    descs.iter().map(|d| wrap_words(d, width).len()).max().unwrap_or(1).clamp(1, max.max(1))
}

/// 설명 한 개를 상자에 맞게. 넘치면 마지막 줄 끝에 `…`.
pub fn desc_lines(desc: &str, width: usize, max: usize) -> Vec<String> {
    let mut lines = wrap_words(desc, width);
    let max = max.max(1);
    if lines.len() > max {
        lines.truncate(max);
        if let Some(last) = lines.last_mut() {
            last.push('…');
        }
    }
    lines
}

// ── 검색 ────────────────────────────────────────────────────────────────────

/// 소문자, `/`와 `.`은 공백. `-`와 `_`는 그대로(`git-sync`가 잡히게).
pub fn tokens(query: &str) -> Vec<String> {
    query.to_lowercase().replace(['/', '.'], " ").split_whitespace().map(String::from).collect()
}

/// 조각이 전부 들어 있어야 한다(AND). 빈 조각 목록은 전부 잡는다.
pub fn matches(haystack: &str, tokens: &[String]) -> bool {
    let h = haystack.to_lowercase().replace(['/', '.'], " ");
    tokens.iter().all(|t| h.contains(t.as_str()))
}

fn haystack(item: &Item) -> String {
    format!("{} {} {} {}", item.section.label(), item.plugin.as_deref().unwrap_or(""), item.label, item.desc)
}

#[derive(Debug, Clone, PartialEq)]
pub enum Row {
    Header(String),
    Item(usize),
    Plugin(String),
}

/// 절 순서로, 소속이 바뀔 때마다 머리 줄. Plugins 절은 플러그인 행 묶음 뒤에 플러그인별 설정 묶음.
pub fn search(items: &[Item], plugins: &[(String, String)], query: &str) -> Vec<Row> {
    let toks = tokens(query);
    let mut out = Vec::new();
    for section in [Section::General, Section::Appearance, Section::Export] {
        let hits: Vec<usize> = items.iter().enumerate().filter(|(_, it)| it.section == section && matches(&haystack(it), &toks)).map(|(i, _)| i).collect();
        if !hits.is_empty() {
            out.push(Row::Header(section.label().to_string()));
            out.extend(hits.into_iter().map(Row::Item));
        }
    }
    let plugin_hits: Vec<&(String, String)> = plugins.iter().filter(|(n, d)| matches(&format!("plugins {} {}", n, d), &toks)).collect();
    if !plugin_hits.is_empty() {
        out.push(Row::Header(Section::Plugins.label().to_string()));
        out.extend(plugin_hits.into_iter().map(|(n, _)| Row::Plugin(n.clone())));
    }
    for (name, _) in plugins {
        let hits: Vec<usize> = items.iter().enumerate().filter(|(_, it)| it.plugin.as_deref() == Some(name) && matches(&haystack(it), &toks)).map(|(i, _)| i).collect();
        if !hits.is_empty() {
            out.push(Row::Header(format!("{} > {}", Section::Plugins.label(), name)));
            out.extend(hits.into_iter().map(Row::Item));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find<'a>(items: &'a [Item], id: &str) -> &'a Item {
        items.iter().find(|i| i.id == id).unwrap_or_else(|| panic!("no item {}", id))
    }

    #[test]
    fn core_items_are_twelve_in_section_order_with_stable_ids() {
        assert_eq!(
            core_items().iter().map(|i| i.id.clone()).collect::<Vec<_>>(),
            vec![
                "general.home", "general.pdf_storage", "general.citekey_format", "general.language",
                "appearance.line_numbers", "appearance.panel_ratio", "appearance.scroll_direction", "appearance.status_bar",
                "appearance.theme", "appearance.images",
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
    fn manifest_with_settings() -> crate::plugin::Manifest {
        let text = "api = 2\nname = \"demo\"\nrun = \"sh\"\n\n[[settings]]\nkey = \"push\"\ntype = \"bool\"\ndefault = false\ndesc = \"push after commit\"\n\n[[settings]]\nkey = \"n\"\ntype = \"int\"\ndefault = 4\n\n[[settings]]\nkey = \"model\"\ntype = \"choice\"\nchoices = [\"a\", \"b\"]\ndefault = \"a\"\n\n[[settings]]\nkey = \"label\"\ntype = \"string\"\ndefault = \"x\"\n";
        let mut problems = vec![];
        crate::plugin::manifest::parse_manifest(std::path::Path::new("/tmp/plugins/demo"), text, &mut problems).expect("manifest")
    }

    #[test]
    fn plugin_items_follow_the_core_items_and_carry_the_plugin_name() {
        let items = items(&[manifest_with_settings()]);
        assert_eq!(items.len(), 16);
        let p = &items[12];
        assert_eq!(p.id, "plugins.demo.push");
        assert_eq!(p.section, Section::Plugins);
        assert_eq!(p.plugin.as_deref(), Some("demo"));
        assert_eq!(p.label, "push");
        assert_eq!(p.desc, "push after commit");
        assert_eq!(p.kind, Kind::Bool { on: "on", off: "off" });
        assert_eq!(items[15].kind, Kind::Str);
    }

    #[test]
    fn a_plugin_setting_steps_write_into_the_plugins_table() {
        let items = items(&[manifest_with_settings()]);
        let mut c = Config::default();
        let push = find(&items, "plugins.demo.push");
        assert_eq!(value(push, &c), "off");
        assert!(step(push, &mut c, 1));
        assert_eq!(c.plugins["demo"]["push"], toml::Value::Boolean(true));
        assert_eq!(value(push, &c), "on");
        let n = find(&items, "plugins.demo.n");
        assert!(step(n, &mut c, 1));
        assert_eq!(c.plugins["demo"]["n"], toml::Value::Integer(5));
        assert!(step(n, &mut c, -1));
        assert_eq!(value(n, &c), "4");
        let model = find(&items, "plugins.demo.model");
        assert!(step(model, &mut c, -1));
        assert_eq!(value(model, &c), "b", "wraps");
        let label = find(&items, "plugins.demo.label");
        assert!(!step(label, &mut c, 1), "strings are not stepped");
    }

    #[test]
    fn a_wrongly_typed_value_shows_the_default_until_overwritten() {
        let items = items(&[manifest_with_settings()]);
        let mut c = Config::default();
        c.plugins.entry("demo".into()).or_default().insert("push".into(), toml::Value::String("yes".into()));
        let push = find(&items, "plugins.demo.push");
        assert_eq!(value(push, &c), "off");
        assert!(step(push, &mut c, 1));
        assert_eq!(c.plugins["demo"]["push"], toml::Value::Boolean(true));
    }

    #[test]
    fn set_parses_plugin_values_and_rejects_bad_ones() {
        let items = items(&[manifest_with_settings()]);
        let mut c = Config::default();
        let n = find(&items, "plugins.demo.n");
        assert!(set(n, &mut c, "abc").is_err());
        assert!(set(n, &mut c, "12").is_ok());
        assert_eq!(c.plugins["demo"]["n"], toml::Value::Integer(12));
        let model = find(&items, "plugins.demo.model");
        assert!(set(model, &mut c, "zzz").is_err());
        assert!(set(model, &mut c, "b").is_ok());
        let label = find(&items, "plugins.demo.label");
        assert!(set(label, &mut c, "hello world").is_ok());
        assert_eq!(c.plugins["demo"]["label"], toml::Value::String("hello world".into()));
        let push = find(&items, "plugins.demo.push");
        assert!(set(push, &mut c, "true").is_ok());
        assert_eq!(c.plugins["demo"]["push"], toml::Value::Boolean(true));
    }
    fn plugin_list() -> Vec<(String, String)> {
        vec![("demo".into(), "Commit the library on every write".into()), ("other".into(), "Something else".into())]
    }

    fn ids_of(rows: &[Row], items: &[Item]) -> Vec<String> {
        rows.iter()
            .map(|r| match r {
                Row::Header(h) => format!("# {}", h),
                Row::Item(i) => items[*i].id.clone(),
                Row::Plugin(p) => format!("@{}", p),
            })
            .collect()
    }

    #[test]
    fn tokens_lowercase_and_split_on_space_slash_and_dot() {
        assert_eq!(tokens("Git-Sync/push"), vec!["git-sync", "push"]);
        assert_eq!(tokens("a.b  c"), vec!["a", "b", "c"]);
        assert!(tokens("   ").is_empty());
        assert!(matches("Plugins demo push_on_write git push after commit", &tokens("demo/push")));
        assert!(!matches("Appearance line numbers", &tokens("push")));
    }

    #[test]
    fn search_groups_matches_by_section_and_plugin_in_order() {
        let items = items(&[manifest_with_settings()]);
        let rows = search(&items, &plugin_list(), "push");
        assert_eq!(ids_of(&rows, &items), vec!["# Plugins > demo", "plugins.demo.push"]);
        let rows = search(&items, &plugin_list(), "line");
        assert_eq!(ids_of(&rows, &items), vec!["# Appearance", "appearance.line_numbers"]);
        let rows = search(&items, &plugin_list(), "demo/push");
        assert_eq!(ids_of(&rows, &items), vec!["# Plugins > demo", "plugins.demo.push"]);
    }

    #[test]
    fn search_matches_descriptions_and_plugin_rows() {
        let items = items(&[manifest_with_settings()]);
        // "commit"은 demo 플러그인 설명과 push 설정의 desc("push after commit")에 있다
        let rows = search(&items, &plugin_list(), "commit");
        assert_eq!(ids_of(&rows, &items), vec!["# Plugins", "@demo", "# Plugins > demo", "plugins.demo.push"]);
    }

    #[test]
    fn an_empty_query_lists_everything_with_headers() {
        let items = items(&[manifest_with_settings()]);
        let rows = search(&items, &plugin_list(), "");
        let ids = ids_of(&rows, &items);
        assert_eq!(ids[0], "# General");
        assert!(ids.contains(&"# Appearance".to_string()) && ids.contains(&"# Export".to_string()));
        assert!(ids.contains(&"@other".to_string()));
        assert_eq!(ids.iter().filter(|s| !s.starts_with('#')).count(), 16 + 2);
    }

    #[test]
    fn images_cycles_the_six_names() {
        let items = core_items();
        let im = find(&items, "appearance.images");
        let mut c = Config::default();
        assert_eq!(value(im, &c), "auto");
        assert!(step(im, &mut c, 1));
        assert_eq!(c.images, crate::config::Images::Off);
        assert!(step(im, &mut c, -1));
        assert!(step(im, &mut c, -1));
        assert_eq!(value(im, &c), "halfblocks", "wraps");
        assert!(set(im, &mut c, "kitty").is_ok());
        assert_eq!(c.images, crate::config::Images::Kitty);
        assert!(set(im, &mut c, "png").is_err());
    }

    #[test]
    fn desc_box_height_follows_the_longest_description_within_the_cap() {
        let short = "one line";
        let long = "aaaa bbbb cccc dddd eeee ffff gggg hhhh iiii jjjj kkkk llll mmmm nnnn oooo";
        assert_eq!(desc_height(&[short, ""], 40, 4), 1, "nothing to say still keeps one row");
        assert_eq!(desc_height(&[short, long], 40, 4), 2, "{:?}", wrap_words(long, 40));
        assert_eq!(desc_height(&[long], 10, 4), 4, "capped");
        assert_eq!(desc_height(&[], 40, 4), 1);
    }

    #[test]
    fn desc_lines_wrap_and_end_with_an_ellipsis_when_cut() {
        let long = "aaaa bbbb cccc dddd eeee ffff gggg hhhh";
        assert_eq!(desc_lines(long, 10, 4), vec!["aaaa bbbb", "cccc dddd", "eeee ffff", "gggg hhhh"]);
        assert_eq!(desc_lines(long, 10, 2), vec!["aaaa bbbb", "cccc dddd…"]);
        assert_eq!(desc_lines("", 10, 2), vec![""]);
        assert_eq!(desc_lines("fits", 10, 1), vec!["fits"]);
    }

    #[test]
    fn theme_cycles_terminal_presets_and_files_and_applies_at_once() {
        let items = core_items();
        let th = find(&items, "appearance.theme");
        let mut c = Config::default();
        assert_eq!(value(th, &c), "terminal");
        assert!(step(th, &mut c, 1));
        assert_eq!(c.theme, "dark");
        assert!(crate::theme::theme().bg.is_some(), "dark paints a background");
        assert!(step(th, &mut c, 1));
        assert_eq!(c.theme, "light");
        assert!(set(th, &mut c, "terminal").is_ok());
        assert_eq!(crate::theme::theme(), crate::theme::Theme::terminal());
        assert!(set(th, &mut c, "no-such-theme").is_err());
        crate::theme::set_theme(crate::theme::Theme::terminal());
    }
}
