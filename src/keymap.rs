use crossterm::event::{KeyCode, KeyModifiers};
use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyPress {
    pub code: KeyCode,
    pub mods: KeyModifiers,
}

impl KeyPress {
    /// 문자 키에서는 SHIFT를 버린다.
    ///
    /// 대문자는 shift가 이미 문자 자체에 인코딩돼 있어서 `Char('G')` + SHIFT는
    /// 같은 정보를 두 번 표현한다. 터미널마다 SHIFT를 싣기도 하고 안 싣기도 해서,
    /// 정규화하지 않으면 `G` 바인딩이 어떤 터미널에서는 걸리고 어떤 터미널에서는
    /// 안 걸린다. 문자가 아닌 키(`<S-Tab>` 등)에서는 SHIFT가 유일한 구분이므로 남긴다.
    pub fn new(code: KeyCode, mods: KeyModifiers) -> Self {
        let mods = if matches!(code, KeyCode::Char(_)) {
            mods & !KeyModifiers::SHIFT
        } else {
            mods
        };
        KeyPress { code, mods }
    }
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
        return Ok(KeyPress::new(KeyCode::Char(c), KeyModifiers::NONE));
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

    Ok(KeyPress::new(code, mods))
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
    CopyCitation,
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
    Plugins,
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
    // ── 플러그인 미리보기 탭 전용. 다른 탭에서는 무동작 ──
    TabNextPage,
    TabPrevPage,
    TabZoomIn,
    TabZoomOut,
    TabZoomReset,
    TabPanLeft,
    TabPanRight,
    // ── 플러그인 명령. 인덱스는 로드 시 만든 명령 테이블을 가리킨다 ──
    #[serde(skip)]
    Plugin(crate::plugin::PluginCmdId),
}

impl Action {
    /// 모든 변형에 설명과 섹션이 붙어 있는지 검사하는 테스트 전용 목록.
    /// 도움말은 키맵의 바인딩에서 생성되므로 이 목록을 쓰지 않는다.
    #[cfg(test)]
    pub fn all() -> &'static [Action] {
        use Action::*;
        &[
            Quit, Cancel, Undo, Redo, NextPreviewTab, Search, CopyCitekey, CopyCitation, OpenPdf,
            OpenWeb, FetchMetadata, ExportMenu, Delete, Help, EditNote, SortMenu,
            Collections, Tags, AttachPdf, Settings, Plugins, Noop,
            EntryDown, EntryUp, EntryTop, EntryBottom, EntryScreenTop, EntryScreenMiddle,
            EntryScreenBottom, EntryHalfPageDown, EntryHalfPageUp, ToggleSelect, SelectAll,
            FocusCollections, FocusPreview,
            CollectionDown, CollectionUp, CollectionTop, CollectionBottom,
            CollectionHalfPageDown, CollectionHalfPageUp, FocusEntries,
            PreviewScrollDown, PreviewScrollUp, PreviewTop, PreviewBottom,
            PreviewHalfPageDown, PreviewHalfPageUp, NextTab, PrevTab, PrevTabOrFocusEntries,
            TabNextPage, TabPrevPage, TabZoomIn, TabZoomOut, TabZoomReset, TabPanLeft, TabPanRight,
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
            | NextTab | PrevTab | PrevTabOrFocusEntries | NextPreviewTab
            | TabNextPage | TabPrevPage | TabZoomIn | TabZoomOut | TabZoomReset | TabPanLeft | TabPanRight => "Navigation",

            ToggleSelect | SelectAll | Cancel => "Selection",

            OpenPdf | OpenWeb | FetchMetadata | AttachPdf | CopyCitekey | CopyCitation | EditNote
            | ExportMenu | Delete => "Entry actions",

            Collections | Tags | Undo | Redo => "Editing",

            Search | SortMenu => "Search and sort",

            Help | Settings | Plugins | Quit | Noop => "Application",

            Plugin(_) => "Plugins",
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
            CopyCitation => "Copy a formatted citation (APA, IEEE or Chicago) to the clipboard",
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
            Plugins => "Open Settings on the plugin list",
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
            TabNextPage => "Next page of the preview tab",
            TabPrevPage => "Previous page of the preview tab",
            TabZoomIn => "Zoom the preview tab in by 25%",
            TabZoomOut => "Zoom the preview tab out by 25%",
            TabZoomReset => "Fit the preview tab page to the panel width",
            TabPanLeft => "Pan the preview tab left when the page is wider than the panel",
            TabPanRight => "Pan the preview tab right",

            Plugin(_) => "Run a plugin command",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    Continue,
    Quit,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Binding {
    pub keys: Vec<KeyPress>,
    pub actions: Vec<Action>,
    pub desc: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Layer {
    pub bindings: Vec<Binding>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerId {
    Collections,
    Entries,
    Preview,
}

#[derive(Debug, Clone, Default)]
pub struct Keymap {
    pub collections: Layer,
    pub entries: Layer,
    pub preview: Layer,
}

impl Keymap {
    pub fn layer(&self, id: LayerId) -> &Layer {
        match id {
            LayerId::Collections => &self.collections,
            LayerId::Entries => &self.entries,
            LayerId::Preview => &self.preview,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Resolution {
    Pending,
    Run(Vec<Action>),
    Unbound,
}

/// `pending`에 `key`를 이어 붙인 시퀀스를 레이어에서 찾는다.
/// 더 긴 바인딩의 접두사이면 기다리고, 정확히 일치하면 실행하고,
/// 아무것도 안 걸리면 대기를 버린다. 타임아웃은 없다.
pub fn resolve(layer: &Layer, pending: &[KeyPress], key: KeyPress) -> Resolution {
    let mut seq: Vec<KeyPress> = pending.to_vec();
    seq.push(key);

    let has_longer = layer
        .bindings
        .iter()
        .any(|b| b.keys.len() > seq.len() && b.keys.starts_with(&seq));
    if has_longer {
        return Resolution::Pending;
    }

    match layer.bindings.iter().find(|b| b.keys == seq) {
        Some(b) => Resolution::Run(b.actions.clone()),
        None => Resolution::Unbound,
    }
}

fn b(keys: &str, actions: &[Action]) -> Binding {
    Binding {
        keys: keys
            .split(' ')
            .map(|k| parse_key(k).expect("default keymap has a bad key"))
            .collect(),
        actions: actions.to_vec(),
        desc: None,
    }
}

/// 세 레이어 모두에 복제되는 전역 바인딩.
fn global_bindings() -> Vec<Binding> {
    use Action::*;
    vec![
        b("q", &[Quit]),
        b("<Esc>", &[Cancel]),
        b("<C-c>", &[Quit]),
        b("<C-z>", &[Undo]),
        b("<C-y>", &[Redo]),
        b("<Tab>", &[NextPreviewTab]),
        b("/", &[Search]),
        b("y", &[CopyCitekey]),
        b("Y", &[CopyCitation]),
        b("o", &[OpenPdf]),
        b("w", &[OpenWeb]),
        b("f", &[FetchMetadata]),
        b("e", &[ExportMenu]),
        b("d", &[Delete]),
        b("?", &[Help]),
        b("`", &[Help]),
        b("~", &[Help]),
        b("<F1>", &[Help]),
        b("N", &[EditNote]),
        b("s", &[SortMenu]),
        b("c", &[Collections]),
        b("t", &[Tags]),
        b("A", &[AttachPdf]),
        b(",", &[Settings]),
        b("<Enter>", &[Noop]),
        // 패널로 바로. 어느 패널에서든 한 키
        b("1", &[FocusCollections]),
        b("2", &[FocusEntries]),
        b("3", &[FocusPreview]),
    ]
}

pub fn default_keymap() -> Keymap {
    use Action::*;

    let mut entries = vec![
        b("h", &[FocusCollections]),
        b("<Left>", &[FocusCollections]),
        b("l", &[FocusPreview]),
        b("<Right>", &[FocusPreview]),
        b("j", &[EntryDown]),
        b("<Down>", &[EntryDown]),
        b("k", &[EntryUp]),
        b("<Up>", &[EntryUp]),
        b("g g", &[EntryTop]),
        b("G", &[EntryBottom]),
        b("<C-d>", &[EntryHalfPageDown]),
        b("<C-u>", &[EntryHalfPageUp]),
        b("H", &[EntryScreenTop]),
        b("M", &[EntryScreenMiddle]),
        b("L", &[EntryScreenBottom]),
        b("<Space>", &[ToggleSelect, EntryDown]),
        b("V", &[SelectAll]),
    ];
    entries.extend(global_bindings());

    // 컬렉션 패널의 h는 옛 handle_normal에서 빈 블록이라 바인딩하지 않는다.
    let mut collections = vec![
        b("l", &[FocusEntries]),
        b("<Right>", &[FocusEntries]),
        b("j", &[CollectionDown]),
        b("<Down>", &[CollectionDown]),
        b("k", &[CollectionUp]),
        b("<Up>", &[CollectionUp]),
        b("g g", &[CollectionTop]),
        b("G", &[CollectionBottom]),
        b("<C-d>", &[CollectionHalfPageDown]),
        b("<C-u>", &[CollectionHalfPageUp]),
    ];
    collections.extend(global_bindings());

    let mut preview = vec![
        b("h", &[PrevTabOrFocusEntries]),
        b("<Left>", &[PrevTabOrFocusEntries]),
        b("l", &[NextTab]),
        b("<Right>", &[NextTab]),
        b("j", &[PreviewScrollDown]),
        b("<Down>", &[PreviewScrollDown]),
        b("k", &[PreviewScrollUp]),
        b("<Up>", &[PreviewScrollUp]),
        b("g g", &[PreviewTop]),
        b("G", &[PreviewBottom]),
        b("<C-d>", &[PreviewHalfPageDown]),
        b("<C-u>", &[PreviewHalfPageUp]),
        b("n", &[TabNextPage]),
        b("p", &[TabPrevPage]),
        b("+", &[TabZoomIn]),
        b("=", &[TabZoomIn]),
        b("-", &[TabZoomOut]),
        b("0", &[TabZoomReset]),
        b("H", &[TabPanLeft]),
        b("L", &[TabPanRight]),
    ];
    preview.extend(global_bindings());

    Keymap {
        collections: Layer { bindings: collections },
        entries: Layer { bindings: entries },
        preview: Layer { bindings: preview },
    }
}

/// `on`은 문자열 하나 또는 배열이다.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum OnField {
    One(String),
    Many(Vec<String>),
}

/// `run`의 원소 하나. `.`이 있으면 플러그인 명령 참조(`entry-tidy.tidy`), 없으면 내장 액션.
/// 내장 액션 이름이 틀리면 serde의 "unknown variant" 에러가 그대로 나와 기존
/// `UnknownAction` 판정이 유지된다.
#[derive(Debug, Clone, PartialEq)]
pub enum RunItem {
    Builtin(Action),
    Plugin(String),
}

impl<'de> serde::Deserialize<'de> for RunItem {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::IntoDeserializer;
        let s = String::deserialize(d)?;
        if s.contains('.') {
            return Ok(RunItem::Plugin(s));
        }
        Action::deserialize(s.as_str().into_deserializer()).map(RunItem::Builtin)
    }
}

/// `run`도 하나 또는 배열이다.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum RunField {
    One(RunItem),
    Many(Vec<RunItem>),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BindingFile {
    pub on: OnField,
    pub run: RunField,
    #[serde(default)]
    pub desc: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayerFile {
    #[serde(default)]
    pub clear_defaults: bool,
    #[serde(default)]
    pub prepend_keymap: Vec<BindingFile>,
    #[serde(default)]
    pub append_keymap: Vec<BindingFile>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NormalFile {
    #[serde(default)]
    pub collections: LayerFile,
    #[serde(default)]
    pub entries: LayerFile,
    #[serde(default)]
    pub preview: LayerFile,
}

/// 아직 배선되지 않은 모드도 필드로 선언한다. 그래야 `[help]`는 파싱을 통과해
/// 후처리에서 "미구현 레이어" 경고가 되고, `[normal.entrys]` 같은 오타는
/// `deny_unknown_fields`가 에러로 잡는다.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeymapFile {
    #[serde(default)]
    pub normal: NormalFile,
    #[serde(default)]
    pub help: Option<LayerFile>,
    #[serde(default)]
    pub search: Option<LayerFile>,
    #[serde(default)]
    pub sort_menu: Option<LayerFile>,
    #[serde(default)]
    pub export_menu: Option<LayerFile>,
    #[serde(default)]
    pub settings: Option<LayerFile>,
    #[serde(default)]
    pub file_picker: Option<LayerFile>,
    #[serde(default)]
    pub fetch_preview: Option<LayerFile>,
    #[serde(default)]
    pub search_result_picker: Option<LayerFile>,
    #[serde(default)]
    pub context_menu: Option<LayerFile>,
    #[serde(default)]
    pub confirm: Option<LayerFile>,
    #[serde(default)]
    pub picker: Option<LayerFile>,
}

pub fn keymap_path() -> std::path::PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("bibox")
        .join("keymap.toml")
}

enum BindingError {
    BadKey(String),
    UnknownPlugin(String),
}

fn to_binding(bf: &BindingFile, commands: &crate::plugin::PluginCommands) -> Result<Binding, BindingError> {
    let keys: Result<Vec<KeyPress>, KeyParseError> = match &bf.on {
        OnField::One(s) => vec![parse_key(s)].into_iter().collect(),
        OnField::Many(v) => v.iter().map(|s| parse_key(s)).collect(),
    };
    let keys = keys.map_err(|e| BindingError::BadKey(e.token))?;
    let items: Vec<&RunItem> = match &bf.run {
        RunField::One(a) => vec![a],
        RunField::Many(v) => v.iter().collect(),
    };
    let mut actions = Vec::with_capacity(items.len());
    for it in items {
        match it {
            RunItem::Builtin(a) => actions.push(*a),
            RunItem::Plugin(name) => match commands.find(name) {
                Some(id) => actions.push(Action::Plugin(id)),
                None => return Err(BindingError::UnknownPlugin(name.clone())),
            },
        }
    }
    Ok(Binding { keys, actions, desc: bf.desc.clone() })
}

/// 이 레이어에 들어갈 플러그인 기본 바인딩.
///
/// 내장 기본과 같거나 접두사 관계면 버리고 경고한다(내장이 이기므로 조용히 죽는 키를 알려야 한다).
/// 사용자 바인딩(prepend/append)과는 접두사 관계일 때만 버린다. 그대로 두면 `check_layer`가
/// 접두사 충돌 에러를 내어 플러그인 설치가 사용자 키맵 전체를 폴백시킨다. 같은 키는 병합 순서가
/// 정하는 정상 그림자라 아무 말도 하지 않는다. 플러그인끼리 겹치면 먼저 로드된(알파벳순) 쪽이 남는다.
fn plugin_default_bindings(
    commands: &crate::plugin::PluginCommands,
    layer: LayerId,
    layer_name: &str,
    builtin: &[Binding],
    user: &[&Binding],
    warnings: &mut Vec<KeymapProblem>,
) -> Vec<Binding> {
    let mut out: Vec<Binding> = Vec::new();
    for (id, cmd) in commands.iter() {
        let Some(keys) = &cmd.key else { continue };
        if !cmd.layers.contains(&layer) {
            continue;
        }
        let same = |other: &[KeyPress]| other == keys.as_slice();
        let prefix_related = |other: &[KeyPress]| !same(other) && (other.starts_with(keys) || keys.starts_with(other));
        let shadowed_by = builtin
            .iter()
            .find(|b| same(&b.keys) || prefix_related(&b.keys))
            .or_else(|| user.iter().copied().find(|b| prefix_related(&b.keys)));
        if let Some(b) = shadowed_by {
            let by = match b.actions.first() {
                Some(Action::Plugin(pid)) => commands.get(*pid).map(|c| c.full_name()).unwrap_or_default(),
                Some(a) => format!("{} ({:?})", render_seq(&b.keys), a),
                None => render_seq(&b.keys),
            };
            warnings.push(KeymapProblem::PluginKeyShadowed {
                plugin: cmd.plugin.clone(),
                command: cmd.id.clone(),
                layer: layer_name.to_string(),
                key: render_seq(keys),
                by,
            });
            continue;
        }
        if let Some(b) = out.iter().find(|b| same(&b.keys) || prefix_related(&b.keys)) {
            let by_plugin = match b.actions.first() {
                Some(Action::Plugin(pid)) => commands.get(*pid).map(|c| c.plugin.clone()).unwrap_or_default(),
                _ => String::new(),
            };
            warnings.push(KeymapProblem::PluginKeyTaken {
                plugin: cmd.plugin.clone(),
                command: cmd.id.clone(),
                layer: layer_name.to_string(),
                key: render_seq(keys),
                by_plugin,
            });
            continue;
        }
        out.push(Binding { keys: keys.clone(), actions: vec![Action::Plugin(id)], desc: Some(cmd.desc.clone()) });
    }
    out
}

/// 유효 목록은 prepend ++ 내장 기본 ++ 플러그인 기본 ++ append 다. 조회는 첫 일치.
/// `clear_defaults`는 내장과 플러그인 기본을 함께 버린다.
/// 잘못된 키 표기를 만나면 그 바인딩만 건너뛰고 계속한다. 조기 반환하면 한 번에
/// 하나씩만 보고하게 되어 사용자가 여러 번 고쳐야 한다.
fn merge_layer(
    defaults: Layer,
    lf: &LayerFile,
    layer_id: LayerId,
    layer_name: &str,
    commands: &crate::plugin::PluginCommands,
    errors: &mut Vec<KeymapProblem>,
    warnings: &mut Vec<KeymapProblem>,
) -> Layer {
    let mut parse_list = |list: &[BindingFile]| -> Vec<Binding> {
        let mut out = Vec::new();
        for bf in list {
            match to_binding(bf, commands) {
                Ok(b) => out.push(b),
                Err(BindingError::BadKey(token)) => errors.push(KeymapProblem::BadKey {
                    layer: layer_name.to_string(),
                    token,
                }),
                Err(BindingError::UnknownPlugin(name)) => warnings.push(KeymapProblem::UnknownPluginCommand {
                    layer: layer_name.to_string(),
                    name,
                }),
            }
        }
        out
    };
    let prepend = parse_list(&lf.prepend_keymap);
    let append = parse_list(&lf.append_keymap);
    let base: Vec<Binding> = if lf.clear_defaults { Vec::new() } else { defaults.bindings };
    let plugin = if lf.clear_defaults {
        Vec::new()
    } else {
        let user: Vec<&Binding> = prepend.iter().chain(append.iter()).collect();
        plugin_default_bindings(commands, layer_id, layer_name, &base, &user, warnings)
    };

    let mut out = prepend;
    out.extend(base);
    out.extend(plugin);
    out.extend(append);
    Layer { bindings: out }
}

pub fn merge(
    defaults: Keymap,
    file: &KeymapFile,
    commands: &crate::plugin::PluginCommands,
    errors: &mut Vec<KeymapProblem>,
    warnings: &mut Vec<KeymapProblem>,
) -> Keymap {
    Keymap {
        collections: merge_layer(defaults.collections, &file.normal.collections, LayerId::Collections, "normal.collections", commands, errors, warnings),
        entries: merge_layer(defaults.entries, &file.normal.entries, LayerId::Entries, "normal.entries", commands, errors, warnings),
        preview: merge_layer(defaults.preview, &file.normal.preview, LayerId::Preview, "normal.preview", commands, errors, warnings),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum KeymapProblem {
    Syntax { detail: String },
    UnknownLayer { detail: String },
    UnknownAction { detail: String },
    BadKey { layer: String, token: String },
    PrefixConflict { layer: String, shorter: String, longer: String },
    DuplicateBinding { layer: String, keys: String },
    LayerNotWired { layer: String },
    UnknownPluginCommand { layer: String, name: String },
    PluginKeyShadowed { plugin: String, command: String, layer: String, key: String, by: String },
    PluginKeyTaken { plugin: String, command: String, layer: String, key: String, by_plugin: String },
}

pub struct LoadReport {
    pub keymap: Keymap,
    pub errors: Vec<KeymapProblem>,
    pub warnings: Vec<KeymapProblem>,
}

pub fn render_seq(keys: &[KeyPress]) -> String {
    keys.iter().map(|k| render_key(*k)).collect::<Vec<_>>().join("")
}

/// 한 소스 리스트(prepend 또는 append) 안의 중복만 경고한다.
///
/// 병합된 목록에서 중복을 찾으면 안 된다. `prepend_keymap`으로 기본 바인딩을
/// 덮는 것이 이 기능의 존재 이유인데, 병합 후에는 그것이 중복으로 보인다.
/// 리맵할 때마다 경고가 뜨면 기능을 쓸 수 없다.
fn check_duplicates(name: &str, list: &[BindingFile], commands: &crate::plugin::PluginCommands, warnings: &mut Vec<KeymapProblem>) {
    let keys: Vec<Vec<KeyPress>> = list
        .iter()
        .filter_map(|bf| to_binding(bf, commands).ok().map(|b| b.keys))
        .collect();
    for (i, a) in keys.iter().enumerate() {
        if keys.iter().take(i).any(|b| b == a) {
            warnings.push(KeymapProblem::DuplicateBinding {
                layer: name.to_string(),
                keys: render_seq(a),
            });
        }
    }
}

/// 접두사 충돌은 병합된 목록에서 찾는다. 사용자가 `g`를 바인딩하면 기본 `gg`가
/// 영원히 안 걸리는데, 그 충돌은 소스 리스트 경계를 넘어서 생긴다.
fn check_layer(
    name: &str,
    layer: &Layer,
    errors: &mut Vec<KeymapProblem>,
    _warnings: &mut Vec<KeymapProblem>,
) {
    for (i, a) in layer.bindings.iter().enumerate() {
        for b in layer.bindings.iter().skip(i + 1) {
            if a.keys == b.keys {
                // 병합으로 생긴 그림자다. 정상 동작이므로 아무 말도 하지 않는다.
            } else if b.keys.starts_with(&a.keys) {
                errors.push(KeymapProblem::PrefixConflict {
                    layer: name.to_string(),
                    shorter: render_seq(&a.keys),
                    longer: render_seq(&b.keys),
                });
            } else if a.keys.starts_with(&b.keys) {
                errors.push(KeymapProblem::PrefixConflict {
                    layer: name.to_string(),
                    shorter: render_seq(&b.keys),
                    longer: render_seq(&a.keys),
                });
            }
        }
    }
}

pub fn load_keymap_from_str(s: &str, commands: &crate::plugin::PluginCommands) -> LoadReport {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();

    // 파일이 없거나 비어 있어도 플러그인 기본 키는 들어가야 하므로 폴백도 merge를 거친다.
    let fallback = |warnings: &mut Vec<KeymapProblem>| {
        let mut ignored = Vec::new();
        merge(default_keymap(), &KeymapFile::default(), commands, &mut ignored, warnings)
    };

    // ① 파싱. 문법, 알 수 없는 레이어, 알 수 없는 액션이 여기서 갈린다.
    let file: KeymapFile = match toml::from_str(s) {
        Ok(f) => f,
        Err(e) => {
            let detail = e.to_string();
            let problem = if detail.contains("unknown field") {
                KeymapProblem::UnknownLayer { detail }
            } else if detail.contains("unknown variant") || detail.contains("did not match any variant") {
                KeymapProblem::UnknownAction { detail }
            } else {
                KeymapProblem::Syntax { detail }
            };
            let keymap = fallback(&mut warnings);
            return LoadReport { keymap, errors: vec![problem], warnings };
        }
    };

    // ② 아직 배선되지 않은 레이어는 경고다.
    for (name, present) in [
        ("help", file.help.is_some()),
        ("search", file.search.is_some()),
        ("sort_menu", file.sort_menu.is_some()),
        ("export_menu", file.export_menu.is_some()),
        ("settings", file.settings.is_some()),
        ("file_picker", file.file_picker.is_some()),
        ("fetch_preview", file.fetch_preview.is_some()),
        ("search_result_picker", file.search_result_picker.is_some()),
        ("context_menu", file.context_menu.is_some()),
        ("confirm", file.confirm.is_some()),
        ("picker", file.picker.is_some()),
    ] {
        if present {
            warnings.push(KeymapProblem::LayerNotWired { layer: name.to_string() });
        }
    }

    // ③ 병합. 키 표기 오류를 전부 모은다.
    let merged = merge(default_keymap(), &file, commands, &mut errors, &mut warnings);

    // ④ 소스 리스트 안의 중복(경고)과 병합 목록의 접두사 충돌(에러)
    for (name, lf) in [
        ("normal.collections", &file.normal.collections),
        ("normal.entries", &file.normal.entries),
        ("normal.preview", &file.normal.preview),
    ] {
        check_duplicates(name, &lf.prepend_keymap, commands, &mut warnings);
        check_duplicates(name, &lf.append_keymap, commands, &mut warnings);
    }
    check_layer("normal.collections", &merged.collections, &mut errors, &mut warnings);
    check_layer("normal.entries", &merged.entries, &mut errors, &mut warnings);
    check_layer("normal.preview", &merged.preview, &mut errors, &mut warnings);

    // ⑤ 에러가 하나라도 있으면 파일을 통째로 버린다. 플러그인 기본 키는 남는다.
    if errors.is_empty() {
        LoadReport { keymap: merged, errors, warnings }
    } else {
        // 폴백에서 나오는 플러그인 경고는 위에서 이미 한 번 나왔으므로 버린다.
        let mut dup = Vec::new();
        let keymap = fallback(&mut dup);
        LoadReport { keymap, errors, warnings }
    }
}

pub fn load_keymap(commands: &crate::plugin::PluginCommands) -> LoadReport {
    match std::fs::read_to_string(keymap_path()) {
        Ok(s) => load_keymap_from_str(&s, commands),
        Err(_) => load_keymap_from_str("", commands),
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

    fn kp(s: &str) -> KeyPress {
        parse_key(s).unwrap()
    }

    #[test]
    fn resolve_runs_a_single_key_binding() {
        let layer = &default_keymap().entries;
        assert_eq!(resolve(layer, &[], kp("j")), Resolution::Run(vec![Action::EntryDown]));
    }

    #[test]
    fn resolve_waits_on_a_prefix_then_completes() {
        let layer = &default_keymap().entries;
        assert_eq!(resolve(layer, &[], kp("g")), Resolution::Pending);
        assert_eq!(resolve(layer, &[kp("g")], kp("g")), Resolution::Run(vec![Action::EntryTop]));
    }

    #[test]
    fn resolve_drops_a_pending_sequence_that_matches_nothing() {
        let layer = &default_keymap().entries;
        assert_eq!(resolve(layer, &[kp("g")], kp("x")), Resolution::Unbound);
    }

    #[test]
    fn resolve_returns_unbound_for_an_unbound_key() {
        let layer = &default_keymap().entries;
        assert_eq!(resolve(layer, &[], kp("Z")), Resolution::Unbound);
    }

    #[test]
    fn space_runs_two_actions_in_order() {
        let layer = &default_keymap().entries;
        assert_eq!(
            resolve(layer, &[], kp("<Space>")),
            Resolution::Run(vec![Action::ToggleSelect, Action::EntryDown])
        );
    }

    #[test]
    fn the_same_key_means_different_things_in_different_layers() {
        let km = default_keymap();
        assert_eq!(resolve(&km.entries, &[], kp("j")), Resolution::Run(vec![Action::EntryDown]));
        assert_eq!(resolve(&km.collections, &[], kp("j")), Resolution::Run(vec![Action::CollectionDown]));
        assert_eq!(resolve(&km.preview, &[], kp("j")), Resolution::Run(vec![Action::PreviewScrollDown]));
    }

    /// 회귀 가드. 옛 handle_normal이 반응하던 모든 키가 같은 액션으로 가는지
    /// 전수로 단언한다. 개수를 고정하지 않고 표를 통째로 대조한다.
    #[test]
    fn default_keymap_reproduces_todays_bindings() {
        let km = default_keymap();

        let global: &[(&str, &[Action])] = &[
            ("q", &[Action::Quit]),
            ("<Esc>", &[Action::Cancel]),
            ("<C-c>", &[Action::Quit]),
            ("<C-z>", &[Action::Undo]),
            ("<C-y>", &[Action::Redo]),
            ("<Tab>", &[Action::NextPreviewTab]),
            ("/", &[Action::Search]),
            ("y", &[Action::CopyCitekey]),
            ("Y", &[Action::CopyCitation]),
            ("o", &[Action::OpenPdf]),
            ("w", &[Action::OpenWeb]),
            ("f", &[Action::FetchMetadata]),
            ("e", &[Action::ExportMenu]),
            ("d", &[Action::Delete]),
            ("?", &[Action::Help]),
            ("`", &[Action::Help]),
            ("~", &[Action::Help]),
            ("<F1>", &[Action::Help]),
            ("N", &[Action::EditNote]),
            ("s", &[Action::SortMenu]),
            ("c", &[Action::Collections]),
            ("t", &[Action::Tags]),
            ("A", &[Action::AttachPdf]),
            (",", &[Action::Settings]),
            ("<Enter>", &[Action::Noop]),
            // 패널로 바로. 숫자 접두사(5j)를 없애고 얻은 자리(2026-09-15)
            ("1", &[Action::FocusCollections]),
            ("2", &[Action::FocusEntries]),
            ("3", &[Action::FocusPreview]),
        ];
        for (layer, name) in [
            (&km.collections, "collections"),
            (&km.entries, "entries"),
            (&km.preview, "preview"),
        ] {
            for (key, actions) in global {
                assert_eq!(
                    resolve(layer, &[], kp(key)),
                    Resolution::Run(actions.to_vec()),
                    "layer {} key {}", name, key
                );
            }
        }

        let per_layer: &[(&str, &[(&str, Action)])] = &[
            ("entries", &[
                ("h", Action::FocusCollections), ("<Left>", Action::FocusCollections),
                ("l", Action::FocusPreview), ("<Right>", Action::FocusPreview),
                ("j", Action::EntryDown), ("<Down>", Action::EntryDown),
                ("k", Action::EntryUp), ("<Up>", Action::EntryUp),
                ("G", Action::EntryBottom),
                ("<C-d>", Action::EntryHalfPageDown), ("<C-u>", Action::EntryHalfPageUp),
                ("H", Action::EntryScreenTop), ("M", Action::EntryScreenMiddle),
                ("L", Action::EntryScreenBottom), ("V", Action::SelectAll),
            ]),
            ("collections", &[
                ("l", Action::FocusEntries), ("<Right>", Action::FocusEntries),
                ("j", Action::CollectionDown), ("<Down>", Action::CollectionDown),
                ("k", Action::CollectionUp), ("<Up>", Action::CollectionUp),
                ("G", Action::CollectionBottom),
                ("<C-d>", Action::CollectionHalfPageDown), ("<C-u>", Action::CollectionHalfPageUp),
            ]),
            ("preview", &[
                ("h", Action::PrevTabOrFocusEntries), ("<Left>", Action::PrevTabOrFocusEntries),
                ("l", Action::NextTab), ("<Right>", Action::NextTab),
                ("j", Action::PreviewScrollDown), ("<Down>", Action::PreviewScrollDown),
                ("k", Action::PreviewScrollUp), ("<Up>", Action::PreviewScrollUp),
                ("G", Action::PreviewBottom),
                ("<C-d>", Action::PreviewHalfPageDown), ("<C-u>", Action::PreviewHalfPageUp),
                ("n", Action::TabNextPage), ("p", Action::TabPrevPage),
                ("+", Action::TabZoomIn), ("=", Action::TabZoomIn), ("-", Action::TabZoomOut),
                ("0", Action::TabZoomReset), ("H", Action::TabPanLeft), ("L", Action::TabPanRight),
            ]),
        ];
        for (name, rows) in per_layer {
            let layer = match *name {
                "entries" => &km.entries,
                "collections" => &km.collections,
                _ => &km.preview,
            };
            for (key, action) in *rows {
                assert_eq!(
                    resolve(layer, &[], kp(key)),
                    Resolution::Run(vec![*action]),
                    "layer {} key {}", name, key
                );
            }
        }

        for (layer, expected) in [
            (&km.collections, Action::CollectionTop),
            (&km.entries, Action::EntryTop),
            (&km.preview, Action::PreviewTop),
        ] {
            assert_eq!(resolve(layer, &[kp("g")], kp("g")), Resolution::Run(vec![expected]));
        }

        // 컬렉션 패널의 h는 오늘 아무 일도 하지 않으므로 바인딩하지 않는다.
        assert_eq!(resolve(&km.collections, &[], kp("h")), Resolution::Unbound);

        // 엔트리 전용 키는 다른 레이어에 없다. 미리보기의 H/L은 플러그인 탭의 pan이라 예외.
        for key in ["H", "M", "L", "V", "<Space>"] {
            assert_eq!(resolve(&km.collections, &[], kp(key)), Resolution::Unbound, "collections {}", key);
        }
        for key in ["M", "V", "<Space>"] {
            assert_eq!(resolve(&km.preview, &[], kp(key)), Resolution::Unbound, "preview {}", key);
        }
    }

    #[test]
    fn shift_is_dropped_for_character_keys() {
        // 터미널에 따라 대문자에 SHIFT가 실려 온다. 정규화하지 않으면 G 바인딩이
        // 어떤 터미널에서만 걸린다. tmux에서 실제로 G가 먹히지 않아 발견했다.
        let shifted = KeyPress::new(KeyCode::Char('G'), KeyModifiers::SHIFT);
        assert_eq!(shifted, kp("G"));
        assert_eq!(shifted.mods, KeyModifiers::NONE);
    }

    #[test]
    fn shift_survives_on_non_character_keys() {
        let k = KeyPress::new(KeyCode::Tab, KeyModifiers::SHIFT);
        assert_eq!(k.mods, KeyModifiers::SHIFT);
        assert_ne!(k, kp("<Tab>"));
    }

    #[test]
    fn control_still_distinguishes_a_character_key() {
        assert_ne!(KeyPress::new(KeyCode::Char('d'), KeyModifiers::CONTROL), kp("d"));
        assert_eq!(KeyPress::new(KeyCode::Char('d'), KeyModifiers::CONTROL), kp("<C-d>"));
    }

    fn parse_file(s: &str) -> KeymapFile {
        toml::from_str(s).unwrap()
    }

    #[test]
    fn prepend_wins_over_a_default_binding() {
        let file = parse_file(r#"
            [normal.entries]
            prepend_keymap = [ { on = "j", run = "entry_up" } ]
        "#);
        let km = merge(default_keymap(), &file, &crate::plugin::PluginCommands::default(), &mut Vec::new(), &mut Vec::new());
        assert_eq!(resolve(&km.entries, &[], kp("j")), Resolution::Run(vec![Action::EntryUp]));
        assert_eq!(resolve(&km.preview, &[], kp("j")), Resolution::Run(vec![Action::PreviewScrollDown]));
    }

    #[test]
    fn append_adds_a_key_the_defaults_do_not_have() {
        let file = parse_file(r#"
            [normal.entries]
            append_keymap = [ { on = "Z", run = "quit" } ]
        "#);
        let km = merge(default_keymap(), &file, &crate::plugin::PluginCommands::default(), &mut Vec::new(), &mut Vec::new());
        assert_eq!(resolve(&km.entries, &[], kp("Z")), Resolution::Run(vec![Action::Quit]));
        assert_eq!(resolve(&km.entries, &[], kp("j")), Resolution::Run(vec![Action::EntryDown]));
    }

    #[test]
    fn clear_defaults_drops_every_default_in_that_layer_only() {
        let file = parse_file(r#"
            [normal.entries]
            clear_defaults = true
            prepend_keymap = [ { on = "x", run = "quit" } ]
        "#);
        let km = merge(default_keymap(), &file, &crate::plugin::PluginCommands::default(), &mut Vec::new(), &mut Vec::new());
        assert_eq!(resolve(&km.entries, &[], kp("x")), Resolution::Run(vec![Action::Quit]));
        assert_eq!(resolve(&km.entries, &[], kp("j")), Resolution::Unbound);
        assert_eq!(resolve(&km.collections, &[], kp("j")), Resolution::Run(vec![Action::CollectionDown]));
    }

    #[test]
    fn on_accepts_a_sequence_and_run_accepts_a_list() {
        let file = parse_file(r#"
            [normal.entries]
            prepend_keymap = [ { on = ["g", "b"], run = ["toggle_select", "entry_down"] } ]
        "#);
        let km = merge(default_keymap(), &file, &crate::plugin::PluginCommands::default(), &mut Vec::new(), &mut Vec::new());
        assert_eq!(resolve(&km.entries, &[kp("g")], kp("b")),
                   Resolution::Run(vec![Action::ToggleSelect, Action::EntryDown]));
    }

    #[test]
    fn noop_disables_a_default_key() {
        let file = parse_file(r#"
            [normal.entries]
            prepend_keymap = [ { on = "d", run = "noop" } ]
        "#);
        let km = merge(default_keymap(), &file, &crate::plugin::PluginCommands::default(), &mut Vec::new(), &mut Vec::new());
        assert_eq!(resolve(&km.entries, &[], kp("d")), Resolution::Run(vec![Action::Noop]));
    }

    #[test]
    fn a_binding_desc_overrides_the_action_description() {
        let file = parse_file(r#"
            [normal.entries]
            prepend_keymap = [ { on = "j", run = "entry_down", desc = "한 칸 아래" } ]
        "#);
        let km = merge(default_keymap(), &file, &crate::plugin::PluginCommands::default(), &mut Vec::new(), &mut Vec::new());
        let bind = km.entries.bindings.iter().find(|b| b.keys == vec![kp("j")]).unwrap();
        assert_eq!(bind.desc.as_deref(), Some("한 칸 아래"));
    }

    fn check(s: &str) -> (Vec<KeymapProblem>, Vec<KeymapProblem>) {
        let r = load_keymap_from_str(s, &crate::plugin::PluginCommands::default());
        (r.errors, r.warnings)
    }

    #[test]
    fn the_default_keymap_itself_validates_clean() {
        // 기본 키맵에 접두사 충돌이나 중복이 있으면 사용자 파일이 없어도 경고가 뜬다.
        let r = load_keymap_from_str("", &crate::plugin::PluginCommands::default());
        assert!(r.errors.is_empty(), "default keymap has errors: {:?}", r.errors);
        assert!(r.warnings.is_empty(), "default keymap has warnings: {:?}", r.warnings);
    }

    #[test]
    fn a_toml_syntax_error_is_reported_and_falls_back_to_defaults() {
        let r = load_keymap_from_str("[normal.entries]\nprepend_keymap = [\n", &crate::plugin::PluginCommands::default());
        assert!(!r.errors.is_empty());
        assert_eq!(resolve(&r.keymap.entries, &[], kp("j")), Resolution::Run(vec![Action::EntryDown]));
    }

    #[test]
    fn an_unknown_layer_name_is_an_error() {
        let (errors, _) = check("[normal.entrys]\n");
        assert!(!errors.is_empty(), "a typo'd layer name must be reported");
    }

    #[test]
    fn an_unknown_action_name_is_an_error() {
        let (errors, _) = check(r#"
            [normal.entries]
            prepend_keymap = [ { on = "x", run = "opne_pdf" } ]
        "#);
        assert!(!errors.is_empty(), "an unknown action name must be reported");
    }

    #[test]
    fn a_bad_key_notation_is_an_error() {
        let (errors, _) = check(r#"
            [normal.preview]
            prepend_keymap = [ { on = "<Ctrl-d>", run = "quit" } ]
        "#);
        assert!(
            errors.iter().any(|e| matches!(e, KeymapProblem::BadKey { token, .. } if token == "<Ctrl-d>")),
            "expected the offending token to be named, got {:?}", errors
        );
    }

    #[test]
    fn a_prefix_conflict_is_an_error() {
        let (errors, _) = check(r#"
            [normal.entries]
            prepend_keymap = [ { on = "g", run = "quit" } ]
        "#);
        assert!(
            errors.iter().any(|e| matches!(e, KeymapProblem::PrefixConflict { .. })),
            "g conflicts with the default gg, got {:?}", errors
        );
    }

    #[test]
    fn a_duplicate_binding_in_one_list_is_a_warning_and_the_first_wins() {
        let r = load_keymap_from_str(r#"
            [normal.entries]
            prepend_keymap = [
              { on = "x", run = "quit" },
              { on = "x", run = "undo" },
            ]
        "#, &crate::plugin::PluginCommands::default());
        assert!(r.errors.is_empty(), "a duplicate must not discard the file, got {:?}", r.errors);
        assert!(!r.warnings.is_empty());
        assert_eq!(resolve(&r.keymap.entries, &[], kp("x")), Resolution::Run(vec![Action::Quit]));
    }

    #[test]
    fn a_not_yet_wired_layer_is_a_warning_and_the_file_still_applies() {
        let r = load_keymap_from_str(r#"
            [help]
            prepend_keymap = [ { on = "x", run = "quit" } ]

            [normal.entries]
            prepend_keymap = [ { on = "j", run = "entry_up" } ]
        "#, &crate::plugin::PluginCommands::default());
        assert!(r.errors.is_empty(), "got {:?}", r.errors);
        assert!(!r.warnings.is_empty());
        assert_eq!(resolve(&r.keymap.entries, &[], kp("j")), Resolution::Run(vec![Action::EntryUp]));
    }

    #[test]
    fn every_error_discards_the_whole_file() {
        let r = load_keymap_from_str(r#"
            [normal.entries]
            prepend_keymap = [
              { on = "j", run = "entry_up" },
              { on = "x", run = "opne_pdf" },
            ]
        "#, &crate::plugin::PluginCommands::default());
        assert!(!r.errors.is_empty());
        // 좋은 바인딩까지 전부 버리고 기본값으로 간다.
        assert_eq!(resolve(&r.keymap.entries, &[], kp("j")), Resolution::Run(vec![Action::EntryDown]));
    }

    #[test]
    fn every_bad_key_notation_is_collected_not_just_the_first() {
        // TOML 파싱 에러는 serde가 첫 에러에서 멈춰 하나만 나오지만,
        // 파싱 이후에 나오는 키 표기 오류는 전부 모아야 한 번에 고칠 수 있다.
        let (errors, _) = check(r#"
            [normal.entries]
            prepend_keymap = [ { on = "<Ctrl-d>", run = "quit" } ]

            [normal.preview]
            prepend_keymap = [ { on = "<Meta-x>", run = "quit" } ]
        "#);
        let tokens: Vec<&str> = errors.iter().filter_map(|e| match e {
            KeymapProblem::BadKey { token, .. } => Some(token.as_str()),
            _ => None,
        }).collect();
        assert!(tokens.contains(&"<Ctrl-d>"), "got {:?}", tokens);
        assert!(tokens.contains(&"<Meta-x>"), "got {:?}", tokens);
    }

    #[test]
    fn overriding_a_default_is_not_a_duplicate() {
        // prepend로 기본값을 덮는 것이 이 기능의 존재 이유다. 경고가 뜨면 안 된다.
        // tmux에서 j를 리맵했더니 경고 화면이 떠서 발견했다.
        let r = load_keymap_from_str(r#"
            [normal.entries]
            prepend_keymap = [ { on = "j", run = "entry_up" } ]
        "#, &crate::plugin::PluginCommands::default());
        assert!(r.errors.is_empty(), "got {:?}", r.errors);
        assert!(r.warnings.is_empty(), "overriding a default must be silent, got {:?}", r.warnings);
        assert_eq!(resolve(&r.keymap.entries, &[], kp("j")), Resolution::Run(vec![Action::EntryUp]));
    }

    // ── 플러그인 명령 ──

    fn cmds(specs: &[(&str, &str, Option<&str>, &[LayerId])]) -> crate::plugin::PluginCommands {
        use crate::plugin::manifest::{Command, Manifest};
        let mut by_plugin: std::collections::BTreeMap<String, Vec<Command>> = Default::default();
        for (plugin, id, key, layers) in specs {
            by_plugin.entry(plugin.to_string()).or_default().push(Command {
                id: id.to_string(),
                desc: format!("{} desc", id),
                key: key.map(|k| k.split(' ').map(|t| parse_key(t).unwrap()).collect()),
                layers: layers.to_vec(),
                menus: vec![],
            });
        }
        let manifests: Vec<Manifest> = by_plugin
            .into_iter()
            .map(|(name, commands)| Manifest {
                name,
                version: None,
                description: None,
                run: vec!["sh".into()],
                commands,
                activation: crate::plugin::manifest::Activation::Lazy,
                fields: vec![],
                views: vec![],
                events: vec![],
                cli: None,
                settings: vec![],
                builtin: None,
                dir: std::path::PathBuf::from("/tmp"),
                guide: None,
            })
            .collect();
        let env = crate::plugin::PluginEnv {
            bin: "/bin/true".into(), config_dir: "/tmp".into(), db: "/tmp/db.json".into(),
            notes: "/tmp/n".into(), pdfs: "/tmp/p".into(), home: None, extra: Default::default(),
        };
        crate::plugin::PluginHost::new(manifests, Default::default(), env).commands().clone()
    }

    const ALL: &[LayerId] = &[LayerId::Collections, LayerId::Entries, LayerId::Preview];

    #[test]
    fn a_plugin_default_key_lands_in_its_layers_with_its_desc() {
        let c = cmds(&[("tidy", "run", Some("="), &[LayerId::Entries])]);
        let r = load_keymap_from_str("", &c);
        assert!(r.errors.is_empty() && r.warnings.is_empty(), "{:?} {:?}", r.errors, r.warnings);
        let id = c.find("tidy.run").unwrap();
        let hit = r.keymap.entries.bindings.iter().find(|b| b.actions == vec![Action::Plugin(id)]).unwrap();
        assert_eq!(hit.keys, vec![parse_key("=").unwrap()]);
        assert_eq!(hit.desc.as_deref(), Some("run desc"));
        assert!(!r.keymap.collections.bindings.iter().any(|b| b.actions == vec![Action::Plugin(id)]));
    }

    #[test]
    fn a_user_can_bind_a_plugin_command_by_its_full_name() {
        let c = cmds(&[("tidy", "run", None, ALL)]);
        let toml = "[normal.entries]\nprepend_keymap = [{ on = \"<C-t>\", run = \"tidy.run\" }]\n";
        let r = load_keymap_from_str(toml, &c);
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        let id = c.find("tidy.run").unwrap();
        assert_eq!(resolve(&r.keymap.entries, &[], parse_key("<C-t>").unwrap()), Resolution::Run(vec![Action::Plugin(id)]));
    }

    #[test]
    fn a_plugin_command_can_sit_in_a_run_list_next_to_builtins() {
        let c = cmds(&[("tidy", "run", None, ALL)]);
        let toml = "[normal.entries]\nprepend_keymap = [{ on = \"x\", run = [\"tidy.run\", \"entry_down\"] }]\n";
        let r = load_keymap_from_str(toml, &c);
        let id = c.find("tidy.run").unwrap();
        assert_eq!(resolve(&r.keymap.entries, &[], parse_key("x").unwrap()), Resolution::Run(vec![Action::Plugin(id), Action::EntryDown]));
    }

    #[test]
    fn an_unknown_plugin_reference_is_a_warning_that_drops_only_that_binding() {
        let c = cmds(&[]);
        let toml = "[normal.entries]\nprepend_keymap = [{ on = \"x\", run = \"gone.cmd\" }, { on = \"y\", run = \"entry_down\" }]\n";
        let r = load_keymap_from_str(toml, &c);
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        assert_eq!(r.warnings, vec![KeymapProblem::UnknownPluginCommand { layer: "normal.entries".into(), name: "gone.cmd".into() }]);
        assert_eq!(resolve(&r.keymap.entries, &[], parse_key("y").unwrap()), Resolution::Run(vec![Action::EntryDown]));
        assert_eq!(resolve(&r.keymap.entries, &[], parse_key("x").unwrap()), Resolution::Unbound);
    }

    #[test]
    fn an_unknown_builtin_action_name_is_still_an_error() {
        let c = cmds(&[]);
        let r = load_keymap_from_str("[normal.entries]\nprepend_keymap = [{ on = \"x\", run = \"entry_dwon\" }]\n", &c);
        assert!(matches!(r.errors[0], KeymapProblem::UnknownAction { .. }));
    }

    #[test]
    fn merge_order_is_user_prepend_then_builtin_then_plugin_then_user_append() {
        let c = cmds(&[("p", "cmd", Some("j"), &[LayerId::Entries]), ("p", "cmd2", Some("z"), &[LayerId::Entries])]);
        let toml = "[normal.entries]\nprepend_keymap = [{ on = \"z\", run = \"entry_up\" }]\nappend_keymap = [{ on = \"z\", run = \"entry_top\" }]\n";
        let r = load_keymap_from_str(toml, &c);
        // j: 플러그인이 내장 j를 덮으려 했지만 내장이 이긴다 (경고)
        assert_eq!(resolve(&r.keymap.entries, &[], parse_key("j").unwrap()), Resolution::Run(vec![Action::EntryDown]));
        // z: 사용자 prepend가 플러그인을 이긴다 (경고 없음. 정상 그림자)
        assert_eq!(resolve(&r.keymap.entries, &[], parse_key("z").unwrap()), Resolution::Run(vec![Action::EntryUp]));
        assert_eq!(r.warnings.len(), 1, "{:?}", r.warnings);
        assert!(matches!(&r.warnings[0], KeymapProblem::PluginKeyShadowed { plugin, command, key, .. } if plugin == "p" && command == "cmd" && key == "j"));
    }

    #[test]
    fn a_plugin_key_that_is_a_prefix_of_an_existing_binding_is_dropped_not_an_error() {
        let c = cmds(&[("p", "cmd", Some("g"), &[LayerId::Entries])]); // 내장 "g g"의 접두사
        let r = load_keymap_from_str("", &c);
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        assert!(matches!(&r.warnings[0], KeymapProblem::PluginKeyShadowed { key, by, .. } if key == "g" && by.contains("gg")));
        assert_eq!(resolve(&r.keymap.entries, &[], parse_key("g").unwrap()), Resolution::Pending);
    }

    #[test]
    fn a_plugin_key_that_extends_a_user_binding_is_dropped_too() {
        let c = cmds(&[("p", "cmd", Some("x y"), &[LayerId::Entries])]);
        let toml = "[normal.entries]\nappend_keymap = [{ on = \"x\", run = \"entry_down\" }]\n";
        let r = load_keymap_from_str(toml, &c);
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        assert!(matches!(&r.warnings[0], KeymapProblem::PluginKeyShadowed { key, .. } if key == "xy"));
    }

    #[test]
    fn two_plugins_on_the_same_key_keep_the_first_and_warn() {
        let c = cmds(&[("a", "cmd", Some("="), &[LayerId::Entries]), ("b", "cmd", Some("="), &[LayerId::Entries])]);
        let r = load_keymap_from_str("", &c);
        let a = c.find("a.cmd").unwrap();
        assert_eq!(resolve(&r.keymap.entries, &[], parse_key("=").unwrap()), Resolution::Run(vec![Action::Plugin(a)]));
        assert!(matches!(&r.warnings[0], KeymapProblem::PluginKeyTaken { plugin, by_plugin, .. } if plugin == "b" && by_plugin == "a"));
    }

    #[test]
    fn clear_defaults_drops_plugin_defaults_as_well() {
        let c = cmds(&[("p", "cmd", Some("="), &[LayerId::Entries])]);
        let r = load_keymap_from_str("[normal.entries]\nclear_defaults = true\n", &c);
        assert_eq!(resolve(&r.keymap.entries, &[], parse_key("=").unwrap()), Resolution::Unbound);
    }

    #[test]
    fn the_default_keymap_is_unchanged_when_there_are_no_plugins() {
        let r = load_keymap_from_str("", &crate::plugin::PluginCommands::default());
        assert_eq!(r.keymap.entries, default_keymap().entries);
        assert_eq!(r.keymap.collections, default_keymap().collections);
        assert_eq!(r.keymap.preview, default_keymap().preview);
    }

    #[test]
    fn plugin_actions_have_a_section_and_a_desc() {
        let a = Action::Plugin(crate::plugin::PluginCmdId(0));
        assert_eq!(a.section(), "Plugins");
        assert!(!a.desc().is_empty());
    }
    #[test]
    fn copy_citation_is_a_global_entry_action_on_capital_y() {
        let km = default_keymap();
        for layer in [&km.collections, &km.entries, &km.preview] {
            assert_eq!(resolve(layer, &[], kp("Y")), Resolution::Run(vec![Action::CopyCitation]));
        }
        assert_eq!(Action::CopyCitation.section(), "Entry actions");
    }
}
