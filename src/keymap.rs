use crossterm::event::{KeyCode, KeyModifiers};
use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyPress {
    pub code: KeyCode,
    pub mods: KeyModifiers,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyParseError {
    pub token: String,
}

/// `<C-d>` `<Esc>` `<F1>` 같은 yazi식 표기와 단일 문자를 읽는다.
pub fn parse_key(s: &str) -> Result<KeyPress, KeyParseError> {
    let err = || KeyParseError { token: s.to_string() };

    if !s.starts_with('<') {
        let mut chars = s.chars();
        let (Some(c), None) = (chars.next(), chars.next()) else {
            return Err(err());
        };
        return Ok(KeyPress { code: KeyCode::Char(c), mods: KeyModifiers::NONE });
    }

    let inner = s.strip_prefix('<').and_then(|r| r.strip_suffix('>')).ok_or_else(err)?;

    // 수식어 접두사: C- (Control), A- (Alt), S- (Shift)
    let (mods, rest) = match inner.split_once('-') {
        Some(("C", rest)) => (KeyModifiers::CONTROL, rest),
        Some(("A", rest)) => (KeyModifiers::ALT, rest),
        Some(("S", rest)) => (KeyModifiers::SHIFT, rest),
        _ => (KeyModifiers::NONE, inner),
    };

    let code = match rest {
        "Esc" => KeyCode::Esc,
        "Space" => KeyCode::Char(' '),
        "Tab" => KeyCode::Tab,
        "Enter" => KeyCode::Enter,
        "Backspace" => KeyCode::Backspace,
        "Left" => KeyCode::Left,
        "Right" => KeyCode::Right,
        "Up" => KeyCode::Up,
        "Down" => KeyCode::Down,
        _ => {
            if let Some(n) = rest.strip_prefix('F').and_then(|n| n.parse::<u8>().ok()) {
                if (1..=12).contains(&n) {
                    KeyCode::F(n)
                } else {
                    return Err(err());
                }
            } else {
                let mut chars = rest.chars();
                match (chars.next(), chars.next()) {
                    (Some(c), None) => KeyCode::Char(c),
                    _ => return Err(err()),
                }
            }
        }
    };

    Ok(KeyPress { code, mods })
}

/// `parse_key`의 역이다. 도움말 표에 키를 찍을 때 쓴다.
pub fn render_key(k: KeyPress) -> String {
    let name = match k.code {
        KeyCode::Esc => "Esc".to_string(),
        KeyCode::Char(' ') => "Space".to_string(),
        KeyCode::Tab => "Tab".to_string(),
        KeyCode::Enter => "Enter".to_string(),
        KeyCode::Backspace => "Backspace".to_string(),
        KeyCode::Left => "Left".to_string(),
        KeyCode::Right => "Right".to_string(),
        KeyCode::Up => "Up".to_string(),
        KeyCode::Down => "Down".to_string(),
        KeyCode::F(n) => format!("F{}", n),
        KeyCode::Char(c) => c.to_string(),
        other => format!("{:?}", other),
    };

    let modifier = if k.mods.contains(KeyModifiers::CONTROL) {
        Some("C")
    } else if k.mods.contains(KeyModifiers::ALT) {
        Some("A")
    } else if k.mods.contains(KeyModifiers::SHIFT) {
        Some("S")
    } else {
        None
    };

    let bare_char = matches!(k.code, KeyCode::Char(c) if c != ' ');
    match (modifier, bare_char) {
        (None, true) => name,
        (None, false) => format!("<{}>", name),
        (Some(m), _) => format!("<{}-{}>", m, name),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    // ── 전역 (세 레이어 모두) ──
    Quit,
    Cancel,
    Undo,
    Redo,
    NextPreviewTab,
    Search,
    CopyCitekey,
    OpenPdf,
    OpenWeb,
    FetchMetadata,
    ExportMenu,
    Delete,
    Help,
    EditNote,
    SortMenu,
    Collections,
    Tags,
    AttachPdf,
    Settings,
    Noop,
    // ── normal.entries 전용 ──
    EntryDown,
    EntryUp,
    EntryTop,
    EntryBottom,
    EntryScreenTop,
    EntryScreenMiddle,
    EntryScreenBottom,
    EntryHalfPageDown,
    EntryHalfPageUp,
    ToggleSelect,
    SelectAll,
    FocusCollections,
    FocusPreview,
    // ── normal.collections 전용 ──
    CollectionDown,
    CollectionUp,
    CollectionTop,
    CollectionBottom,
    CollectionHalfPageDown,
    CollectionHalfPageUp,
    FocusEntries,
    // ── normal.preview 전용 ──
    PreviewScrollDown,
    PreviewScrollUp,
    PreviewTop,
    PreviewBottom,
    PreviewHalfPageDown,
    PreviewHalfPageUp,
    NextTab,
    PrevTab,
    PrevTabOrFocusEntries,
}

impl Action {
    /// 도움말 생성과 테스트가 전수 순회에 쓴다.
    pub fn all() -> &'static [Action] {
        use Action::*;
        &[
            Quit, Cancel, Undo, Redo, NextPreviewTab, Search, CopyCitekey, OpenPdf,
            OpenWeb, FetchMetadata, ExportMenu, Delete, Help, EditNote, SortMenu,
            Collections, Tags, AttachPdf, Settings, Noop,
            EntryDown, EntryUp, EntryTop, EntryBottom, EntryScreenTop, EntryScreenMiddle,
            EntryScreenBottom, EntryHalfPageDown, EntryHalfPageUp, ToggleSelect, SelectAll,
            FocusCollections, FocusPreview,
            CollectionDown, CollectionUp, CollectionTop, CollectionBottom,
            CollectionHalfPageDown, CollectionHalfPageUp, FocusEntries,
            PreviewScrollDown, PreviewScrollUp, PreviewTop, PreviewBottom,
            PreviewHalfPageDown, PreviewHalfPageUp, NextTab, PrevTab, PrevTabOrFocusEntries,
        ]
    }

    pub fn section(&self) -> &'static str {
        use Action::*;
        match self {
            EntryDown | EntryUp | EntryTop | EntryBottom | EntryScreenTop
            | EntryScreenMiddle | EntryScreenBottom | EntryHalfPageDown | EntryHalfPageUp
            | CollectionDown | CollectionUp | CollectionTop | CollectionBottom
            | CollectionHalfPageDown | CollectionHalfPageUp
            | PreviewScrollDown | PreviewScrollUp | PreviewTop | PreviewBottom
            | PreviewHalfPageDown | PreviewHalfPageUp
            | FocusCollections | FocusEntries | FocusPreview
            | NextTab | PrevTab | PrevTabOrFocusEntries | NextPreviewTab => "Navigation",

            ToggleSelect | SelectAll | Cancel => "Selection",

            OpenPdf | OpenWeb | FetchMetadata | AttachPdf | CopyCitekey | EditNote
            | ExportMenu | Delete => "Entry actions",

            Collections | Tags | Undo | Redo => "Editing",

            Search | SortMenu => "Search and sort",

            Help | Settings | Quit | Noop => "Application",
        }
    }

    /// 도움말의 기본 설명. 바인딩에 `desc`가 있으면 그쪽이 이긴다.
    pub fn desc(&self) -> &'static str {
        use Action::*;
        match self {
            Quit => "Leave bibox",
            Cancel => "Drop the current selection, or quit when nothing is selected",
            Undo => "Undo the last change to the library",
            Redo => "Redo the change that was last undone",
            NextPreviewTab => "Cycle the preview panel between Info, Note and PDF",
            Search => "Filter entries by title, author, key or tag; filters collections when that panel has focus",
            CopyCitekey => "Copy the BibTeX citation key to the system clipboard",
            OpenPdf => "Open the attached PDF, or offer to fetch one when missing",
            OpenWeb => "Open the entry DOI or URL in the default browser",
            FetchMetadata => "Look the entry up by DOI, or search by title when it has none",
            ExportMenu => "Export the selection as BibTeX, PDFs or a zip archive",
            Delete => "Delete the current entry after a confirmation prompt",
            Help => "Open this help screen",
            EditNote => "Open the entry Markdown note in $EDITOR",
            SortMenu => "Choose the sort field and toggle ascending or descending order",
            Collections => "Add or remove the entry from collections",
            Tags => "Edit the tags attached to the entry",
            AttachPdf => "Pick a PDF from disk and attach it to the current entry",
            Settings => "Open the settings screen",
            Noop => "Do nothing (used to disable a key)",

            EntryDown => "Move down one entry",
            EntryUp => "Move up one entry",
            EntryTop => "Jump to the first entry",
            EntryBottom => "Jump to the last entry",
            EntryScreenTop => "Jump near the top of the visible entry list",
            EntryScreenMiddle => "Jump to the middle of the entry list",
            EntryScreenBottom => "Jump near the bottom of the visible entry list",
            EntryHalfPageDown => "Scroll the entry list down half a screen",
            EntryHalfPageUp => "Scroll the entry list up half a screen",
            ToggleSelect => "Select or deselect the current entry",
            SelectAll => "Select every visible entry, or clear if all are selected",
            FocusCollections => "Move focus to the Collections panel",
            FocusPreview => "Move focus to the Preview panel",

            CollectionDown => "Move down one collection",
            CollectionUp => "Move up one collection",
            CollectionTop => "Jump to the first collection",
            CollectionBottom => "Jump to the last collection",
            CollectionHalfPageDown => "Scroll the collection list down half a screen",
            CollectionHalfPageUp => "Scroll the collection list up half a screen",
            FocusEntries => "Move focus to the Entries panel",

            PreviewScrollDown => "Scroll the preview down one line",
            PreviewScrollUp => "Scroll the preview up one line",
            PreviewTop => "Jump to the top of the preview",
            PreviewBottom => "Jump to the bottom of the preview",
            PreviewHalfPageDown => "Scroll the preview down half a screen",
            PreviewHalfPageUp => "Scroll the preview up half a screen",
            NextTab => "Step forward a preview tab",
            PrevTab => "Step back a preview tab",
            PrevTabOrFocusEntries => "Step back a preview tab, or leave for the Entries panel",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_key_reads_plain_characters() {
        let k = parse_key("j").unwrap();
        assert_eq!(k.code, KeyCode::Char('j'));
        assert_eq!(k.mods, KeyModifiers::NONE);
    }

    #[test]
    fn parse_key_reads_control_notation() {
        let k = parse_key("<C-d>").unwrap();
        assert_eq!(k.code, KeyCode::Char('d'));
        assert_eq!(k.mods, KeyModifiers::CONTROL);
    }

    #[test]
    fn parse_key_reads_named_keys() {
        assert_eq!(parse_key("<Esc>").unwrap().code, KeyCode::Esc);
        assert_eq!(parse_key("<Space>").unwrap().code, KeyCode::Char(' '));
        assert_eq!(parse_key("<Tab>").unwrap().code, KeyCode::Tab);
        assert_eq!(parse_key("<Enter>").unwrap().code, KeyCode::Enter);
        assert_eq!(parse_key("<Left>").unwrap().code, KeyCode::Left);
        assert_eq!(parse_key("<F1>").unwrap().code, KeyCode::F(1));
    }

    #[test]
    fn parse_key_rejects_unknown_notation() {
        let err = parse_key("<Ctrl-d>").unwrap_err();
        assert_eq!(err.token, "<Ctrl-d>");
    }

    #[test]
    fn render_key_round_trips_every_notation() {
        for s in ["j", "G", "<C-d>", "<Esc>", "<Space>", "<Tab>", "<Enter>", "<Left>", "<F1>"] {
            let k = parse_key(s).unwrap();
            assert_eq!(render_key(k), s, "round trip failed for {}", s);
        }
    }

    #[test]
    fn action_names_are_snake_case_in_toml() {
        let a: Action = serde_json::from_str("\"entry_down\"").unwrap();
        assert_eq!(a, Action::EntryDown);
        let b: Action = serde_json::from_str("\"prev_tab_or_focus_entries\"").unwrap();
        assert_eq!(b, Action::PrevTabOrFocusEntries);
    }

    #[test]
    fn unknown_action_name_is_rejected() {
        let r: Result<Action, _> = serde_json::from_str("\"opne_pdf\"");
        assert!(r.is_err(), "an unknown action name must not deserialize");
    }

    #[test]
    fn every_action_has_a_description_and_a_section() {
        const SECTIONS: &[&str] = &[
            "Navigation", "Selection", "Entry actions",
            "Editing", "Search and sort", "Application",
        ];
        for a in Action::all() {
            assert!(!a.desc().is_empty(), "{:?} has an empty description", a);
            assert!(SECTIONS.contains(&a.section()), "{:?} has an unknown section {}", a, a.section());
        }
    }
}
