use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers, MouseEvent, MouseEventKind, MouseButton},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
    Frame, Terminal,
};
use std::io;

use crate::bibtex::entry_to_filename;
use crate::config::Config;
use crate::keymap::{default_keymap, resolve, Action, Flow, KeyPress, Keymap, LayerId, Resolution};
use crate::models::Entry;
use crate::plugin::fields::{row_cells, Anchor, FieldDecl, FieldStore, Place};
use crate::plugin::host::{HostEvent, Incoming};
use crate::plugin::protocol::{Capabilities, CommandParams, CommandResult, FieldValue, FieldsGetParams, FieldsResult, FieldsSetParams, SelectedParams, StatusSetParams, UiAnswer, UiRequest, ViewParams, ViewResult};
use crate::plugin::rpc::{Id, RpcError};
use crate::plugin::{PluginCmdId, PluginError, PluginHost};
use serde_json::Value;
use std::collections::{BTreeMap, VecDeque};
use crate::theme::theme;
use crate::storage::{find_by_key_mut, load_db, save_db};
use std::sync::mpsc::Receiver;
use std::sync::Arc;

// ── State ────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
enum Panel {
    Collections,
    Entries,
    Preview,
}

/// 탭 렌더 스레드의 답: (요청한 탭 인덱스, 요청 키, 결과).
/// 탭 스레드의 결과. 플러그인 호출·해석·그림 읽기·폭 맞춤까지 끝난 것. 오류에는 플러그인 이름이 붙어 있다.
type TabRender = (usize, crate::preview_tabs::CacheKey, Result<(crate::preview_tabs::Content, u32), String>);

#[derive(Clone, Copy, PartialEq)]
enum PreviewMode {
    Info,
    Note,
    /// `host.tabs()`의 인덱스. 플러그인이 제거되면 `reload_plugins`가 Info로 돌린다.
    Plugin(usize),
}

enum Mode {
    Normal,
    Search,
    Confirm(ConfirmAction),
    Message(String),
    Help,
    SortMenu,
    CollectionPicker,
    TagEditor,
    Loading(String),
    ExportMenu,
    Settings,
    SettingsInput(SettingsInput),
    FilePicker(FilePickerContext),
    FetchPreview,
    SearchResultPicker,
    ContextMenu,
    PluginUi(PluginUiState),
    CitationStyle(usize),
}

struct ContextMenuState {
    x: u16,
    y: u16,
    index: usize,
}

impl ContextMenuState {
    const ITEMS: &'static [(&'static str, Action)] = &[
        ("Open PDF",   Action::OpenPdf),
        ("Web",        Action::OpenWeb),
        ("Copy Key",   Action::CopyCitekey),
        ("Collect",    Action::Collections),
        ("Tag",        Action::Tags),
        ("Note",       Action::EditNote),
        ("Export",     Action::ExportMenu),
        ("Fetch Meta", Action::FetchMetadata),
        ("Sort",       Action::SortMenu),
        ("Delete",     Action::Delete),
    ];
}

struct FieldChange {
    field: String,
    old_val: String,
    new_val: String,
    changed: bool, // old != new
}

struct SearchResultPickerState {
    key: String, // citekey of the entry being updated
    results: Vec<crate::crossref::SearchResult>,
    index: usize,
}

struct FetchPreviewState {
    key: String,
    changes: Vec<FieldChange>,
    selected: Vec<bool>,
    index: usize,
}

#[derive(Clone, PartialEq)]
enum ExportScope {
    Selected,
    /// 컬렉션 이름. 하위 컬렉션(`name/...`)을 포함한다. 컬렉션 패널의 규칙과 같다.
    Collection(String),
    All,
}

/// `name`과 그 하위 컬렉션에 든 항목. 컬렉션 패널이 `gym`을 골랐을 때 보이는 것과 같은 집합.
fn keys_in_collection(entries: &[Entry], name: &str) -> Vec<String> {
    let prefix = format!("{}/", name);
    entries
        .iter()
        .filter(|e| e.collections.iter().any(|c| c == name || c.starts_with(&prefix)))
        .map(|e| e.bibtex_key.clone())
        .collect()
}

/// `gym/method/x` -> [`gym/method/x`, `gym/method`, `gym`]. 현재 컬렉션과 그 상위 전부.
fn collection_and_ancestors(name: &str) -> Vec<String> {
    let mut out = vec![name.to_string()];
    let mut cur = name;
    while let Some(i) = cur.rfind('/') {
        cur = &cur[..i];
        out.push(cur.to_string());
    }
    out
}

#[derive(Clone, Copy, PartialEq)]
enum ExportFormat {
    BibTeX,
    Yaml,
    Ris,
}

impl ExportFormat {
    fn label(&self) -> &'static str {
        match self {
            ExportFormat::BibTeX => "BibTeX (.bib)",
            ExportFormat::Yaml => "YAML (.yaml)",
            ExportFormat::Ris => "RIS (.ris)",
        }
    }
    fn ext(&self) -> &'static str {
        match self {
            ExportFormat::BibTeX => "bibtex",
            ExportFormat::Yaml => "yaml",
            ExportFormat::Ris => "ris",
        }
    }
}

struct ExportState {
    scope_options: Vec<(ExportScope, String)>,
    scope_idx: usize,
    format_idx: usize,
    include_pdf: bool,
    /// 0 = scope section, 1 = format section, 2 = include_pdf toggle
    section: usize,
}

enum ConfirmAction {
    RemovePlugin(String),
    /// 내보낸 파일과 항목 수. y면 파일 관리자에서 보여 준다.
    RevealExport(std::path::PathBuf, usize),
    /// clone은 끝났고 사용자 확인을 기다리는 설치
    InstallStaged(crate::plugin::Staged),
    Delete(String),
    FetchPdf(String),
    OpenBrowser(String, String), // (citekey, url)
    FetchMetaByTitle(String, String), // (citekey, title)
}

enum FilePickerContext {
    AttachPdf(String),  // citekey
    /// `settings::Item.id`. 고르면 Settings로 돌아온다.
    Setting(String),
}

/// Plugins 절의 한 행. 설치된 것은 `list_rows`에서, 설치 안 된 내장은 `BUILTINS`에서.
#[derive(Debug, Clone)]
struct PluginRow {
    name: String,
    version: String,
    /// built-in | local | git | dir | error
    source: String,
    /// error 행은 문제 문구
    description: String,
    installed: bool,
    builtin: bool,
}

fn plugin_rows() -> Vec<PluginRow> {
    let dir = crate::plugin::plugins_dir();
    let mut rows: Vec<PluginRow> = crate::plugin::cli::list_rows(&dir)
        .into_iter()
        .map(|r| PluginRow { name: r.name, version: r.version, source: r.source, description: r.description, installed: true, builtin: r.builtin })
        .collect();
    for b in crate::plugin::builtin::BUILTINS {
        if rows.iter().any(|r| r.name == b.name) {
            continue;
        }
        let description = toml::from_str::<toml::Value>(b.manifest)
            .ok()
            .and_then(|v| v.get("description").and_then(|d| d.as_str()).map(String::from))
            .unwrap_or_default();
        rows.push(PluginRow {
            name: b.name.to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            source: "built-in".to_string(),
            description,
            installed: false,
            builtin: true,
        });
    }
    rows.sort_by(|a, b| b.installed.cmp(&a.installed).then(a.name.cmp(&b.name)));
    rows
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SettingsFocus {
    Sections,
    Rows,
}

/// Settings 팝업의 상태. 선택기나 확인 팝업을 다녀와도 위치가 남도록 `App`에 둔다.
struct SettingsState {
    focus: SettingsFocus,
    /// `settings::Section::ALL`의 인덱스
    section: usize,
    /// 오른쪽 칸 행 인덱스(`settings_pane_rows` 결과 기준)
    row: usize,
    /// 열린 플러그인 페이지. None이면 목록.
    page: Option<String>,
    /// Plugins 절의 행. 열 때와 설치·제거 뒤에 다시 읽는다(디스크를 매 프레임 읽지 않으려고).
    plugins: Vec<PluginRow>,
    /// 검색어. Some이면 오른쪽 칸이 검색 결과다.
    query: Option<String>,
    /// 글자가 검색어로 가는 중(`/` 직후). Enter로 끝내면 j/k h/l이 결과에서 동작한다.
    typing: bool,
    /// 검색 전 위치 (focus, section, row, page). Esc로 되돌린다.
    saved: Option<(SettingsFocus, usize, usize, Option<String>)>,
    /// 아래 줄에 한 번 보이는 알림. (문구, 오류인가). 다음 키에 사라진다.
    notice: Option<(String, bool)>,
}

impl SettingsState {
    fn new() -> Self {
        SettingsState {
            focus: SettingsFocus::Sections,
            section: 0,
            row: 0,
            page: None,
            plugins: Vec::new(),
            query: None,
            typing: false,
            saved: None,
            notice: None,
        }
    }

    fn section(&self) -> crate::settings::Section {
        crate::settings::Section::ALL[self.section]
    }
}

/// 오른쪽 칸의 행. `Item`만 h/l/Enter를 받는다.
#[derive(Debug, Clone, PartialEq)]
enum PaneRow {
    Header(String),
    Text(String),
    Blank,
    /// `App::settings_items()`의 인덱스
    Item(usize),
    /// Plugins 목록의 한 플러그인
    Plugin(String),
    /// 플러그인 페이지의 Installed 토글
    Installed(String),
    InstallFrom,
}

/// 플러그인 페이지 머리 줄. 버전이 없는 플러그인(로컬, 오류)은 그 칸을 비우지 않고 건너뛴다.
fn plugin_page_title(name: &str, version: &str, source: &str) -> String {
    [name, version, source].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join("  ")
}

fn selectable(row: &PaneRow) -> bool {
    matches!(row, PaneRow::Item(_) | PaneRow::Plugin(_) | PaneRow::Installed(_) | PaneRow::InstallFrom)
}

fn first_selectable(rows: &[PaneRow]) -> usize {
    rows.iter().position(selectable).unwrap_or(0)
}

/// 선택 가능한 다음/이전 행. 없으면 제자리.
fn move_cursor(rows: &[PaneRow], from: usize, delta: i32) -> usize {
    let mut i = from as i64;
    loop {
        i += delta as i64;
        if i < 0 || i as usize >= rows.len() {
            return from;
        }
        if selectable(&rows[i as usize]) {
            return i as usize;
        }
    }
}

enum InputTarget {
    /// `settings::Item.id`
    Item(String),
    /// Install from… 의 입력
    InstallSource,
}

/// int/string 항목의 입력 팝업. 플러그인 UI의 prompt와 같은 모양.
struct SettingsInput {
    title: String,
    buf: String,
    target: InputTarget,
}

struct BgTaskResult {
    key: String,
    file_path: String,
    full_path: String,
}

#[derive(Clone, Copy, PartialEq)]
enum SortCriterion {
    Year,
    Author,
    Title,
    Created,
    Updated,
}

impl SortCriterion {
    fn label(&self) -> &'static str {
        match self {
            SortCriterion::Year => "Year",
            SortCriterion::Author => "Author",
            SortCriterion::Title => "Title",
            SortCriterion::Created => "Created",
            SortCriterion::Updated => "Updated",
        }
    }
    fn default_ascending(&self) -> bool {
        match self {
            SortCriterion::Year => false,
            SortCriterion::Author => true,
            SortCriterion::Title => true,
            SortCriterion::Created => false,
            SortCriterion::Updated => false,
        }
    }
    fn all() -> [SortCriterion; 5] {
        [SortCriterion::Year, SortCriterion::Author, SortCriterion::Title, SortCriterion::Created, SortCriterion::Updated]
    }
}

struct ChecklistPicker {
    title: String,
    items: Vec<(String, bool)>,
    index: usize,
    new_item_input: Option<String>,
    new_item_label: String,
}

impl ChecklistPicker {
    fn new(title: String, items: Vec<(String, bool)>, new_item_label: String) -> Self {
        Self { title, items, index: 0, new_item_input: None, new_item_label }
    }
    fn move_up(&mut self) { if self.index > 0 { self.index -= 1; } }
    fn move_down(&mut self) {
        let max = self.items.len();
        if self.index < max { self.index += 1; }
    }
    fn is_on_new_item(&self) -> bool { self.index == self.items.len() }
    fn toggle(&mut self) {
        if self.is_on_new_item() {
            self.new_item_input = Some(String::new());
        } else if let Some(item) = self.items.get_mut(self.index) {
            item.1 = !item.1;
        }
    }
    fn in_input_mode(&self) -> bool { self.new_item_input.is_some() }
    fn apply_char(&mut self, c: char) {
        if let Some(ref mut input) = self.new_item_input { input.push(c); }
    }
    fn backspace(&mut self) {
        if let Some(ref mut input) = self.new_item_input { input.pop(); }
    }
    fn confirm_input(&mut self) {
        if let Some(input) = self.new_item_input.take() {
            let name = input.trim().to_string();
            if !name.is_empty() && !self.items.iter().any(|(n, _)| n == &name) {
                self.items.push((name, true));
            }
        }
    }
    fn cancel_input(&mut self) { self.new_item_input = None; }
    fn checked_names(&self) -> Vec<String> {
        self.items.iter().filter(|(_, c)| *c).map(|(n, _)| n.clone()).collect()
    }
}

pub struct App {
    entries: Vec<Entry>,
    filtered: Vec<usize>,
    list_state: ListState,
    col_list_state: ListState,
    collections: Vec<String>,  // index 0 = "All", rest = collection names
    search_query: String,
    col_search_query: String,
    mode: Mode,
    config: Config,
    // Panels
    focus: Panel,
    preview_mode: PreviewMode,
    preview_scroll: u16,
    preview_max_scroll: u16,
    // Note cache
    note_content: String,
    note_citekey: String,
    // Background tasks
    pending_editor: Option<std::path::PathBuf>,
    bg_result: Option<std::sync::mpsc::Receiver<Result<BgTaskResult>>>,
    bg_fetch_key: Option<String>,
    bg_meta_result: Option<std::sync::mpsc::Receiver<Result<(String, crate::crossref::Metadata)>>>,
    bg_search_result: Option<std::sync::mpsc::Receiver<Result<(String, Vec<crate::crossref::SearchResult>)>>>,
    bg_install: Option<std::sync::mpsc::Receiver<Result<crate::plugin::Staged>>>,
    fetch_preview: Option<FetchPreviewState>,
    search_picker: Option<SearchResultPickerState>,
    file_picker_state: Option<ratatree::FilePickerState>,
    spinner_tick: usize,
    // Help overlay
    help_query: String,
    help_filtering: bool,
    help_scroll: usize,
    // Sort
    sort_by: SortCriterion,
    sort_ascending: bool,
    sort_menu_index: usize,
    prev_sort_by: SortCriterion,
    prev_sort_ascending: bool,
    // Picker
    picker: Option<ChecklistPicker>,
    // 시퀀스 대기 상태(gg 등)와 숫자 접두사 버퍼. 판단은 keymap::resolve가 한다.
    pending: Vec<KeyPress>,
    keymap: Keymap,
    // Undo/Redo stacks (DB snapshots)
    undo_stack: Vec<Vec<Entry>>,
    redo_stack: Vec<Vec<Entry>>,
    // Multi-select
    selected_keys: std::collections::HashSet<String>,
    // Export menu state
    export_state: Option<ExportState>,
    // Settings state
    settings: SettingsState,
    // Panel areas for mouse hit-testing
    panel_areas: [Rect; 3],
    context_menu: ContextMenuState,
    // ── Plugins ──
    host: Arc<PluginHost>,
    plugin_run: Option<PluginRun>,
    // Plugin preview tab: state, per-entry cache and the in-flight render
    tab: crate::preview_tabs::TabState,
    tab_cache: crate::preview_tabs::Cache,
    bg_tab: Option<Receiver<TabRender>>,
    /// 이미지를 그릴 수 있으면 Some. `detect_images`가 시작 때 정한다(설정과 터미널).
    images: Option<ratatui_image::picker::Picker>,
    /// 올려 둔 쪽 (키, pan, 프로토콜). 쪽·배율·pan이 그대로면 다시 만들지 않고 행만 옮겨 그린다.
    tab_sliced: Option<(crate::preview_tabs::CacheKey, u32, ratatui_image::sliced::SlicedProtocol)>,
    /// 지금 만드는 중인 요청을 보낸 시각. 스피너 프레임의 기준.
    tab_pending_since: Option<std::time::Instant>,
    /// 미리보기 항목이 바뀐 시각. `SETTLE_MS`가 지나야 요청한다(목록을 훑는 동안 지나가는 항목은 렌더하지 않음).
    tab_entry_since: std::time::Instant,
    /// 캐시에서 빠진 kitty 그림 id. 다음 프레임 뒤에 터미널에서 지운다.
    kitty_deletes: Vec<u32>,
    events: crate::events::Events,
    /// 팝업이 열려 있는 동안 온 `window/*` 요청. 앞 것이 닫히면 다음 것이 열린다.
    ui_queue: VecDeque<(String, Id, UiRequest)>,
    /// 팝업·로딩 중에 온 메시지. Normal로 돌아오면 하나씩 보인다.
    pending_messages: VecDeque<String>,
    /// `commands/execute`로 요청된 내장 액션. 키 처리 뒤 메인 루프가 실행한다.
    queued_actions: VecDeque<Action>,
    /// 첫 프레임을 그렸는가. 그 뒤 `lifecycle/started`를 보낸다.
    started: bool,
    /// 플러그인 필드 선언(매니페스트 + config 덮어쓰기)과 세션 캐시, 상태 바 조각.
    field_decls: Vec<FieldDecl>,
    fields: FieldStore,
    status_segments: BTreeMap<(String, String), FieldValue>,
    /// 커서가 마지막으로 움직인 시각. `SETTLE_MS` 뒤 `entry/selected`와 그 항목의 fields/get.
    sel_since: Option<std::time::Instant>,
    /// 지난 프레임의 커서 항목. 바뀌면 `sel_since`를 찍는다(키·마우스·필터 어느 길이든).
    last_sel_key: Option<String>,
    fields_rx: Vec<Receiver<(String, crate::plugin::protocol::FieldsResult)>>,
    /// 목록 패널의 안쪽 높이. `visible_keys`가 쓴다. 그릴 때마다 적는다.
    entries_area_height: u16,
}

/// Collect every collection path referenced by `entries`, sorted and deduplicated.
/// The result is the row list for the Collections panel (index 0 = "All" is added by the caller).
fn collection_paths(entries: &[Entry]) -> Vec<String> {
    let mut col_set: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for e in entries {
        for c in &e.collections {
            // Insert every ancestor prefix ("gym" for "gym/seed", "digest" and
            // "digest/2026" for "digest/2026/04"). A parent with no direct entries
            // would otherwise have no row, and its indented children would render
            // under whichever top-level row happens to sort just before them.
            for (i, _) in c.match_indices('/') {
                col_set.insert(c[..i].to_string());
            }
            col_set.insert(c.clone());
        }
    }
    col_set.into_iter().collect()
}

impl App {
    pub fn new(config: Config, host: Arc<PluginHost>) -> Result<Self> {
        let db_path = crate::config::resolve_db_path(&config);
        let db = load_db(&db_path)?;
        let entries = db.entries;
        let events = crate::events::Events { host: Arc::clone(&host) };
        let field_decls = crate::plugin::fields::decls(host.manifests(), &crate::config::plugin_tables(&config));

        let collections = collection_paths(&entries);

        let filtered: Vec<usize> = (0..entries.len()).collect();
        let mut list_state = ListState::default();
        if !filtered.is_empty() { list_state.select(Some(0)); }

        let mut col_list_state = ListState::default();
        col_list_state.select(Some(0)); // "All" selected

        Ok(Self {
            entries,
            filtered,
            list_state,
            col_list_state,
            collections,
            search_query: String::new(),
            col_search_query: String::new(),
            mode: Mode::Normal,
            config,
            focus: Panel::Entries,
            preview_mode: PreviewMode::Info,
            preview_scroll: 0,
            preview_max_scroll: 0,
            note_content: String::new(),
            note_citekey: String::new(),
            pending_editor: None,
            bg_result: None,
            bg_fetch_key: None,
            bg_meta_result: None,
            bg_search_result: None,
            bg_install: None,
            fetch_preview: None,
            search_picker: None,
            file_picker_state: None,
            spinner_tick: 0,
            sort_by: SortCriterion::Created,
            sort_ascending: false,
            sort_menu_index: 0,
            prev_sort_by: SortCriterion::Created,
            prev_sort_ascending: false,
            picker: None,
            pending: Vec::new(),
            keymap: default_keymap(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            selected_keys: std::collections::HashSet::new(),
            export_state: None,
            settings: SettingsState::new(),
            panel_areas: [Rect::default(); 3],
            help_query: String::new(),
            help_filtering: false,
            help_scroll: 0,
            context_menu: ContextMenuState { x: 0, y: 0, index: 0 },
            events,
            host,
            plugin_run: None,
            tab: crate::preview_tabs::TabState::new(),
            tab_cache: Default::default(),
            bg_tab: None,
            images: None,
            tab_sliced: None,
            tab_pending_since: None,
            tab_entry_since: std::time::Instant::now(),
            kitty_deletes: Vec::new(),
            ui_queue: VecDeque::new(),
            pending_messages: VecDeque::new(),
            queued_actions: VecDeque::new(),
            started: false,
            field_decls,
            fields: FieldStore::default(),
            status_segments: BTreeMap::new(),
            sel_since: None,
            last_sel_key: None,
            fields_rx: Vec::new(),
            entries_area_height: 0,
        })
    }

    fn current_collection(&self) -> Option<&str> {
        let idx = self.col_list_state.selected().unwrap_or(0);
        if idx == 0 { None } else { self.collections.get(idx - 1).map(|s| s.as_str()) }
    }

    fn col_count(&self) -> usize { self.collections.len() + 1 }

    fn apply_filters(&mut self) {
        let col = self.current_collection().map(|s| s.to_string());
        let query = self.search_query.to_lowercase();

        self.filtered = self.entries.iter().enumerate()
            .filter(|(_, e)| {
                let col_ok = match &col {
                    None => true,
                    Some(c) => {
                        let prefix = format!("{}/", c);
                        e.collections.iter().any(|ec| ec == c || ec.starts_with(&prefix))
                    }
                };
                if !col_ok { return false; }
                if query.is_empty() { return true; }
                let title = e.title.as_deref().unwrap_or("").to_lowercase();
                let author = e.author.join(" ").to_lowercase();
                let key = e.bibtex_key.to_lowercase();
                let tags = e.tags.join(" ").to_lowercase();
                title.contains(&query) || author.contains(&query)
                    || key.contains(&query) || tags.contains(&query)
            })
            .map(|(i, _)| i)
            .collect();

        if self.filtered.is_empty() {
            self.list_state.select(None);
        } else {
            let cur = self.list_state.selected().unwrap_or(0);
            if cur >= self.filtered.len() {
                self.list_state.select(Some(self.filtered.len() - 1));
            } else {
                self.list_state.select(Some(cur));
            }
        }
        self.apply_sort();
    }

    fn apply_sort(&mut self) {
        let entries = &self.entries;
        let sort_by = self.sort_by;
        let ascending = self.sort_ascending;

        self.filtered.sort_by(|&a, &b| {
            let ea = &entries[a];
            let eb = &entries[b];
            match sort_by {
                SortCriterion::Year => {
                    match (ea.year, eb.year) {
                        (Some(a), Some(b)) => { let c = a.cmp(&b); if ascending { c } else { c.reverse() } }
                        (Some(_), None) => std::cmp::Ordering::Less,
                        (None, Some(_)) => std::cmp::Ordering::Greater,
                        (None, None) => std::cmp::Ordering::Equal,
                    }
                }
                SortCriterion::Author => {
                    let a_n = ea.author.first().and_then(|a| a.split(',').next()).unwrap_or("");
                    let b_n = eb.author.first().and_then(|a| a.split(',').next()).unwrap_or("");
                    match (a_n.is_empty(), b_n.is_empty()) {
                        (true, false) => std::cmp::Ordering::Greater,
                        (false, true) => std::cmp::Ordering::Less,
                        _ => { let c = a_n.to_lowercase().cmp(&b_n.to_lowercase()); if ascending { c } else { c.reverse() } }
                    }
                }
                SortCriterion::Title => {
                    let a_t = ea.title.as_deref().unwrap_or("");
                    let b_t = eb.title.as_deref().unwrap_or("");
                    match (a_t.is_empty(), b_t.is_empty()) {
                        (true, false) => std::cmp::Ordering::Greater,
                        (false, true) => std::cmp::Ordering::Less,
                        _ => { let c = a_t.to_lowercase().cmp(&b_t.to_lowercase()); if ascending { c } else { c.reverse() } }
                    }
                }
                SortCriterion::Created => {
                    let c = ea.created_at.cmp(&eb.created_at);
                    if ascending { c } else { c.reverse() }
                }
                SortCriterion::Updated => {
                    // Fall back to created_at when updated_at is None
                    let a_ts = ea.updated_at.as_deref().unwrap_or(&ea.created_at);
                    let b_ts = eb.updated_at.as_deref().unwrap_or(&eb.created_at);
                    let c = a_ts.cmp(b_ts);
                    if ascending { c } else { c.reverse() }
                }
            }
        });

        if !self.filtered.is_empty() {
            let cur = self.list_state.selected().unwrap_or(0);
            if cur >= self.filtered.len() {
                self.list_state.select(Some(0));
            }
        }
    }

    fn selected_entry(&self) -> Option<&Entry> {
        let sel = self.list_state.selected()?;
        let idx = self.filtered.get(sel)?;
        self.entries.get(*idx)
    }

    fn selected_entry_idx(&self) -> Option<usize> {
        let sel = self.list_state.selected()?;
        self.filtered.get(sel).copied()
    }

    fn move_entry_down(&mut self) {
        if self.filtered.is_empty() { return; }
        let next = match self.list_state.selected() {
            Some(i) => (i + 1).min(self.filtered.len() - 1),
            None => 0,
        };
        self.list_state.select(Some(next));
        self.update_preview();
    }

    fn move_entry_up(&mut self) {
        if self.filtered.is_empty() { return; }
        let prev = match self.list_state.selected() {
            Some(i) if i > 0 => i - 1,
            _ => 0,
        };
        self.list_state.select(Some(prev));
        self.update_preview();
    }

    fn move_col_down(&mut self) {
        let n = self.col_count();
        if n == 0 { return; }
        let next = match self.col_list_state.selected() {
            Some(i) => (i + 1).min(n - 1),
            None => 0,
        };
        self.col_list_state.select(Some(next));
        self.list_state.select(Some(0));
        self.apply_filters();
    }

    fn move_col_up(&mut self) {
        let prev = match self.col_list_state.selected() {
            Some(i) if i > 0 => i - 1,
            _ => 0,
        };
        self.col_list_state.select(Some(prev));
        self.list_state.select(Some(0));
        self.apply_filters();
    }

    fn update_preview(&mut self) {
        self.preview_scroll = 0;
        if self.preview_mode == PreviewMode::Note {
            self.load_note_for_preview();
        }
    }

    fn load_note_for_preview(&mut self) {
        let citekey = match self.selected_entry() {
            Some(entry) => entry.bibtex_key.clone(),
            None => return,
        };
        if citekey == self.note_citekey { return; }
        let note_path = self.config.notes_dir.join(format!("{}.md", citekey));
        self.note_content = if note_path.exists() {
            std::fs::read_to_string(&note_path).unwrap_or_else(|_| "Error reading note.".into())
        } else {
            "No note yet. Press N to create one.".into()
        };
        self.note_citekey = citekey;
    }

    fn open_pdf(&self, entry: &Entry) {
        if let Some(fp) = &entry.file_path {
            let full_path = self.config.bibox_dir.join(fp);
            if !full_path.exists() { return; }
            let path_str = full_path.to_string_lossy().to_string();
            if let Some(viewer) = &self.config.pdf_viewer {
                let _ = std::process::Command::new(viewer).arg(&path_str).spawn();
            } else {
                #[cfg(target_os = "macos")]
                let _ = std::process::Command::new("open").arg(&path_str).spawn();
                #[cfg(not(target_os = "macos"))]
                let _ = std::process::Command::new("xdg-open").arg(&path_str).spawn();
            }
        }
    }


    fn delete_selected(&mut self) -> Result<()> {
        self.push_undo();
        if let Some(idx) = self.selected_entry_idx() {
            let entry = &self.entries[idx];
            if let Some(fp) = &entry.file_path {
                let path = self.config.bibox_dir.join(fp);
                if path.exists() { let _ = std::fs::remove_file(&path); }
            }
            let key = entry.bibtex_key.clone();
            let removed = entry.clone();
            self.entries.retain(|e| e.bibtex_key != key);
            self.persist(crate::events::WriteReason::Delete, vec![removed], true)?;
            self.rebuild_collections();
            self.apply_filters();
        }
        Ok(())
    }

    fn rebuild_collections(&mut self) {
        self.collections = collection_paths(&self.entries);
        let sel = self.col_list_state.selected().unwrap_or(0);
        if sel >= self.col_count() {
            self.col_list_state.select(Some(0));
        }
    }

    const MAX_UNDO: usize = 50;

    /// Push current entries to undo stack before making a change.
    fn push_undo(&mut self) {
        if self.undo_stack.len() >= Self::MAX_UNDO {
            self.undo_stack.remove(0);
        }
        self.undo_stack.push(self.entries.clone());
        self.redo_stack.clear();
    }

    fn undo(&mut self) -> Result<()> {
        if let Some(snapshot) = self.undo_stack.pop() {
            self.redo_stack.push(self.entries.clone());
            self.entries = snapshot;
            self.persist(crate::events::WriteReason::Undo, vec![], true)?;
            self.rebuild_collections();
            self.apply_filters();
            self.mode = Mode::Message(format!("Undo ({})", self.undo_stack.len()));
        } else {
            self.mode = Mode::Message("Nothing to undo.".into());
        }
        Ok(())
    }

    fn redo(&mut self) -> Result<()> {
        if let Some(snapshot) = self.redo_stack.pop() {
            self.undo_stack.push(self.entries.clone());
            self.entries = snapshot;
            self.persist(crate::events::WriteReason::Redo, vec![], true)?;
            self.rebuild_collections();
            self.apply_filters();
            self.mode = Mode::Message(format!("Redo ({})", self.redo_stack.len()));
        } else {
            self.mode = Mode::Message("Nothing to redo.".into());
        }
        Ok(())
    }

    fn open_collection_picker(&mut self) {
        if let Some(entry) = self.selected_entry() {
            let entry_cols: std::collections::HashSet<&String> = entry.collections.iter().collect();
            let all_cols: std::collections::BTreeSet<String> = self.entries.iter()
                .flat_map(|e| e.collections.iter().cloned()).collect();
            let items: Vec<(String, bool)> = all_cols.into_iter()
                .map(|c| { let checked = entry_cols.contains(&c); (c, checked) }).collect();
            let key = entry.bibtex_key.clone();
            self.picker = Some(ChecklistPicker::new(
                format!("Collections for [{}]:", key), items, "+ New collection...".into(),
            ));
            self.mode = Mode::CollectionPicker;
        }
    }

    fn open_collection_picker_multi(&mut self) {
        // For multi-select: show all collections, check ones that ALL selected entries share
        let selected_entries: Vec<&Entry> = self.entries.iter()
            .filter(|e| self.selected_keys.contains(&e.bibtex_key))
            .collect();
        if selected_entries.is_empty() { return; }

        let all_cols: std::collections::BTreeSet<String> = self.entries.iter()
            .flat_map(|e| e.collections.iter().cloned()).collect();

        // Intersection: checked if ALL selected entries have this collection
        let items: Vec<(String, bool)> = all_cols.into_iter().map(|c| {
            let all_have = selected_entries.iter().all(|e| e.collections.contains(&c));
            (c, all_have)
        }).collect();

        let count = selected_entries.len();
        self.picker = Some(ChecklistPicker::new(
            format!("Collections for {} entries:", count), items, "+ New collection...".into(),
        ));
        self.mode = Mode::CollectionPicker;
    }

    fn apply_picker_collections_multi(&mut self) -> Result<()> {
        self.push_undo();
        let new_cols = match &self.picker {
            Some(picker) => picker.checked_names(),
            None => { return Ok(()); }
        };
        self.picker = None;

        // Apply to all selected entries
        for e in self.entries.iter_mut() {
            if self.selected_keys.contains(&e.bibtex_key) {
                e.collections = new_cols.clone();
                e.updated_at = Some(chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string());
            }
        }
        let affected: Vec<Entry> = self.entries.iter().filter(|e| self.selected_keys.contains(&e.bibtex_key)).cloned().collect();
        self.persist(crate::events::WriteReason::Edit, affected, true)?;

        let count = self.selected_keys.len();
        self.selected_keys.clear();
        self.rebuild_collections();
        self.apply_filters();
        self.mode = Mode::Message(format!("Updated collections for {} entries.", count));
        Ok(())
    }

    fn open_tag_editor(&mut self) {
        if let Some(entry) = self.selected_entry() {
            let entry_tags: std::collections::HashSet<&String> = entry.tags.iter().collect();
            let all_tags: std::collections::BTreeSet<String> = self.entries.iter()
                .flat_map(|e| e.tags.iter().cloned()).collect();
            let items: Vec<(String, bool)> = all_tags.into_iter()
                .map(|t| { let checked = entry_tags.contains(&t); (t, checked) }).collect();
            let key = entry.bibtex_key.clone();
            self.picker = Some(ChecklistPicker::new(
                format!("Tags for [{}]:", key), items, "+ New tag...".into(),
            ));
            self.mode = Mode::TagEditor;
        }
    }

    fn apply_picker_collections(&mut self) -> Result<()> {
        self.push_undo();
        let (new_cols, idx) = match (&self.picker, self.selected_entry_idx()) {
            (Some(picker), Some(idx)) => (picker.checked_names(), idx),
            _ => { self.picker = None; return Ok(()); }
        };
        self.picker = None;
        self.entries[idx].collections = new_cols;
        let affected = vec![self.entries[idx].clone()];
        self.persist(crate::events::WriteReason::Edit, affected, true)?;
        self.rebuild_collections();
        self.apply_filters();
        Ok(())
    }

    fn apply_picker_tags(&mut self) -> Result<()> {
        self.push_undo();
        let (new_tags, idx) = match (&self.picker, self.selected_entry_idx()) {
            (Some(picker), Some(idx)) => (picker.checked_names(), idx),
            _ => { self.picker = None; return Ok(()); }
        };
        self.picker = None;
        self.entries[idx].tags = new_tags;
        let affected = vec![self.entries[idx].clone()];
        self.persist(crate::events::WriteReason::Edit, affected, true)?;
        self.apply_filters();
        Ok(())
    }
}

// ── Citation ────────────────────────────────────────────────────────────────

fn handle_citation_style(app: &mut App, key: crossterm::event::KeyEvent) -> Result<bool> {
    let idx = match &app.mode {
        Mode::CitationStyle(i) => *i,
        _ => return Ok(false),
    };
    let styles = crate::citation::Style::all();
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => app.mode = Mode::CitationStyle(idx.saturating_sub(1)),
        KeyCode::Down | KeyCode::Char('j') => app.mode = Mode::CitationStyle((idx + 1).min(styles.len() - 1)),
        KeyCode::Enter => {
            let style = styles[idx];
            let (text, n) = {
                let entries: Vec<&Entry> = if app.selected_keys.is_empty() {
                    app.selected_entry().into_iter().collect()
                } else {
                    app.entries.iter().filter(|e| app.selected_keys.contains(&e.bibtex_key)).collect()
                };
                (crate::citation::format_many(&entries, style), entries.len())
            };
            if let Ok(mut ctx) = arboard::Clipboard::new() {
                let _ = ctx.set_text(&text);
            }
            app.mode = Mode::Message(app.config.msgs.citation_copied(style.label(), n));
        }
        KeyCode::Esc | KeyCode::Char('q') => app.mode = Mode::Normal,
        _ => {}
    }
    Ok(false)
}

fn draw_citation_popup(f: &mut Frame, idx: usize, area: Rect) {
    let popup_area = centered_rect(40, 9, area);
    clear_area(f, popup_area);
    let mut lines = vec![
        Line::from(Span::styled("Citation style", Style::default().fg(theme().heading))),
        Line::from(""),
    ];
    for (i, s) in crate::citation::Style::all().iter().enumerate() {
        let arrow = if i == idx { "▶ " } else { "  " };
        let style = if i == idx { Style::default().fg(theme().accent) } else { Style::default() };
        lines.push(Line::from(vec![
            Span::styled(arrow, Style::default().fg(theme().heading)),
            Span::styled(s.label(), style),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("↑↓ select  Enter copy  Esc cancel", Style::default().fg(theme().muted))));
    let popup = Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(" Copy citation "));
    f.render_widget(popup, popup_area);
}

// ── Plugins ─────────────────────────────────────────────────────────────────

/// 돌고 있는 플러그인 명령. 답은 워커 스레드가 채널로 보낸다. 팝업·진행은 호스트 이벤트로 온다.
struct PluginRun {
    plugin: String,
    rx: Receiver<Result<Value, PluginError>>,
}

/// 팝업이 열려 있으면 뒤에 온 요청은 줄을 선다. 즉시 답할 수 있는 요청(빈 pick)은 호출자가 먼저 걸러 둔다.
fn open_or_queue(open: &mut Option<PluginUiState>, q: &mut VecDeque<(String, Id, UiRequest)>, plugin: &str, id: Id, req: UiRequest) {
    if open.is_some() {
        q.push_back((plugin.to_string(), id, req));
    } else if let Ok(kind) = PluginUiKind::from_request(plugin, req.clone()) {
        *open = Some(PluginUiState { plugin: plugin.to_string(), id, kind });
    } else {
        q.push_front((plugin.to_string(), id, req));
    }
}

/// 왼쪽 내용 뒤 남는 폭에 오른쪽 정렬 조각. 안 들어가면 빈 문자열.
fn row_suffix(width: u16, left: &str, right: &[(String, u16)]) -> String {
    let used = left.chars().count() as u16;
    let avail = width.saturating_sub(used);
    if avail < 3 || right.is_empty() {
        return String::new();
    }
    crate::plugin::fields::fit_right(avail, right)
}

/// `status/set` 하나. 빈 text는 조각을 지운다.
fn apply_status_set(segs: &mut BTreeMap<(String, String), FieldValue>, plugin: &str, params: Value) {
    let Ok(p) = serde_json::from_value::<StatusSetParams>(params) else { return };
    let k = (plugin.to_string(), p.field);
    if p.text.trim().is_empty() {
        segs.remove(&k);
    } else {
        segs.insert(k, FieldValue { text: p.text, color: p.color });
    }
}

/// 플러그인 필드 색. 테마 이름(accent, success, warning, error, muted, heading)이나 `#rrggbb`. 없거나 모르면 muted.
fn field_style(color: &Option<String>) -> Style {
    let t = theme();
    let c = match color.as_deref() {
        Some("accent") => t.accent,
        Some("success") => t.success,
        Some("warning") => t.warning,
        Some("error") => t.error,
        Some("heading") => t.heading,
        Some("muted") | None => t.muted,
        Some(hex) => crate::theme::parse_hex(hex).unwrap_or(t.muted),
    };
    Style::default().fg(c)
}

/// `commands/execute`의 이름은 keymap.toml의 액션 이름과 같다(snake_case). 플러그인 명령은 안 된다.
fn action_by_name(name: &str) -> Option<Action> {
    serde_json::from_value(Value::String(name.to_string())).ok()
}

#[derive(Debug)]
enum PluginUiKind {
    Pick { title: String, items: Vec<String>, index: usize },
    Prompt { title: String, buf: String },
    Confirm { title: String },
}

impl PluginUiKind {
    /// 팝업이 필요 없는 요청은 `Err(즉시 답)`. 빈 pick은 null, progress는 Ack.
    fn from_request(plugin: &str, req: UiRequest) -> Result<PluginUiKind, UiAnswer> {
        match req {
            UiRequest::Pick { items, .. } if items.is_empty() => Err(UiAnswer::Index { index: None }),
            UiRequest::Pick { title, items } => Ok(PluginUiKind::Pick {
                title: title.unwrap_or_else(|| plugin.to_string()),
                items,
                index: 0,
            }),
            UiRequest::Prompt { title, default } => Ok(PluginUiKind::Prompt {
                title: title.unwrap_or_else(|| plugin.to_string()),
                buf: default.unwrap_or_default(),
            }),
            UiRequest::Confirm { title } => Ok(PluginUiKind::Confirm { title: title.unwrap_or_else(|| plugin.to_string()) }),
            UiRequest::Progress { .. } => Err(UiAnswer::Ack {}),
        }
    }
}

struct PluginUiState {
    plugin: String,
    /// 답할 요청의 id
    id: Id,
    kind: PluginUiKind,
}

/// 팝업 키 하나. 답이 정해지면 `Some`. 키는 기존 팝업(정렬 메뉴, 검색창, 확인)과 같다.
fn plugin_ui_step(kind: &mut PluginUiKind, code: KeyCode) -> Option<UiAnswer> {
    match kind {
        PluginUiKind::Pick { items, index, .. } => match code {
            KeyCode::Up | KeyCode::Char('k') => {
                *index = index.saturating_sub(1);
                None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if *index + 1 < items.len() {
                    *index += 1;
                }
                None
            }
            KeyCode::Enter => Some(UiAnswer::Index { index: Some(*index) }),
            KeyCode::Esc | KeyCode::Char('q') => Some(UiAnswer::Index { index: None }),
            _ => None,
        },
        PluginUiKind::Prompt { buf, .. } => match code {
            KeyCode::Char(c) => {
                buf.push(c);
                None
            }
            KeyCode::Backspace => {
                buf.pop();
                None
            }
            KeyCode::Enter => Some(UiAnswer::Text { text: Some(buf.clone()) }),
            KeyCode::Esc => Some(UiAnswer::Text { text: None }),
            _ => None,
        },
        PluginUiKind::Confirm { .. } => match code {
            KeyCode::Char('y') | KeyCode::Char('Y') => Some(UiAnswer::Yes { yes: true }),
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc | KeyCode::Char('q') => Some(UiAnswer::Yes { yes: false }),
            _ => None,
        },
    }
}

impl App {
    fn open_settings(&mut self, section: crate::settings::Section) {
        self.settings = SettingsState::new();
        self.settings.plugins = plugin_rows();
        self.settings.section = crate::settings::Section::ALL.iter().position(|s| *s == section).unwrap_or(0);
        let items = self.settings_items();
        let rows = settings_pane_rows(self, &items);
        self.settings.row = first_selectable(&rows);
        self.mode = Mode::Settings;
    }

    fn settings_items(&self) -> Vec<crate::settings::Item> {
        crate::settings::items(self.host.manifests())
    }

    /// 값을 바꾼 직후마다 부른다. 실패는 알림 줄에. 플러그인은 다음 요청부터 새 값을 본다.
    fn save_settings(&mut self) {
        if let Err(e) = crate::config::save_config(&self.config) {
            self.settings.notice = Some((format!("Could not save config.toml: {}", e), true));
        }
        let tables = crate::config::plugin_tables(&self.config);
        self.host.update_config_tables(tables.clone());
        self.field_decls = crate::plugin::fields::decls(self.host.manifests(), &tables);
        // 떠 있는 플러그인에게만. 안 뜬 것은 다음 initialize가 새 값을 싣는다
        for m in self.host.manifests() {
            if self.host.is_running(&m.name) {
                let config = tables.get(&m.name).cloned().unwrap_or_else(|| Value::Object(Default::default()));
                let _ = self.host.notify(&m.name, "config/changed", serde_json::json!({"config": config}));
            }
        }
    }

    /// Install from… 의 입력. 내장과 로컬은 바로, 저장소는 clone 스레드 뒤 확인 팝업.
    fn start_install(&mut self, source: String) {
        use crate::plugin::cli::{parse_source, Source};
        let dir = crate::plugin::plugins_dir();
        match parse_source(&source) {
            Source::Builtin(name) => match (|| -> Result<std::path::PathBuf> {
                std::fs::create_dir_all(&dir)?;
                if dir.join(&name).exists() {
                    anyhow::bail!("{} already exists", dir.join(&name).display());
                }
                Ok(crate::plugin::write_stub(&dir, &name)?)
            })() {
                Ok(dest) => {
                    self.reload_plugins();
                    self.settings.notice = Some((self.config.msgs.plugin_installed(&name, &dest.display().to_string()), false));
                }
                Err(e) => self.settings.notice = Some((self.config.msgs.plugin_install_failed(&e.to_string()), true)),
            },
            Source::Local(path) => match crate::plugin::install_local(&dir, &path, &self.config.msgs) {
                Ok((name, dest)) => {
                    self.reload_plugins();
                    self.settings.notice = Some((self.config.msgs.plugin_installed(&name, &dest.display().to_string()), false));
                }
                Err(e) => self.settings.notice = Some((self.config.msgs.plugin_install_failed(&e.to_string()), true)),
            },
            Source::GitHub { owner, repo, subdir } => {
                self.spawn_clone(dir, format!("https://github.com/{}/{}.git", owner, repo), subdir, source);
            }
            Source::Url(url) => self.spawn_clone(dir, url, None, source),
        }
    }

    fn spawn_clone(&mut self, dir: std::path::PathBuf, url: String, subdir: Option<std::path::PathBuf>, shown: String) {
        let (tx, rx) = std::sync::mpsc::channel();
        let label = shown.clone();
        std::thread::spawn(move || {
            let r = crate::plugin::stage_from_git(&dir, &url, subdir.as_deref(), &shown);
            // 받는 쪽이 사라졌으면(Esc) clone을 치운다
            if let Err(unsent) = tx.send(r) {
                if let Ok(staged) = unsent.0 {
                    crate::plugin::discard_staged(staged);
                }
            }
        });
        self.bg_install = Some(rx);
        self.spinner_tick = 0;
        self.mode = Mode::Loading(self.config.msgs.plugin_cloning(&label));
    }

    /// 설치·제거 뒤. 호스트를 다시 만들고 키맵·도움말이 새 명령 표를 보게 한다.
    /// 옛 호스트는 마지막 Arc가 떨어질 때 Drop이 프로세스를 정리한다.
    fn reload_plugins(&mut self) {
        let (host, _problems) = PluginHost::discover(&self.config);
        let host = Arc::new(host);
        self.keymap = crate::keymap::load_keymap(host.commands()).keymap;
        self.events.host = Arc::clone(&host);
        self.field_decls = crate::plugin::fields::decls(host.manifests(), &crate::config::plugin_tables(&self.config));
        self.host = host;
        self.settings.plugins = plugin_rows();
        self.settings_fix_cursor();
        // 탭 목록이 바뀌었다. 인덱스가 밀렸을 수 있으니 캐시를 버리고 없어진 탭은 Info로
        let gone = self.tab_cache.clear();
        self.kitty_deletes.extend(gone);
        self.tab.pending = None;
        self.tab_sliced = None;
        if let PreviewMode::Plugin(i) = self.preview_mode {
            if i >= self.host.views().len() {
                self.preview_mode = PreviewMode::Info;
            }
        }
    }

    /// 행 목록이 바뀐 뒤(설치·제거, 페이지 닫기) 커서를 선택 가능한 행에 둔다.
    /// 범위 밖이면 가장 가까운 위쪽 행, 그것도 없으면 첫 행.
    fn settings_fix_cursor(&mut self) {
        let items = self.settings_items();
        let rows = settings_pane_rows(self, &items);
        let r = self.settings.row.min(rows.len().saturating_sub(1));
        self.settings.row = if rows.get(r).is_some_and(selectable) {
            r
        } else {
            (0..=r).rev().find(|i| rows.get(*i).is_some_and(selectable)).unwrap_or_else(|| first_selectable(&rows))
        };
    }

    /// 메모리의 항목을 디스크에 쓰고 `after_write`를 백그라운드로 발화한다.
    /// `fire_hooks = false`는 훅 안에서 생긴 쓰기(재발화 방지)에만 쓴다.
    fn persist(&mut self, reason: crate::events::WriteReason, affected: Vec<Entry>, fire_hooks: bool) -> Result<()> {
        let db_path = crate::config::resolve_db_path(&self.config);
        let mut db = load_db(&db_path)?;
        db.entries = self.entries.clone();
        save_db(&db, &db_path)?;
        if fire_hooks {
            self.fire_after_write(reason, affected);
        }
        Ok(())
    }

    /// `library/written`. 알림이라 바로 돌아온다(안 뜬 lazy 플러그인은 여기서 뜬다). 바뀐 키는 다시 묻는다.
    fn fire_after_write(&mut self, reason: crate::events::WriteReason, affected: Vec<Entry>) {
        let keys: Vec<String> = affected.iter().map(|e| e.bibtex_key.clone()).collect();
        self.fields.forget(&keys);
        for o in self.events.written(reason, affected) {
            if let Err(e) = o.result {
                self.show_message(format!("{}: {}", o.plugin, e));
            }
        }
    }

    fn fire_after_note_save(&mut self, entry: Entry, note_path: std::path::PathBuf) {
        for o in self.events.note_saved(entry, note_path) {
            if let Err(e) = o.result {
                self.show_message(format!("{}: {}", o.plugin, e));
            }
        }
    }

    /// 상태 줄 메시지. 팝업이나 로딩 중이면 미뤄 두었다가 Normal로 돌아올 때 보인다.
    fn show_message(&mut self, text: String) {
        match self.mode {
            Mode::Normal | Mode::Message(_) => self.mode = Mode::Message(text),
            _ => self.pending_messages.push_back(text),
        }
    }

    // ── 플러그인 이벤트 ──────────────────────────────────────────────────────

    /// 팝업 요청 하나. 열려 있으면 줄 세우고, 즉시 답할 수 있으면(빈 pick) 바로 답한다.
    fn offer_window_request(&mut self, plugin: String, id: Id, req: UiRequest) {
        if let Err(answer) = PluginUiKind::from_request(&plugin, req.clone()) {
            self.host.respond(&plugin, id, Ok(answer.to_result()));
            return;
        }
        let mut open = match std::mem::replace(&mut self.mode, Mode::Normal) {
            Mode::PluginUi(s) => Some(s),
            other => {
                self.mode = other;
                None
            }
        };
        open_or_queue(&mut open, &mut self.ui_queue, &plugin, id, req);
        if let Some(s) = open {
            self.mode = Mode::PluginUi(s);
        }
    }

    /// 팝업 답. host에 보내고 줄 선 다음 요청을 연다.
    fn reply_plugin_ui(&mut self, answer: UiAnswer) {
        let Mode::PluginUi(state) = std::mem::replace(&mut self.mode, Mode::Normal) else { return };
        self.host.respond(&state.plugin, state.id, Ok(answer.to_result()));
        self.mode = if self.plugin_run.is_some() { Mode::Loading(format!("{}: running", state.plugin)) } else { Mode::Normal };
        if let Some((plugin, id, req)) = self.ui_queue.pop_front() {
            self.offer_window_request(plugin, id, req);
        }
    }

    /// 플러그인이 보낸 것 하나. 요청은 답하고, 알림은 반영하고, 종료는 알린다.
    fn handle_host_event(&mut self, ev: HostEvent) {
        match ev {
            HostEvent::Incoming { plugin, msg: Incoming::Request { id, method, params } } => {
                if let Some(req) = UiRequest::from_method(&method, &params) {
                    self.offer_window_request(plugin, id, req);
                    return;
                }
                let result = match method.as_str() {
                    "library/apply" => {
                        let entries = params.get("entries").and_then(Value::as_array).cloned().unwrap_or_default();
                        let n = entries.len();
                        self.apply_plugin_entries(entries, true).map(|_| serde_json::json!({"applied": n})).map_err(RpcError::invalid_params)
                    }
                    "commands/execute" => {
                        let name = params.get("command").and_then(Value::as_str).unwrap_or("");
                        match action_by_name(name) {
                            Some(action) => {
                                self.queued_actions.push_back(action);
                                Ok(serde_json::json!({}))
                            }
                            None => Err(RpcError::invalid_params(format!("unknown command {}", name))),
                        }
                    }
                    other => Err(RpcError::method_not_found(other)),
                };
                self.host.respond(&plugin, id, result);
            }
            HostEvent::Incoming { plugin, msg: Incoming::Notification { method, params } } => match method.as_str() {
                "window/progress" => {
                    if self.plugin_run.as_ref().map(|r| r.plugin == plugin).unwrap_or(false) && matches!(self.mode, Mode::Loading(_)) {
                        self.mode = Mode::Loading(format!("{}: {}", plugin, params.get("text").and_then(Value::as_str).unwrap_or("")));
                    }
                }
                "window/message" => self.show_message(format!("{}: {}", plugin, params.get("text").and_then(Value::as_str).unwrap_or(""))),
                "library/refresh" => {
                    if let Err(e) = self.refresh_from_disk() {
                        self.show_message(format!("{}: refresh failed: {}", plugin, e));
                    }
                }
                "status/set" | "fields/set" => self.handle_field_notification(&plugin, &method, params),
                "bibox/bad-line" => self.show_message(format!("{}: {}", plugin, params.get("error").and_then(Value::as_str).unwrap_or("bad line"))),
                _ => {}
            },
            HostEvent::Exited { plugin, status } => {
                self.on_plugin_exited(&plugin);
                self.show_message(match status {
                    Some(c) => format!("{}: exited with code {}", plugin, c),
                    None => format!("{}: exited", plugin),
                });
            }
        }
    }

    /// `status/set`은 선언된 상태 조각만 받고, `fields/set`은 캐시에 덮어쓴다.
    fn handle_field_notification(&mut self, plugin: &str, method: &str, params: Value) {
        match method {
            "status/set" => {
                let field = params.get("field").and_then(Value::as_str).unwrap_or("").to_string();
                let declared = self.field_decls.iter().any(|d| d.plugin == plugin && d.id == field && d.place == Place::Status && d.enabled);
                if declared {
                    apply_status_set(&mut self.status_segments, plugin, params);
                }
            }
            "fields/set" => {
                if let Ok(p) = serde_json::from_value::<FieldsSetParams>(params) {
                    self.fields.set_many(plugin, &p.fields);
                }
            }
            _ => {}
        }
    }

    /// 죽은 플러그인의 값은 전부 빈칸. 목록은 그대로 그려진다.
    fn on_plugin_exited(&mut self, plugin: &str) {
        self.fields.clear_plugin(plugin);
        self.status_segments.retain(|(p, _), _| p != plugin);
    }

    /// 지금 화면에 보이는 항목의 키. 목록 오프셋부터 (높이/3)개.
    fn visible_keys(&self) -> Vec<String> {
        let rows = (self.entries_area_height as usize / 3).max(1);
        let start = self.list_state.offset();
        self.filtered.iter().skip(start).take(rows).map(|&i| self.entries[i].bibtex_key.clone()).collect()
    }

    /// 아직 안 물은 키를 필드가 있는 플러그인마다 fields/get. 답은 fields_rx로 온다.
    fn pull_fields(&mut self, keys: Vec<String>) {
        let mut plugins: Vec<String> = self.field_decls.iter().filter(|d| d.enabled).map(|d| d.plugin.clone()).collect();
        plugins.sort();
        plugins.dedup();
        if plugins.is_empty() {
            return;
        }
        let wanted = self.fields.wanted(&keys, 50);
        if wanted.is_empty() {
            return;
        }
        let entries: Vec<Entry> = self.entries.iter().filter(|e| wanted.contains(&e.bibtex_key)).cloned().collect();
        for plugin in plugins {
            let params = serde_json::to_value(FieldsGetParams { keys: wanted.clone(), entries: entries.clone() }).unwrap_or(Value::Null);
            let host = Arc::clone(&self.host);
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                if let Ok(v) = host.call(&plugin, "fields/get", params, Some(std::time::Duration::from_secs(30))) {
                    if let Ok(r) = serde_json::from_value::<FieldsResult>(v) {
                        let _ = tx.send((plugin, r));
                    }
                }
            });
            self.fields_rx.push(rx);
        }
    }

    /// 커서가 멈춘 뒤: 그 항목의 필드를 묻고 구독자에게 `entry/selected`.
    fn on_cursor_settled(&mut self) {
        let Some(e) = self.selected_entry().cloned() else { return };
        self.pull_fields(vec![e.bibtex_key.clone()]);
        if !self.host.subscribers("entry/selected").is_empty() {
            let host = Arc::clone(&self.host);
            let params = serde_json::to_value(SelectedParams { entry: e }).unwrap_or(Value::Null);
            std::thread::spawn(move || {
                host.emit("entry/selected", params);
            });
        }
    }

    // ── 미리보기 탭 ──────────────────────────────────────────────────────────

    fn preview_modes(&self) -> Vec<PreviewMode> {
        let mut v = vec![PreviewMode::Info, PreviewMode::Note];
        v.extend((0..self.host.views().len()).map(PreviewMode::Plugin));
        v
    }

    fn preview_label(&self, m: PreviewMode) -> String {
        match m {
            PreviewMode::Info => "Info".to_string(),
            PreviewMode::Note => "Note".to_string(),
            PreviewMode::Plugin(i) => self.host.views().get(i).map(|t| t.title.clone()).unwrap_or_default(),
        }
    }

    fn preview_next(&self, wrap: bool) -> Option<PreviewMode> {
        let modes = self.preview_modes();
        let i = modes.iter().position(|m| *m == self.preview_mode).unwrap_or(0);
        if i + 1 < modes.len() { Some(modes[i + 1]) } else if wrap { Some(modes[0]) } else { None }
    }

    fn preview_prev(&self) -> Option<PreviewMode> {
        let modes = self.preview_modes();
        let i = modes.iter().position(|m| *m == self.preview_mode).unwrap_or(0);
        if i > 0 { Some(modes[i - 1]) } else { None }
    }

    /// 탭을 바꿀 때 한 자리에서. Note는 노트를 읽고, 다른 플러그인 탭으로 옮기면 캐시를 비운다
    /// (캐시 키에 플러그인이 없다. 탭 하나·항목 하나만 캐시한다).
    fn set_preview_mode(&mut self, m: PreviewMode) {
        if matches!(m, PreviewMode::Plugin(_)) && m != self.preview_mode {
            let gone = self.tab_cache.clear();
            self.kitty_deletes.extend(gone);
            self.tab.pending = None;
        }
        self.preview_mode = m;
        self.preview_scroll = 0;
        if m == PreviewMode::Note {
            self.load_note_for_preview();
        }
    }

    /// (항목, 쪽, 폭)을 플러그인에 묻는다. 훅처럼 스레드에서, 팝업은 열 수 없다.
    fn request_tab(&mut self, tab_index: usize, key: crate::preview_tabs::CacheKey, images: bool) {
        let Some(t) = self.host.views().get(tab_index).cloned() else { return };
        let params = serde_json::to_value(ViewParams { view: t.run.clone(), entry: self.selected_entry().cloned(), page: key.page, width_px: key.width_px, images }).unwrap_or(Value::Null);
        let host = Arc::clone(&self.host);
        let kitty = self.images.as_ref().map(|p| p.protocol_type() == ratatui_image::picker::ProtocolType::Kitty).unwrap_or(false);
        let (tx, rx) = std::sync::mpsc::channel();
        let k = key.clone();
        crate::trace::log(|| format!("tab.req {} p{} w{}", k.entry_key, k.page, k.width_px));
        std::thread::spawn(move || {
            let t0 = std::time::Instant::now();
            let result = host.call(&t.plugin, "views/render", params, Some(std::time::Duration::from_secs(60)));
            crate::trace::log(|| format!("tab.plugin {:.0}ms p{} ok={}", t0.elapsed().as_secs_f64() * 1000.0, k.page, result.is_ok()));
            // 그림 읽기와 폭 맞춤도 여기서. 메인 스레드는 캐시에 넣기만 한다
            let decoded = match result {
                Err(e) => Err(format!("{}: {}", t.plugin, e)),
                Ok(v) => serde_json::from_value::<ViewResult>(v)
                    .map_err(|e| format!("{}: bad views/render result: {}", t.plugin, e))
                    .and_then(|r| crate::preview_tabs::decode(r, k.width_px).map_err(|e| format!("{}: {}", t.plugin, e))),
            };
            // kitty면 압축·인코딩까지 여기서. 원본은 버린다
            let decoded = decoded.map(|(content, pages)| match content {
                crate::preview_tabs::Content::Image(img) if kitty => (crate::preview_tabs::Content::Kitty(crate::kitty::encode(&img, crate::kitty::next_id())), pages),
                other => (other, pages),
            });
            crate::trace::log(|| format!("tab.decoded {:.0}ms p{}", t0.elapsed().as_secs_f64() * 1000.0, k.page));
            let _ = tx.send((tab_index, k, decoded));
        });
        self.bg_tab = Some(rx);
        self.tab.pending = Some(key);
        self.tab_pending_since = Some(std::time::Instant::now());
    }

    /// 스레드가 끝낸 결과를 캐시에 넣는다.
    fn finish_tab(&mut self, key: crate::preview_tabs::CacheKey, result: Result<(crate::preview_tabs::Content, u32), String>) {
        self.tab.pending = None;
        // 항목이 바뀐 뒤 도착한 답은 버린다(쪽수도 옛 항목 것)
        if self.tab.entry_key.as_deref() != Some(key.entry_key.as_str()) {
            return;
        }
        match result {
            Err(e) => self.tab.error = Some(e),
            Ok((content, pages)) => {
                self.tab.set_pages(pages);
                self.tab.error = None;
                let gone = self.tab_cache.insert(key, content);
                self.kitty_deletes.extend(gone);
            }
        }
    }

    /// 이미지를 그릴 수 있는가.
    fn images_on(&self) -> bool {
        self.images.is_some()
    }

    /// 칸 하나의 픽셀 크기. 텍스트 모드는 1x1(줄 단위로 센다).
    fn tab_cell(&self) -> (u16, u16) {
        match &self.images {
            Some(p) => {
                let fs = p.font_size();
                (fs.width.max(1), fs.height.max(1))
            }
            None => (1, 1),
        }
    }

    /// 미리보기 패널 안쪽. 테두리 2칸과 상태 줄 1칸을 뺀다.
    fn tab_viewport(&self) -> crate::preview_tabs::Viewport {
        let a = self.panel_areas[2];
        let (cell_w, cell_h) = self.tab_cell();
        crate::preview_tabs::Viewport { cols: a.width.saturating_sub(2), rows: a.height.saturating_sub(3), cell_w, cell_h }
    }

    /// 플러그인에 요청할 폭. 텍스트 모드는 0.
    fn tab_width_px(&self) -> u32 {
        if self.images_on() { crate::preview_tabs::width_px(&self.tab_viewport(), self.tab.zoom_pct) } else { 0 }
    }

    fn tab_key(&self) -> Option<crate::preview_tabs::CacheKey> {
        let PreviewMode::Plugin(_) = self.preview_mode else { return None };
        let entry_key = self.tab.entry_key.clone()?;
        Some(crate::preview_tabs::CacheKey { entry_key, page: self.tab.page, width_px: self.tab_width_px() })
    }

    /// 현재 쪽 내용의 (세로 크기, 세로 창, 가로 크기, 가로 창). 텍스트는 줄, 이미지는 픽셀. 내용이 아직 없으면 None.
    fn tab_extents(&self) -> Option<(u32, u32, u32, u32)> {
        use crate::preview_tabs::Content;
        let vp = self.tab_viewport();
        match self.tab_cache.get(&self.tab_key()?)? {
            Content::Lines(l) => Some((l.len() as u32, vp.rows as u32, 0, 0)),
            Content::Image(img) => Some((img.height(), vp.view_h(), img.width(), vp.view_w())),
            Content::Kitty(page) => Some((page.height, vp.view_h(), page.width, vp.view_w())),
        }
    }

    /// 이미지 모드인가: 이미지를 그릴 수 있고, 이 쪽이 텍스트로 온 것이 아니다. 확대·pan은 이때만.
    /// (아직 만드는 중이어도 true. 그래야 `+`를 연달아 누를 수 있다.)
    fn tab_is_image(&self) -> bool {
        use crate::preview_tabs::Content;
        self.images_on() && !matches!(self.tab_key().and_then(|k| self.tab_cache.get(&k)), Some(Content::Lines(_)))
    }

    /// j/k 한 번. 텍스트 1줄, 이미지 3칸.
    fn tab_step(&self) -> u32 {
        if self.tab_is_image() { 3 * self.tab_cell().1 as u32 } else { 1 }
    }

    /// 탭을 가진 플러그인의 `max_zoom` 설정. 없으면 400.
    fn tab_max_zoom(&self) -> u32 {
        let PreviewMode::Plugin(i) = self.preview_mode else { return 400 };
        self.host
            .views()
            .get(i)
            .and_then(|t| self.config.plugins.get(&t.plugin))
            .and_then(|t| t.get("max_zoom"))
            .and_then(|v| v.as_integer())
            .map(|z| z.clamp(25, 1600) as u32)
            .unwrap_or(400)
    }

    /// 명령 요청의 params. 다중 선택이 있으면 그것, 없으면 커서 항목 하나.
    fn command_params(&self, command: &str, trigger: &str) -> CommandParams {
        let focus = match self.focus {
            Panel::Collections => "collections",
            Panel::Entries => "entries",
            Panel::Preview => "preview",
        };
        let entry = self.selected_entry().cloned();
        let entries: Vec<Entry> = if self.selected_keys.is_empty() {
            entry.iter().cloned().collect()
        } else {
            self.entries.iter().filter(|e| self.selected_keys.contains(&e.bibtex_key)).cloned().collect()
        };
        CommandParams {
            command: command.to_string(),
            trigger: trigger.to_string(),
            entry,
            entries,
            focus: Some(focus.to_string()),
            collection: self.current_collection().map(|s| s.to_string()),
        }
    }

    fn start_plugin_command(&mut self, id: PluginCmdId, trigger: &str) {
        let Some(cmd) = self.host.commands().get(id).cloned() else { return };
        let plugin = cmd.plugin.clone();
        let params = serde_json::to_value(self.command_params(&cmd.id, trigger)).unwrap_or(Value::Null);
        let host = Arc::clone(&self.host);
        let (tx, rx) = std::sync::mpsc::channel();
        let p = plugin.clone();
        std::thread::spawn(move || {
            let _ = tx.send(host.call(&p, "commands/run", params, None));
        });
        self.plugin_run = Some(PluginRun { plugin: plugin.clone(), rx });
        self.spinner_tick = 0;
        self.mode = Mode::Loading(format!("{}: running", plugin));
    }

    /// 명령 응답. 오류면 그 문구, 아니면 `message`가 상태 줄에. apply/refresh는 이제 플러그인이 따로 요청한다.
    fn finish_plugin(&mut self, plugin: &str, result: Result<Value, PluginError>) {
        self.mode = Mode::Normal;
        match result {
            Err(e) => self.show_message(format!("{}: {}", plugin, e)),
            Ok(v) => {
                let r: CommandResult = serde_json::from_value(v).unwrap_or_default();
                if let Some(m) = r.message {
                    self.show_message(m);
                }
            }
        }
    }

    /// `apply`를 검증해 통째로 반영한다. undo 스냅샷을 찍는다.
    /// `fire_hooks = false`는 훅 결과의 apply(재발화 방지)에만 쓴다.
    fn apply_plugin_entries(&mut self, incoming: Vec<serde_json::Value>, fire_hooks: bool) -> Result<(), String> {
        let updated = crate::plugin::protocol::validate_apply(&self.entries, None, &incoming)?;
        self.push_undo();
        for u in &updated {
            if let Some(slot) = self.entries.iter_mut().find(|e| e.id == u.id) {
                *slot = u.clone();
            }
        }
        self.persist(crate::events::WriteReason::Edit, updated, fire_hooks).map_err(|e| e.to_string())?;
        self.rebuild_collections();
        self.apply_filters();
        Ok(())
    }

    /// 플러그인이 `$BIBOX_BIN`으로 DB를 바꿨을 때. 커서를 citekey로 복원하고 노트 캐시를 버린다.
    fn refresh_from_disk(&mut self) -> Result<()> {
        let key = self.selected_entry().map(|e| e.bibtex_key.clone());
        let db_path = crate::config::resolve_db_path(&self.config);
        self.entries = load_db(&db_path)?.entries;
        self.rebuild_collections();
        self.apply_filters();
        if let Some(key) = key {
            if let Some(pos) = self.filtered.iter().position(|&i| self.entries[i].bibtex_key == key) {
                self.list_state.select(Some(pos));
            }
        }
        self.note_citekey.clear();
        self.update_preview();
        Ok(())
    }
}

fn draw_plugin_ui(f: &mut Frame, state: &PluginUiState, area: Rect) {
    match &state.kind {
        PluginUiKind::Pick { title, items, index } => {
            let height = (items.len() as u16 + 5).min(20);
            let popup_area = centered_rect(55, height, area);
            clear_area(f, popup_area);
            let visible = (height as usize).saturating_sub(5).max(1);
            let start = index.saturating_sub(visible.saturating_sub(1));
            let mut lines = vec![
                Line::from(Span::styled(title.clone(), Style::default().fg(theme().heading))),
                Line::from(""),
            ];
            for (i, item) in items.iter().enumerate().skip(start).take(visible) {
                let arrow = if i == *index { "▶ " } else { "  " };
                let style = if i == *index { Style::default().fg(theme().accent) } else { Style::default() };
                lines.push(Line::from(vec![
                    Span::styled(arrow, Style::default().fg(theme().heading)),
                    Span::styled(item.clone(), style),
                ]));
            }
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled("↑↓ select  Enter choose  Esc cancel", Style::default().fg(theme().muted))));
            let popup = Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(format!(" {} ", state.plugin)));
            f.render_widget(popup, popup_area);
        }
        PluginUiKind::Prompt { title, buf } => {
            let popup_area = centered_rect(60, 5, area);
            clear_area(f, popup_area);
            let text = Paragraph::new(vec![
                Line::from(Span::styled(title.clone(), Style::default().fg(theme().heading))),
                Line::from(format!("> {}▏", buf)),
            ])
            .block(Block::default().borders(Borders::ALL).title(format!(" {} ", state.plugin)));
            f.render_widget(text, popup_area);
        }
        PluginUiKind::Confirm { title } => draw_confirm_popup(f, &format!("{} (y/n)", title), area),
    }
}

fn handle_plugin_ui(app: &mut App, key: crossterm::event::KeyEvent) -> Result<bool> {
    let answer = match &mut app.mode {
        Mode::PluginUi(state) => plugin_ui_step(&mut state.kind, key.code),
        _ => None,
    };
    if let Some(answer) = answer {
        app.reply_plugin_ui(answer);
    }
    Ok(false)
}

// ── Drawing ──────────────────────────────────────────────────────────────────

/// 팝업 자리를 비운다. 테마에 배경이 있으면 `Clear`가 Reset으로 돌린 칸을 다시 칠한다.
fn clear_area(f: &mut Frame, area: Rect) {
    f.render_widget(Clear, area);
    let t = theme();
    if let Some(bg) = t.bg {
        f.render_widget(Block::default().style(Style::default().fg(t.fg).bg(bg)), area);
    }
}

fn draw(f: &mut Frame, app: &mut App) {
    let size = f.area();
    // 테마 배경. 뒤의 위젯은 bg를 지정하지 않으면 칸의 bg를 그대로 두므로 한 번이면 된다
    let t = theme();
    if let Some(bg) = t.bg {
        f.render_widget(Block::default().style(Style::default().fg(t.fg).bg(bg)), size);
    }

    // Main layout: content | status bar
    // 바를 끄면 그 한 줄을 패널에 돌려준다. 빈 줄로 남기지 않는다.
    let status_bar_height = if app.config.status_bar { 1 } else { 0 };
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(status_bar_height)])
        .split(size);

    // 3-panel horizontal: collections | entries | preview
    let r = app.config.panel_ratio;
    let total = r[0] + r[1] + r[2];
    let pct = |v: u16| -> u16 { (v as u32 * 100 / total as u32) as u16 };
    let panels = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(pct(r[0])),
            Constraint::Percentage(pct(r[1])),
            Constraint::Percentage(pct(r[2])),
        ])
        .split(outer[0]);

    app.panel_areas = [panels[0], panels[1], panels[2]];

    draw_collections_panel(f, app, panels[0]);
    draw_entries_panel(f, app, panels[1]);
    draw_preview_panel(f, app, panels[2]);
    if app.config.status_bar {
        draw_status_bar(f, app, outer[1]);
    }

    // File picker takes over full screen. ratatree paints only its own text, so wipe the panels first
    // or they show through between entries.
    if let Mode::FilePicker(_) = &app.mode {
        if let Some(picker_state) = &mut app.file_picker_state {
            clear_area(f, size);
            f.render_stateful_widget(ratatree::FilePicker::default(), size, picker_state);
        }
        return;
    }

    // Overlays
    match &app.mode {
        Mode::Confirm(ConfirmAction::Delete(key)) => {
            let key = key.clone();
            draw_confirm_popup(f, &format!("Delete '{}'? (y/n)", key), size);
        }
        Mode::Confirm(ConfirmAction::FetchPdf(key)) => {
            let key = key.clone();
            draw_confirm_popup(f, &format!("No PDF for '{}'. Fetch from web? (y/n)", key), size);
        }
        Mode::Confirm(ConfirmAction::OpenBrowser(key, _url)) => {
            let key = key.clone();
            draw_confirm_popup(f, &format!("Fetch failed (access denied). Open '{}' in browser? (y/n)", key), size);
        }
        Mode::Confirm(ConfirmAction::FetchMetaByTitle(_key, _title)) => {
            draw_confirm_popup(f, "No DOI. Search Crossref by title? (y/n)", size);
        }
        Mode::Confirm(ConfirmAction::RevealExport(path, n)) => {
            let lines = vec![
                app.config.msgs.exported_to(*n, &tilde_path(path)),
                String::new(),
                format!("{} (y/n)", app.config.msgs.reveal_question()),
            ];
            draw_confirm_lines(f, &lines, size);
        }
        Mode::Confirm(ConfirmAction::RemovePlugin(name)) => {
            let q = format!("{} (y/n)", app.config.msgs.plugin_remove_question(name));
            draw_settings_popup(f, app, size);
            draw_confirm_popup(f, &q, size);
        }
        Mode::Confirm(ConfirmAction::InstallStaged(staged)) => {
            let m = &app.config.msgs;
            let lines = vec![
                m.plugin_install_header(&staged.name, &staged.source),
                m.plugin_not_reviewed().to_string(),
                m.plugin_runs(&staged.run),
                m.plugin_runs_as_you().to_string(),
                String::new(),
                format!("{} (y/n)", m.plugin_install_question()),
            ];
            draw_settings_popup(f, app, size);
            draw_confirm_lines(f, &lines, size);
        }
        Mode::FetchPreview => {
            if let Some(ref state) = app.fetch_preview {
                draw_fetch_preview(f, state, size);
            }
        }
        Mode::SearchResultPicker => {
            if let Some(ref state) = app.search_picker {
                draw_search_result_picker(f, state, size);
            }
        }
        Mode::Message(msg) => {
            let msg = msg.clone();
            draw_message_popup(f, &msg, size);
        }
        Mode::Loading(msg) => {
            let frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
            let spinner = frames[app.spinner_tick % frames.len()];
            let text = format!("{} {}", spinner, msg);
            draw_message_popup(f, &text, size);
        }
        Mode::Help => draw_help_popup(f, app, size),
        Mode::SortMenu => draw_sort_popup(f, app, size),
        Mode::CollectionPicker | Mode::TagEditor => {
            if let Some(ref picker) = app.picker {
                draw_checklist_popup(f, picker, size);
            }
        }
        Mode::ExportMenu => {
            if let Some(ref es) = app.export_state {
                draw_export_popup(f, es, size);
            }
        }
        Mode::Settings => {
            draw_settings_popup(f, app, size);
        }
        Mode::SettingsInput(_) => {
            draw_settings_popup(f, app, size);
            if let Mode::SettingsInput(input) = &app.mode {
                draw_settings_input(f, input, size);
            }
        }
        Mode::ContextMenu => {
            draw_context_menu(f, app, size);
        }
        Mode::PluginUi(state) => draw_plugin_ui(f, state, size),
        Mode::CitationStyle(idx) => draw_citation_popup(f, *idx, size),
        _ => {}
    }
}

fn draw_collections_panel(f: &mut Frame, app: &App, area: Rect) {
    let focused = app.focus == Panel::Collections;
    let border_style = if focused {
        Style::default().fg(theme().accent)
    } else {
        Style::default().fg(theme().muted)
    };

    let mut items: Vec<ListItem> = vec![];

    // "All" item
    let all_count = app.entries.len();
    items.push(ListItem::new(format!("All ({})", all_count)));

    // Tree-style collection items: indent by depth, show only the last segment as label
    for col in &app.collections {
        let depth = col.matches('/').count();
        let label = col.split('/').next_back().unwrap_or(col.as_str());
        let prefix_str = format!("{}/", col);
        let count = app.entries.iter().filter(|e| {
            e.collections.iter().any(|c| c == col || c.starts_with(&prefix_str))
        }).count();
        let indent = "  ".repeat(depth);
        let connector = if depth > 0 { "└ " } else { "" };
        items.push(ListItem::new(format!("{}{}{} ({})", indent, connector, label, count)));
    }

    let title = if focused {
        Line::from(vec![
            Span::styled(" ● ", Style::default().fg(theme().accent).add_modifier(Modifier::BOLD)),
            Span::styled("Collections ", Style::default().fg(theme().accent).add_modifier(Modifier::BOLD)),
        ])
    } else {
        Line::from(Span::styled(" Collections ", Style::default().fg(theme().muted)))
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(title);

    let highlight_style = if focused {
        Style::default().fg(theme().selection_fg).bg(theme().selection_bg).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme().inactive_fg).bg(theme().inactive_bg).add_modifier(Modifier::BOLD)
    };

    let list = List::new(items)
        .block(block)
        .highlight_style(highlight_style)
        .highlight_symbol(if focused { "▸ " } else { "  " });

    let mut state = app.col_list_state;
    f.render_stateful_widget(list, area, &mut state);
}

fn draw_entries_panel(f: &mut Frame, app: &mut App, area: Rect) {
    app.entries_area_height = area.height.saturating_sub(2);
    // 테두리 2 + highlight symbol 2
    let inner_w = area.width.saturating_sub(4);
    let (field_decls, fields) = (&app.field_decls, &app.fields);
    let focused = app.focus == Panel::Entries;
    let border_style = if focused {
        Style::default().fg(theme().accent)
    } else {
        Style::default().fg(theme().muted)
    };

    use crate::config::LineNumbers;
    let selected_idx = app.list_state.selected().unwrap_or(0);

    let items: Vec<ListItem> = app.filtered.iter().enumerate().map(|(i, &idx)| {
        let e = &app.entries[idx];
        let title = e.title.as_deref().unwrap_or("(no title)");
        let author = e.author_display();
        let year = e.year.map(|y| y.to_string()).unwrap_or_else(|| "n.d.".to_string());
        let pdf_mark = if e.file_path.is_some() { " ◆" } else { "" };

        let line_num = match app.config.line_numbers {
            LineNumbers::Absolute => format!("{:>3} ", i + 1),
            LineNumbers::Relative => {
                if i == selected_idx {
                    format!("{:>3} ", i + 1)
                } else {
                    let diff = (i as isize - selected_idx as isize).unsigned_abs();
                    format!("{:>3} ", diff)
                }
            }
            LineNumbers::None => String::new(),
        };
        let num_style = if i == selected_idx {
            Style::default().fg(theme().heading)
        } else {
            Style::default().fg(theme().muted)
        };

        let pad = " ".repeat(line_num.len());
        let is_selected = app.selected_keys.contains(&e.bibtex_key);
        let sel_mark = if is_selected { "✓ " } else { "  " };
        let sel_style = if is_selected { Style::default().fg(theme().success) } else { Style::default() };

        // 플러그인 필드: 앵커 뒤에 한 칸 띄워, 오른쪽 정렬은 줄 끝에
        let (after1, right1) = row_cells(field_decls, fields, &e.bibtex_key, 1);
        let (after2, right2) = row_cells(field_decls, fields, &e.bibtex_key, 2);
        let (after3, right3) = row_cells(field_decls, fields, &e.bibtex_key, 3);
        let anchored = |cells: &[crate::plugin::fields::Placed], a: Anchor| -> Vec<Span<'static>> {
            cells.iter().filter(|c| c.anchor == Some(a)).map(|c| Span::styled(format!(" {}", c.text), field_style(&c.color))).collect()
        };
        let suffix = |left: &str, cells: &[crate::plugin::fields::Placed]| -> Option<Span<'static>> {
            let right: Vec<(String, u16)> = cells.iter().map(|c| (c.text.clone(), c.width)).collect();
            let s = row_suffix(inner_w, left, &right);
            if s.is_empty() { None } else { Some(Span::styled(s, field_style(&cells.first().and_then(|c| c.color.clone())))) }
        };
        let key_after = anchored(&after1, Anchor::Key);
        let pdf_after = anchored(&after1, Anchor::Pdf);
        let left1 = format!("{}{}{}{}{}{}", line_num, sel_mark, e.bibtex_key, key_after.iter().map(|s| s.content.as_ref()).collect::<String>(), pdf_mark, pdf_after.iter().map(|s| s.content.as_ref()).collect::<String>());
        let mut spans1 = vec![
            Span::styled(line_num.clone(), num_style),
            Span::styled(sel_mark, sel_style),
            Span::styled(e.bibtex_key.clone(), Style::default().fg(theme().heading).add_modifier(Modifier::BOLD)),
        ];
        spans1.extend(key_after);
        spans1.push(Span::styled(pdf_mark.to_string(), Style::default().fg(theme().success)));
        spans1.extend(pdf_after);
        spans1.extend(anchored(&after2, Anchor::Key));
        spans1.extend(suffix(&left1, &right1));
        let line1 = Line::from(spans1);
        let left2 = format!("{}  {}", pad, title);
        let mut spans2 = vec![Span::raw(left2.clone())];
        spans2.extend(anchored(&after2, Anchor::Pdf));
        spans2.extend(anchored(&after2, Anchor::Year));
        spans2.extend(suffix(&left2, &right2));
        let line2 = Line::from(spans2);
        let left3 = format!("{}  {} · {}", pad, author, year);
        let year_after = anchored(&after3, Anchor::Year);
        let left3_full = format!("{}{}", left3, year_after.iter().map(|s| s.content.as_ref()).collect::<String>());
        let mut spans3 = vec![Span::styled(left3, Style::default().fg(theme().muted))];
        spans3.extend(anchored(&after3, Anchor::Key));
        spans3.extend(anchored(&after3, Anchor::Pdf));
        spans3.extend(year_after);
        spans3.extend(suffix(&left3_full, &right3));
        let line3 = Line::from(spans3);

        let mut item = ListItem::new(Text::from(vec![line1, line2, line3]));
        if is_selected {
            item = item.style(Style::default().fg(theme().success));
        }
        item
    }).collect();

    let sel_count = app.selected_keys.len();
    let title_text = if !app.search_query.is_empty() {
        format!("Search: {} ({}) ", app.search_query, app.filtered.len())
    } else if sel_count > 0 {
        format!("Entries ({}) - {} selected ", app.filtered.len(), sel_count)
    } else {
        format!("Entries ({}) ", app.filtered.len())
    };

    let title = if focused {
        Line::from(vec![
            Span::styled(" ● ", Style::default().fg(theme().accent).add_modifier(Modifier::BOLD)),
            Span::styled(title_text, Style::default().fg(theme().accent).add_modifier(Modifier::BOLD)),
        ])
    } else {
        Line::from(Span::styled(format!(" {}", title_text), Style::default().fg(theme().muted)))
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(title);

    let highlight_style = if focused {
        Style::default().bg(theme().inactive_bg).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme().inactive_fg).bg(theme().inactive_bg)
    };

    let list = List::new(items)
        .block(block)
        .highlight_style(highlight_style)
        .highlight_symbol(if focused { "▸ " } else { "  " });

    f.render_stateful_widget(list, area, &mut app.list_state);
}

fn draw_preview_panel(f: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focus == Panel::Preview;
    let border_style = if focused {
        Style::default().fg(theme().accent)
    } else {
        Style::default().fg(theme().muted)
    };

    // Tab bar for preview modes. Info and Note are core; the rest come from plugins' [[tabs]]
    let modes = app.preview_modes();
    let tab_spans: Vec<Span> = modes.iter().map(|m| {
        if *m == app.preview_mode {
            Span::styled(
                format!(" {} ", app.preview_label(*m)),
                Style::default().fg(theme().selection_fg).bg(theme().selection_bg).add_modifier(Modifier::BOLD),
            )
        } else {
            Span::styled(format!(" {} ", app.preview_label(*m)), Style::default().fg(theme().muted))
        }
    }).collect();

    let mut title_spans = vec![];
    if focused {
        title_spans.push(Span::styled(" ● ", Style::default().fg(theme().accent).add_modifier(Modifier::BOLD)));
    } else {
        title_spans.push(Span::raw(" "));
    }
    for (i, s) in tab_spans.into_iter().enumerate() {
        title_spans.push(s);
        if i < modes.len() - 1 {
            title_spans.push(Span::styled(" │ ", Style::default().fg(theme().muted)));
        }
    }
    title_spans.push(Span::raw(" "));

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(Line::from(title_spans));

    let inner = block.inner(area);
    f.render_widget(block, area);

    match app.preview_mode {
        PreviewMode::Info => draw_preview_info(f, app, inner),
        PreviewMode::Note => draw_preview_note(f, app, inner),
        PreviewMode::Plugin(i) => draw_preview_plugin(f, app, inner, i),
    }
}

fn draw_preview_info(f: &mut Frame, app: &mut App, area: Rect) {
    let entry = match app.selected_entry() {
        Some(e) => e.clone(),
        None => {
            f.render_widget(Paragraph::new("No entry selected.").style(Style::default().fg(theme().muted)), area);
            return;
        }
    };

    let mut lines: Vec<Line<'static>> = vec![];

    if let Some(t) = &entry.title {
        lines.push(Line::from(Span::styled(t.clone(), Style::default().add_modifier(Modifier::BOLD))));
        lines.push(Line::from(""));
    }

    macro_rules! field {
        ($label:expr, $value:expr) => {
            lines.push(Line::from(vec![
                Span::styled(format!("{:<12}", $label), Style::default().fg(theme().accent)),
                Span::raw($value.to_string()),
            ]));
        };
    }

    field!("Key:", &entry.bibtex_key);
    field!("Type:", entry.entry_type.to_string());
    if !entry.author.is_empty() { field!("Author:", entry.author.join("; ")); }
    if let Some(y) = entry.year { field!("Year:", y.to_string()); }
    if let Some(m) = &entry.month { field!("Month:", m.clone()); }
    if let Some(j) = &entry.journal { field!("Journal:", j.clone()); }
    if let Some(bt) = &entry.booktitle { field!("Booktitle:", bt.clone()); }
    if let Some(p) = &entry.publisher { field!("Publisher:", p.clone()); }
    if let Some(doi) = &entry.doi { field!("DOI:", doi.clone()); }
    if let Some(url) = &entry.url { field!("URL:", url.clone()); }
    if let Some(hp) = &entry.howpublished { field!("Published:", hp.clone()); }
    if !entry.tags.is_empty() { field!("Tags:", entry.tags.join(", ")); }
    if !entry.collections.is_empty() {
        field!("Collections:", entry.collections.join(", "));
    } else {
        lines.push(Line::from(vec![
            Span::styled(format!("{:<12}", "Collections:"), Style::default().fg(theme().accent)),
            Span::styled("(none)", Style::default().fg(theme().muted)),
        ]));
    }
    if let Some(fp) = &entry.file_path {
        field!("File:", fp.clone());
    } else {
        lines.push(Line::from(vec![
            Span::styled(format!("{:<12}", "File:"), Style::default().fg(theme().accent)),
            Span::styled("No PDF", Style::default().fg(theme().muted)),
        ]));
    }
    if let Some(ref abs) = entry.abstract_text {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("Abstract:", Style::default().fg(theme().accent))));
        lines.push(Line::from(Span::styled(abs.clone(), Style::default().fg(theme().muted))));
    }

    let content_lines = lines.len() as u16;
    app.preview_max_scroll = content_lines.saturating_sub(1);
    app.preview_scroll = app.preview_scroll.min(app.preview_max_scroll);

    let p = Paragraph::new(lines)
        .scroll((app.preview_scroll, 0))
        .wrap(ratatui::widgets::Wrap { trim: true });
    f.render_widget(p, area);
}

fn draw_preview_note(f: &mut Frame, app: &mut App, area: Rect) {
    // Load note if needed
    if let Some(entry) = app.selected_entry() {
        let key = entry.bibtex_key.clone();
        if key != app.note_citekey {
            let note_path = app.config.notes_dir.join(format!("{}.md", key));
            app.note_content = if note_path.exists() {
                std::fs::read_to_string(&note_path).unwrap_or_else(|_| "Error reading note.".into())
            } else {
                "No note yet. Press N to create one.".into()
            };
            app.note_citekey = key;
        }
    } else {
        f.render_widget(
            Paragraph::new("No entry selected.").style(Style::default().fg(theme().muted)),
            area,
        );
        return;
    }

    let lines = render_markdown_to_lines(&app.note_content);

    let content_lines = lines.len() as u16;
    app.preview_max_scroll = content_lines.saturating_sub(1);
    app.preview_scroll = app.preview_scroll.min(app.preview_max_scroll);

    let p = Paragraph::new(lines)
        .scroll((app.preview_scroll, 0))
        .wrap(ratatui::widgets::Wrap { trim: true });
    f.render_widget(p, area);
}

fn render_markdown_to_lines(md: &str) -> Vec<Line<'static>> {
    use pulldown_cmark::{Event, Parser, Tag, TagEnd, HeadingLevel};

    let parser = Parser::new(md);
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut current_spans: Vec<Span<'static>> = Vec::new();

    // Style stack
    let mut bold = false;
    let mut italic = false;
    let mut in_heading: Option<HeadingLevel> = None;
    let mut in_blockquote = false;
    let mut _in_list = false;
    let mut list_ordered = false;
    let mut list_index: u64 = 0;
    let mut in_code_block = false;
    let mut list_item_started = false;

    for event in parser {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                in_heading = Some(level);
            }
            Event::End(TagEnd::Heading(_)) => {
                // Flush heading line
                let style = match in_heading {
                    Some(HeadingLevel::H1) => Style::default().fg(theme().heading).add_modifier(Modifier::BOLD),
                    Some(HeadingLevel::H2) => Style::default().fg(theme().accent).add_modifier(Modifier::BOLD),
                    _ => Style::default().fg(theme().success).add_modifier(Modifier::BOLD),
                };
                // Re-style all spans in this heading
                let heading_spans: Vec<Span<'static>> = current_spans.drain(..)
                    .map(|s| Span::styled(s.content.to_string(), style))
                    .collect();
                lines.push(Line::from(heading_spans));
                lines.push(Line::from(""));
                in_heading = None;
            }
            Event::Start(Tag::Strong) => { bold = true; }
            Event::End(TagEnd::Strong) => { bold = false; }
            Event::Start(Tag::Emphasis) => { italic = true; }
            Event::End(TagEnd::Emphasis) => { italic = false; }
            Event::Code(text) => {
                let style = Style::default().fg(theme().success).bg(theme().inactive_bg);
                current_spans.push(Span::styled(format!(" {} ", text), style));
            }
            Event::Start(Tag::CodeBlock(_)) => {
                if !current_spans.is_empty() {
                    lines.push(Line::from(std::mem::take(&mut current_spans)));
                }
                in_code_block = true;
            }
            Event::End(TagEnd::CodeBlock) => {
                in_code_block = false;
                lines.push(Line::from(""));
            }
            Event::Start(Tag::BlockQuote(_)) => { in_blockquote = true; }
            Event::End(TagEnd::BlockQuote(_)) => {
                in_blockquote = false;
                lines.push(Line::from(""));
            }
            Event::Start(Tag::List(ordered)) => {
                _in_list = true;
                list_ordered = ordered.is_some();
                list_index = ordered.unwrap_or(1);
            }
            Event::End(TagEnd::List(_)) => {
                _in_list = false;
                lines.push(Line::from(""));
            }
            Event::Start(Tag::Item) => {
                list_item_started = true;
            }
            Event::End(TagEnd::Item) => {
                if !current_spans.is_empty() {
                    lines.push(Line::from(std::mem::take(&mut current_spans)));
                }
            }
            Event::Start(Tag::Paragraph) => {}
            Event::End(TagEnd::Paragraph) => {
                if !current_spans.is_empty() {
                    lines.push(Line::from(std::mem::take(&mut current_spans)));
                }
                lines.push(Line::from(""));
            }
            Event::Text(text) => {
                if in_code_block {
                    // Code block: render each line with background
                    let style = Style::default().fg(theme().inactive_fg).bg(theme().inactive_bg);
                    for line in text.lines() {
                        lines.push(Line::from(Span::styled(format!("  {}", line), style)));
                    }
                } else {
                    // Handle list bullet/number prefix
                    if list_item_started {
                        let prefix = if in_blockquote { "│ " } else { "" };
                        if list_ordered {
                            current_spans.push(Span::styled(
                                format!("{}  {}. ", prefix, list_index),
                                Style::default().fg(theme().muted),
                            ));
                            list_index += 1;
                        } else {
                            current_spans.push(Span::styled(
                                format!("{}  • ", prefix),
                                Style::default().fg(theme().muted),
                            ));
                        }
                        list_item_started = false;
                    } else if in_blockquote && current_spans.is_empty() {
                        current_spans.push(Span::styled(
                            "│ ".to_string(),
                            Style::default().fg(theme().accent),
                        ));
                    }

                    let mut style = Style::default();
                    if bold { style = style.add_modifier(Modifier::BOLD); }
                    if italic { style = style.add_modifier(Modifier::ITALIC); }
                    if in_blockquote { style = style.fg(theme().muted).add_modifier(Modifier::ITALIC); }
                    current_spans.push(Span::styled(text.to_string(), style));
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if !current_spans.is_empty() {
                    lines.push(Line::from(std::mem::take(&mut current_spans)));
                }
            }
            Event::Rule => {
                lines.push(Line::from(Span::styled(
                    "────────────────────────────────".to_string(),
                    Style::default().fg(theme().muted),
                )));
            }
            _ => {}
        }
    }

    // Flush remaining
    if !current_spans.is_empty() {
        lines.push(Line::from(current_spans));
    }

    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "Empty note.".to_string(),
            Style::default().fg(theme().muted),
        )));
    }

    lines
}

/// 플러그인 탭. 캐시에 있는 쪽을 이미지(잘라서) 또는 텍스트로 그리고, 없으면 요청을 보낸다.
fn draw_preview_plugin(f: &mut Frame, app: &mut App, area: Rect, tab_index: usize) {
    use crate::preview_tabs::Content;
    let entry = app.selected_entry().cloned();
    let Some(entry) = entry else {
        f.render_widget(Paragraph::new(app.config.msgs.tab_no_entry()).style(Style::default().fg(theme().muted)), area);
        return;
    };
    if app.tab.on_entry(Some(&entry.bibtex_key)) {
        app.tab_entry_since = std::time::Instant::now();
    }
    let gone = app.tab_cache.retain_entry(&entry.bibtex_key);
    app.kitty_deletes.extend(gone);
    if entry.file_path.is_none() {
        f.render_widget(Paragraph::new(app.config.msgs.tab_no_pdf()).style(Style::default().fg(theme().muted)), area);
        return;
    }
    let rows = Layout::default().direction(Direction::Vertical).constraints([Constraint::Min(1), Constraint::Length(1)]).split(area);
    let body = rows[0];
    let images = app.images_on();
    let Some(key) = app.tab_key() else { return };
    // 오류가 남아 있으면 다시 묻지 않는다(매 프레임 플러그인을 띄우는 루프를 막는다). 쪽·배율을 바꾸면 오류가 지워져 다시 묻는다.
    // 항목이 방금 바뀌었으면 커서가 멈출 때까지 기다린다
    let waiting = app.tab.error.is_none() && app.tab_cache.get(&key).is_none();
    let settled = crate::preview_tabs::settled(app.tab_entry_since, std::time::Instant::now());
    if waiting && settled && app.tab.pending.as_ref() != Some(&key) && app.bg_tab.is_none() {
        app.request_tab(tab_index, key.clone(), images);
    }
    // error를 복제해 두어야 아래 팔에서 app.tab을 고칠 수 있다(match 대상이 빌린 채로 남는다)
    let error = app.tab.error.clone();
    let pending = app.tab.pending.is_some();
    let (cell_w, cell_h) = app.tab_cell();
    let dim = Style::default().fg(theme().muted);
    // 만드는 중이면 스피너. 루프가 16ms마다 그리므로 시각만으로 돈다
    let rendering = {
        let ms = app.tab_pending_since.map(|t| t.elapsed().as_millis()).unwrap_or(0);
        format!("{} {}", crate::preview_tabs::spinner(ms), app.config.msgs.tab_rendering(app.tab.page))
    };
    let status = match (error, pending, app.tab_cache.get(&key)) {
        // 오류는 본문에 줄바꿈해서. 상태 줄 한 칸에는 poppler 안내 같은 긴 문장이 안 들어간다
        (Some(e), _, _) => {
            f.render_widget(Paragraph::new(e).style(Style::default().fg(theme().error)).wrap(Wrap { trim: false }), body);
            Line::from(Span::styled(format!("page {}/{}", app.tab.page, app.tab.pages), Style::default().fg(theme().muted)))
        }
        (None, _, Some(Content::Lines(lines))) => {
            let view = body.height as u32;
            app.tab.clamp(lines.len() as u32, view, 0, 0);
            let p = Paragraph::new(lines.iter().map(|l| Line::from(l.as_str())).collect::<Vec<_>>()).scroll((app.tab.scroll as u16, 0));
            f.render_widget(p, body);
            if pending {
                Line::from(Span::styled(rendering.clone(), dim))
            } else {
                Line::from(Span::styled(format!("page {}/{}  text", app.tab.page, app.tab.pages), dim))
            }
        }
        (None, _, Some(Content::Kitty(page))) => {
            let vp = crate::preview_tabs::Viewport { cols: body.width, rows: body.height, cell_w, cell_h };
            app.tab.clamp(page.height, vp.view_h(), page.width, vp.view_w());
            let scroll_rows = crate::preview_tabs::scroll_rows(app.tab.scroll, cell_h);
            let pan_cols = (app.tab.pan / cell_w.max(1) as u32).min(u16::MAX as u32) as u16;
            f.render_widget(KittyPage { page, scroll_rows, pan_cols, cell_w, cell_h }, body);
            if pending {
                Line::from(Span::styled(rendering.clone(), dim))
            } else {
                Line::from(vec![
                    Span::styled(format!("page {}/{}  {}%", app.tab.page, app.tab.pages, app.tab.zoom_pct), dim),
                    Span::styled("  [kitty]", dim.add_modifier(Modifier::DIM)),
                ])
            }
        }
        (None, _, Some(Content::Image(img))) => {
            let vp = crate::preview_tabs::Viewport { cols: body.width, rows: body.height, cell_w, cell_h };
            app.tab.clamp(img.height(), vp.view_h(), img.width(), vp.view_w());
            // 쪽은 한 번만 올린다. 스크롤은 행만 옮기고, 쪽·배율·pan이 바뀔 때만 다시 만든다
            let stale = app.tab_sliced.as_ref().map(|(k, p, _)| *k != key || *p != app.tab.pan).unwrap_or(true);
            if stale {
                if let Some(picker) = app.images.as_ref() {
                    let t0 = std::time::Instant::now();
                    app.tab_sliced = page_protocol(picker, img, &vp, app.tab.pan).map(|s| (key.clone(), app.tab.pan, s));
                    crate::trace::log(|| format!("tab.proto {:.0}ms {}x{} p{}", t0.elapsed().as_secs_f64() * 1000.0, img.width(), img.height(), key.page));
                }
            }
            if let Some((_, _, sliced)) = app.tab_sliced.as_ref() {
                f.render_widget(page_widget(sliced, crate::preview_tabs::scroll_rows(app.tab.scroll, cell_h)), body);
            }
            let proto_name = app.images.as_ref().map(|p| format!("{:?}", p.protocol_type()).to_lowercase()).unwrap_or_default();
            if pending {
                Line::from(Span::styled(rendering.clone(), dim))
            } else {
                Line::from(vec![
                    Span::styled(format!("page {}/{}  {}%", app.tab.page, app.tab.pages, app.tab.zoom_pct), dim),
                    Span::styled(format!("  [{}]", proto_name), dim.add_modifier(Modifier::DIM)),
                ])
            }
        }
        // 아직 아무것도 없으면(만드는 중이거나 커서가 멈추길 기다리는 중) 본문 한가운데에. 상태 줄 한 칸은 눈에 안 띈다
        (None, _, None) if waiting => {
            let mid = Rect { y: body.y + body.height / 2, height: 1.min(body.height), ..body };
            f.render_widget(Paragraph::new(rendering.clone()).style(Style::default().fg(theme().fg)).alignment(ratatui::layout::Alignment::Center), mid);
            Line::from("")
        }
        (None, _, None) => Line::from(""),
    };
    f.render_widget(Paragraph::new(status), rows[1]);
}

/// 하단 바 문자열. 키는 전부 활성 키맵에서 역조회하므로 리맵하면 바도 따라 바뀐다.
///
/// 도움말 오버레이가 생긴 뒤로 여기에 액션 키를 전부 늘어놓을 이유가 없어졌다.
/// 하루에 여러 번 쓰는 것만 남기고 나머지는 도움말이 안내한다.
fn status_bar_text(keymap: &Keymap, focus: Panel) -> String {
    let layer = keymap.layer(layer_for(focus));
    let k = |a: Action| shortcut_hint(layer, a);

    // 바인딩이 없는 항목은 통째로 뺀다. 빈 힌트를 남기면 " search"처럼 앞이 빈다.
    fn push(parts: &mut Vec<String>, hint: String, label: &str) {
        if !hint.is_empty() {
            parts.push(format!("{} {}", hint, label));
        }
    }
    /// 화살표 라벨은 키에 붙여 쓴다. "h←collections"가 "h는 왼쪽 컬렉션으로"로 읽힌다.
    fn push_arrow(parts: &mut Vec<String>, hint: String, label: &str) {
        if !hint.is_empty() {
            parts.push(format!("{}{}", hint, label));
        }
    }
    fn push_pair(parts: &mut Vec<String>, down: String, up: String, label: &str) {
        if !down.is_empty() && !up.is_empty() {
            parts.push(format!("{}/{} {}", down, up, label));
        }
    }

    let mut parts: Vec<String> = Vec::new();

    match focus {
        Panel::Collections => {
            push_arrow(&mut parts, k(Action::FocusEntries), "→entries");
            push_pair(&mut parts, k(Action::CollectionDown), k(Action::CollectionUp), "navigate");
        }
        Panel::Entries => {
            push_arrow(&mut parts, k(Action::FocusCollections), "←collections");
            push_arrow(&mut parts, k(Action::FocusPreview), "→preview");
            push_pair(&mut parts, k(Action::EntryDown), k(Action::EntryUp), "navigate");
        }
        Panel::Preview => {
            push_arrow(&mut parts, k(Action::PrevTabOrFocusEntries), "←entries");
            push(&mut parts, k(Action::NextPreviewTab), "switch mode");
            push_pair(&mut parts, k(Action::PreviewScrollDown), k(Action::PreviewScrollUp), "scroll");
        }
    }

    let navigation = parts.join("  ");
    parts.clear();
    for (action, label) in [
        (Action::Search, "search"),
        (Action::OpenPdf, "open"),
        (Action::ExportMenu, "export"),
        (Action::Help, "help"),
        (Action::Quit, "quit"),
    ] {
        push(&mut parts, k(action), label);
    }
    let actions = parts.join("  ");

    match (navigation.is_empty(), actions.is_empty()) {
        (false, false) => format!("{}  │  {}", navigation, actions),
        (false, true) => navigation,
        (true, false) => actions,
        (true, true) => String::new(),
    }
}

fn draw_status_bar(f: &mut Frame, app: &App, area: Rect) {
    let status = match &app.mode {
        Mode::Search => {
            if app.focus == Panel::Collections {
                format!("/ {} (collection search, Esc to clear)", app.col_search_query)
            } else {
                format!("/ {} (Esc to clear)", app.search_query)
            }
        }
        _ => status_bar_text(&app.keymap, app.focus),
    };
    // 오른쪽에 플러그인 조각. 조각이 없으면 지금과 같은 화면
    let segments: Vec<String> = if app.config.plugin_status_bar { app.status_segments.values().map(|v| v.text.clone()).collect() } else { Vec::new() };
    let status = crate::plugin::fields::status_line(area.width, &status, &segments);
    let status_widget = Paragraph::new(status).style(Style::default().fg(theme().muted));
    f.render_widget(status_widget, area);
}

/// Split a batch of drained terminal events into the keystrokes to replay and the
/// single mouse event worth acting on.
///
/// Mouse scrolls arrive faster than they can be drawn, so only the newest one
/// matters. Keystrokes are the opposite: every one is meaningful and the order is
/// the user's typing, so they must all survive.
fn coalesce_events(events: &[Event]) -> (Vec<crossterm::event::KeyEvent>, Option<crossterm::event::MouseEvent>) {
    let mut keys = Vec::new();
    let mut mouse = None;
    for ev in events {
        match ev {
            Event::Key(k) => keys.push(*k),
            Event::Mouse(m) => mouse = Some(*m),
            _ => {}
        }
    }
    (keys, mouse)
}

fn centered_rect(percent_x: u16, height: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length((r.height.saturating_sub(height)) / 2),
            Constraint::Length(height),
            Constraint::Min(0),
        ])
        .split(r);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

/// 이 액션에 걸린 첫 바인딩을 레이어에서 역조회한다. 사용자가 리맵했으면
/// 리맵한 키가 그대로 나온다. 바인딩이 없으면 빈 문자열이다.
fn shortcut_hint(layer: &crate::keymap::Layer, action: Action) -> String {
    layer
        .bindings
        .iter()
        .find(|b| b.actions.len() == 1 && b.actions[0] == action)
        .map(|b| b.keys.iter().map(|k| crate::keymap::render_key(*k)).collect::<Vec<_>>().join(""))
        .unwrap_or_default()
}

/// 내장 10개 뒤에 `menus`에 "context"가 있는 플러그인 명령. 라벨은 매니페스트의 `desc`.
fn context_menu_items(commands: &crate::plugin::PluginCommands) -> Vec<(String, Action)> {
    let mut items: Vec<(String, Action)> = ContextMenuState::ITEMS
        .iter()
        .map(|(l, a)| (l.to_string(), *a))
        .collect();
    for (id, c) in commands.iter() {
        if c.menus.iter().any(|m| m == "context") {
            items.push((c.desc.clone(), Action::Plugin(id)));
        }
    }
    items
}

impl App {
    fn context_menu_items(&self) -> Vec<(String, Action)> {
        context_menu_items(self.host.commands())
    }
}

fn draw_context_menu(f: &mut Frame, app: &App, screen: Rect) {
    let items = app.context_menu_items();
    let menu_w: u16 = 30;
    let menu_h = items.len() as u16 + 2; // +2 for borders

    let x = if app.context_menu.x + menu_w > screen.width {
        screen.width.saturating_sub(menu_w)
    } else {
        app.context_menu.x
    };
    let y = if app.context_menu.y + menu_h > screen.height {
        screen.height.saturating_sub(menu_h)
    } else {
        app.context_menu.y
    };

    let area = Rect::new(x, y, menu_w, menu_h);
    clear_area(f, area);

    let list_items: Vec<ListItem> = items.iter().enumerate().map(|(i, (label, action))| {
        let key = shortcut_hint(&app.keymap.entries, *action);
        let style = if i == app.context_menu.index {
            Style::default().fg(theme().selection_fg).bg(theme().selection_bg).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        ListItem::new(Line::from(vec![
            Span::styled(format!(" {:<20}", label), style),
            Span::styled(format!("{:>3} ", key), style.add_modifier(Modifier::DIM)),
        ]))
    }).collect();

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme().accent))
        .title(Span::styled(" Actions ", Style::default().fg(theme().accent).add_modifier(Modifier::BOLD)));

    let list = List::new(list_items).block(block);
    f.render_widget(list, area);
}

fn draw_confirm_popup(f: &mut Frame, msg: &str, area: Rect) {
    let popup_area = centered_rect(60, 5, area);
    clear_area(f, popup_area);
    let text = Paragraph::new(msg)
        .block(Block::default().borders(Borders::ALL).title(" Confirm "))
        .style(Style::default().fg(theme().error));
    f.render_widget(text, popup_area);
}

/// 홈 디렉토리를 `~`로 줄인 표시용 경로.
fn tilde_path(path: &std::path::Path) -> String {
    if let Some(home) = dirs::home_dir() {
        if let Ok(rest) = path.strip_prefix(&home) {
            return format!("~/{}", rest.display());
        }
    }
    path.display().to_string()
}

/// macOS는 Finder에서 파일을 선택해 보여 주고, 그 밖에서는 디렉토리를 연다.
fn reveal_in_file_manager(path: &std::path::Path) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg("-R").arg(path).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn();
    #[cfg(not(target_os = "macos"))]
    let _ = std::process::Command::new("xdg-open").arg(path.parent().unwrap_or(path)).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn();
}

fn draw_confirm_lines(f: &mut Frame, lines: &[String], area: Rect) {
    let popup_area = centered_rect(70, (lines.len() as u16 + 2).max(5), area);
    clear_area(f, popup_area);
    let text: Vec<Line> = lines.iter().map(|l| Line::from(l.as_str())).collect();
    let p = Paragraph::new(text)
        .block(Block::default().borders(Borders::ALL).title(" Confirm "))
        .style(Style::default().fg(theme().error));
    f.render_widget(p, popup_area);
}

/// 결과 팝업. 긴 문장(git 오류 등)은 줄바꿈하고 그만큼 키운다. 한 줄에 잘리면 이유를 못 읽는다.
fn draw_message_popup(f: &mut Frame, msg: &str, area: Rect) {
    // ratatui의 Wrap은 폭보다 긴 단어(경로)를 글자 단위로 자르므로 단어 수와 글자 수 두 추정 중 큰 쪽에 한 줄 여유
    let inner_w = (area.width * 60 / 100).saturating_sub(2).max(10) as usize;
    let lines: usize = msg
        .lines()
        .map(|l| crate::settings::wrap_words(l, inner_w).len().max(l.chars().count().div_ceil(inner_w)).max(1))
        .sum();
    let height = (lines as u16 + 3).clamp(5, area.height.saturating_sub(2).max(5));
    let popup_area = centered_rect(60, height, area);
    clear_area(f, popup_area);
    let text = Paragraph::new(msg)
        .block(Block::default().borders(Borders::ALL).title(" Info "))
        .style(Style::default().fg(theme().success))
        .wrap(Wrap { trim: false });
    f.render_widget(text, popup_area);
}

/// One keyboard shortcut as data, so the help screen can be searched and tested
/// rather than being a block of hardcoded prose that drifts from the handlers.
/// 화면에 나오는 섹션 순서. `Action::section()`이 돌려주는 값과 같아야 한다.
const SECTION_ORDER: &[&str] = &[
    "Navigation", "Selection", "Entry actions", "Editing", "Search and sort", "Application", "Plugins",
];

struct HelpRow {
    keys: String,
    action: String,
    desc: String,
    section: &'static str,
}

/// 도움말 표는 활성 레이어의 바인딩에서 생성된다. 손으로 유지하지 않는다.
///
/// `draw_help_popup`이 "섹션이 바뀌면 제목을 끼워 넣는" 방식이라 행이 섹션순으로
/// 정렬돼 있어야 한다. 옛 `const HELP`는 손으로 그 순서를 맞춰 두었지만 키맵의
/// 바인딩 순서는 그렇지 않으므로 여기서 정렬한다. 안정 정렬이라 같은 섹션 안의
/// 순서는 키맵에 쓴 순서를 따른다.
fn help_rows(layer: &crate::keymap::Layer, commands: &crate::plugin::PluginCommands) -> Vec<HelpRow> {
    let mut rows: Vec<HelpRow> = layer
        .bindings
        .iter()
        .filter(|b| b.actions.first().is_some_and(|a| *a != Action::Noop))
        .map(|b| {
            let first = b.actions[0];
            let (action, fallback_desc) = match first {
                Action::Plugin(id) => match commands.get(id) {
                    Some(c) => (c.full_name(), c.desc.clone()),
                    None => (format!("{:?}", first), first.desc().to_string()),
                },
                other => (format!("{:?}", other), other.desc().to_string()),
            };
            HelpRow {
                keys: b.keys.iter().map(|k| crate::keymap::render_key(*k)).collect::<Vec<_>>().join(""),
                action,
                desc: b.desc.clone().unwrap_or(fallback_desc),
                section: first.section(),
            }
        })
        .collect();
    rows.sort_by_key(|r| SECTION_ORDER.iter().position(|s| *s == r.section).unwrap_or(usize::MAX));

    // 같은 동작에 걸린 별칭 키는 한 행으로 합친다. 옛 `const HELP`가 "h  ←"처럼
    // 손으로 합쳐 두었던 것과 같은 모양이고, 합치지 않으면 j와 <Down>이 따로 나와
    // 표가 알맹이 없이 길어진다.
    let mut merged: Vec<HelpRow> = Vec::new();
    for r in rows {
        match merged.last_mut() {
            Some(prev) if prev.section == r.section && prev.action == r.action && prev.desc == r.desc => {
                prev.keys.push_str("  ");
                prev.keys.push_str(&r.keys);
            }
            _ => merged.push(r),
        }
    }
    merged
}

/// `query`에 걸리는 행. 빈 질의는 전부 돌려준다. 키, 액션 이름, 설명 전부에서
/// 대소문자를 무시하고 부분 문자열로 찾는다. "copy"가 설명에만 있는 행도 걸린다.
fn filter_rows<'a>(rows: &'a [HelpRow], query: &str) -> Vec<&'a HelpRow> {
    if query.is_empty() {
        return rows.iter().collect();
    }
    let q = query.to_lowercase();
    rows.iter()
        .filter(|r| {
            r.keys.to_lowercase().contains(&q)
                || r.action.to_lowercase().contains(&q)
                || r.desc.to_lowercase().contains(&q)
        })
        .collect()
}

fn draw_help_popup(f: &mut Frame, app: &App, area: Rect) {
    let height = area.height.saturating_sub(4).max(10);
    let popup_area = centered_rect(90, height, area);
    clear_area(f, popup_area);

    let all = help_rows(app.keymap.layer(layer_for(app.focus)), app.host.commands());
    let rows = filter_rows(&all, &app.help_query);

    // Section headers are inserted between groups, so the row list the user sees
    // is longer than the binding list that was filtered.
    let mut lines: Vec<Line> = Vec::new();
    let mut last_section: Option<&str> = None;
    for r in &rows {
        if last_section != Some(r.section) {
            if last_section.is_some() { lines.push(Line::from("")); }
            lines.push(Line::from(Span::styled(
                format!(" {}", r.section),
                Style::default().fg(theme().accent).add_modifier(Modifier::BOLD),
            )));
            last_section = Some(r.section);
        }
        lines.push(Line::from(vec![
            Span::styled(format!("  {:<16}", r.keys), Style::default().fg(theme().heading)),
            Span::styled(format!("{:<22}", r.action), Style::default().fg(theme().fg)),
            Span::styled(r.desc.clone(), Style::default().fg(theme().muted)),
        ]));
    }
    if rows.is_empty() {
        lines.push(Line::from(Span::styled(
            "  (no shortcut matches this filter)",
            Style::default().fg(theme().muted),
        )));
    }

    let title = if app.help_query.is_empty() {
        format!(" Help  ({} shortcuts) ", rows.len())
    } else {
        format!(" Help  ({} of {} shortcuts) ", rows.len(), all.len())
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme().accent))
        .title(Span::styled(title, Style::default().fg(theme().heading).add_modifier(Modifier::BOLD)));
    let inner = block.inner(popup_area);
    f.render_widget(block, popup_area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(inner);

    // Clamp here as well as in the handler: the handler does not know the viewport
    // height, so it can only stop the offset from running past the last row.
    let visible = chunks[0].height as usize;
    let max_scroll = lines.len().saturating_sub(visible);
    let scroll = app.help_scroll.min(max_scroll);

    let body: Vec<Line> = lines.into_iter().skip(scroll).collect();
    f.render_widget(Paragraph::new(body), chunks[0]);

    let footer = if app.help_filtering {
        Line::from(vec![
            Span::styled(" Filter: ", Style::default().fg(theme().heading)),
            Span::styled(app.help_query.clone(), Style::default().fg(theme().fg)),
            Span::styled("▌", Style::default().fg(theme().heading)),
        ])
    } else if !app.help_query.is_empty() {
        Line::from(vec![
            Span::styled(" Filter: ", Style::default().fg(theme().muted)),
            Span::styled(app.help_query.clone(), Style::default().fg(theme().fg)),
            Span::styled("   Esc clear   / edit   j/k scroll   q close", Style::default().fg(theme().muted)),
        ])
    } else {
        Line::from(Span::styled(
            " / filter    j/k ↑/↓ scroll    C-d/C-u half page    g/G top/bottom    Esc or q close",
            Style::default().fg(theme().muted),
        ))
    };
    f.render_widget(Paragraph::new(footer), chunks[1]);
}

fn draw_sort_popup(f: &mut Frame, app: &App, area: Rect) {
    let popup_area = centered_rect(55, 14, area);
    clear_area(f, popup_area);
    let criteria = SortCriterion::all();
    let mut lines = vec![
        Line::from(Span::styled("Sort by:", Style::default().fg(theme().heading))),
        Line::from(""),
    ];
    for (i, c) in criteria.iter().enumerate() {
        let selected = *c == app.sort_by;
        let arrow = if i == app.sort_menu_index { "▶ " } else { "  " };
        let dir = if selected { if app.sort_ascending { "↑ asc" } else { "↓ desc" } } else { "     " };
        let style = if selected { Style::default().fg(theme().accent) } else { Style::default() };
        lines.push(Line::from(vec![
            Span::styled(arrow, Style::default().fg(theme().heading)),
            Span::styled(format!("{:<12}", c.label()), style),
            Span::styled(dir, Style::default().fg(theme().muted)),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("↑↓ select  Enter apply  Space toggle ↑↓  Esc cancel", Style::default().fg(theme().muted))));
    let popup = Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(" Sort "));
    f.render_widget(popup, popup_area);
}

fn draw_checklist_popup(f: &mut Frame, picker: &ChecklistPicker, area: Rect) {
    let item_count = picker.items.len() + 3;
    let height = (item_count as u16 + 4).min(20);
    let popup_area = centered_rect(60, height, area);
    clear_area(f, popup_area);

    let mut lines = vec![
        Line::from(Span::styled(&picker.title, Style::default().fg(theme().heading))),
        Line::from(""),
    ];
    for (i, (name, checked)) in picker.items.iter().enumerate() {
        let arrow = if i == picker.index { "▶ " } else { "  " };
        let check = if *checked { "[x]" } else { "[ ]" };
        let check_style = if *checked { Style::default().fg(theme().success) } else { Style::default().fg(theme().muted) };
        lines.push(Line::from(vec![
            Span::styled(arrow, Style::default().fg(theme().heading)),
            Span::styled(format!("{} ", check), check_style),
            Span::raw(name.as_str()),
        ]));
    }
    lines.push(Line::from(Span::styled("  ─────────────────", Style::default().fg(theme().muted))));
    let new_arrow = if picker.is_on_new_item() { "▶ " } else { "  " };
    if let Some(ref input) = picker.new_item_input {
        lines.push(Line::from(vec![
            Span::styled(new_arrow, Style::default().fg(theme().heading)),
            Span::styled(format!("{}▏", input), Style::default().fg(theme().accent)),
        ]));
    } else {
        lines.push(Line::from(vec![
            Span::styled(new_arrow, Style::default().fg(theme().heading)),
            Span::styled(&picker.new_item_label, Style::default().fg(theme().accent)),
        ]));
    }
    lines.push(Line::from(""));
    let footer = if picker.in_input_mode() { "Enter confirm  Esc cancel" } else { "↑↓ navigate  Space toggle  Enter done  Esc cancel" };
    lines.push(Line::from(Span::styled(footer, Style::default().fg(theme().muted))));
    let popup = Paragraph::new(lines).block(Block::default().borders(Borders::ALL));
    f.render_widget(popup, popup_area);
}

fn draw_export_popup(f: &mut Frame, es: &ExportState, area: Rect) {
    let height = (es.scope_options.len() + 10) as u16;
    let popup_area = centered_rect(60, height.min(18), area);
    clear_area(f, popup_area);

    let formats = [ExportFormat::BibTeX, ExportFormat::Yaml, ExportFormat::Ris];
    let mut lines = vec![
        Line::from(Span::styled("Scope:", Style::default().fg(theme().heading))),
    ];
    for (i, (_, label)) in es.scope_options.iter().enumerate() {
        let arrow = if es.section == 0 && i == es.scope_idx { "▶ " } else { "  " };
        let style = if i == es.scope_idx { Style::default().fg(theme().accent) } else { Style::default() };
        lines.push(Line::from(vec![
            Span::styled(arrow, Style::default().fg(theme().heading)),
            Span::styled(label.as_str(), style),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("Format:", Style::default().fg(theme().heading))));
    for (i, fmt) in formats.iter().enumerate() {
        let arrow = if es.section == 1 && i == es.format_idx { "▶ " } else { "  " };
        let style = if i == es.format_idx { Style::default().fg(theme().accent) } else { Style::default() };
        lines.push(Line::from(vec![
            Span::styled(arrow, Style::default().fg(theme().heading)),
            Span::styled(fmt.label(), style),
        ]));
    }
    lines.push(Line::from(""));
    let pdf_arrow = if es.section == 2 { "▶ " } else { "  " };
    let pdf_check = if es.include_pdf { "[x]" } else { "[ ]" };
    let pdf_style = if es.include_pdf { Style::default().fg(theme().success) } else { Style::default().fg(theme().muted) };
    lines.push(Line::from(vec![
        Span::styled(pdf_arrow, Style::default().fg(theme().heading)),
        Span::styled(pdf_check, pdf_style),
        Span::raw(" Include PDFs"),
    ]));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "↑↓ navigate  Tab section  Space toggle  Enter export  Esc cancel",
        Style::default().fg(theme().muted),
    )));

    let popup = Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(" Export "));
    f.render_widget(popup, popup_area);
}

/// 현재 절(또는 페이지, 검색)의 오른쪽 칸 행. 커서 이동과 그리기가 같은 목록을 본다.
fn settings_pane_rows(app: &App, items: &[crate::settings::Item]) -> Vec<PaneRow> {
    use crate::settings::Section;
    let st = &app.settings;
    if let Some(q) = &st.query {
        let plugins: Vec<(String, String)> = st.plugins.iter().map(|p| (p.name.clone(), p.description.clone())).collect();
        return crate::settings::search(items, &plugins, q)
            .into_iter()
            .map(|r| match r {
                crate::settings::Row::Header(h) => PaneRow::Header(h),
                crate::settings::Row::Item(i) => PaneRow::Item(i),
                crate::settings::Row::Plugin(p) => PaneRow::Plugin(p),
            })
            .collect();
    }
    if st.section() != Section::Plugins {
        return items.iter().enumerate().filter(|(_, it)| it.section == st.section()).map(|(i, _)| PaneRow::Item(i)).collect();
    }
    match &st.page {
        None => {
            let mut rows: Vec<PaneRow> = st.plugins.iter().map(|p| PaneRow::Plugin(p.name.clone())).collect();
            rows.push(PaneRow::InstallFrom);
            rows
        }
        Some(name) => {
            let mut rows = Vec::new();
            let Some(p) = st.plugins.iter().find(|p| &p.name == name) else { return rows };
            rows.push(PaneRow::Header(plugin_page_title(&p.name, &p.version, &p.source)));
            if !p.description.is_empty() {
                rows.push(PaneRow::Text(p.description.clone()));
            }
            rows.push(PaneRow::Blank);
            rows.push(PaneRow::Installed(p.name.clone()));
            if p.installed && p.source != "error" {
                rows.extend(items.iter().enumerate().filter(|(_, it)| it.plugin.as_deref() == Some(name)).map(|(i, _)| PaneRow::Item(i)));
            }
            rows
        }
    }
}

/// 칸 폭에 맞춰 단어 단위로 접는다. 하드 랩은 여기서만 한다.
/// 설명 상자에 보일 문장. 설정 항목은 그 설명, 플러그인 행은 플러그인 설명(페이지를 안 열어도 뭔지 보인다).
fn row_desc(app: &App, items: &[crate::settings::Item], row: &PaneRow) -> String {
    match row {
        PaneRow::Item(i) => items[*i].desc.clone(),
        PaneRow::Plugin(name) => app.settings.plugins.iter().find(|p| &p.name == name).map(|p| p.description.clone()).unwrap_or_default(),
        PaneRow::InstallFrom => "owner/repo, a git URL or a local path".to_string(),
        PaneRow::Header(_) | PaneRow::Text(_) | PaneRow::Blank | PaneRow::Installed(_) => String::new(),
    }
}

/// 설명 상자의 최대 줄 수. 폭 90이면 350자쯤. 그 이상은 `…`.
const DESC_MAX_LINES: usize = 4;

fn draw_settings_popup(f: &mut Frame, app: &App, area: Rect) {
    use crate::settings::{desc_height, desc_lines, wrap_words, Section};
    let height = (area.height * 7 / 10).max(12).min(area.height);
    let popup_area = centered_rect(80, height, area);
    clear_area(f, popup_area);
    let block = Block::default().borders(Borders::ALL).title(" Settings ");
    let inner = block.inner(popup_area);
    f.render_widget(block, popup_area);

    // [절 │ 행들 / 설명 상자] 위에, 알림·키 안내 한 줄 아래에
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(inner);
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(12), Constraint::Length(1), Constraint::Min(1)])
        .split(vertical[0]);

    // 왼쪽: 절
    let st = &app.settings;
    let left_focused = st.focus == SettingsFocus::Sections;
    let dimmed = st.query.is_some();
    let mut left = Vec::new();
    for (i, s) in Section::ALL.iter().enumerate() {
        let selected = i == st.section;
        let style = match (dimmed, selected, left_focused) {
            (true, _, _) => Style::default().fg(theme().muted),
            (false, true, true) => Style::default().fg(theme().accent).add_modifier(Modifier::BOLD),
            (false, true, false) => Style::default().fg(theme().accent),
            _ => Style::default(),
        };
        let mark = if selected { ">" } else { " " };
        left.push(Line::from(Span::styled(format!("{}{}", mark, s.label()), style)));
    }
    f.render_widget(Paragraph::new(left), columns[0]);
    let divider: Vec<Line> = (0..columns[1].height).map(|_| Line::from(Span::styled("│", Style::default().fg(theme().muted)))).collect();
    f.render_widget(Paragraph::new(divider), columns[1]);

    // 오른쪽: 행. Text는 접히므로 행 하나가 여러 줄이 될 수 있다.
    // 설명 상자는 이 목록에서 가장 긴 설명에 맞춰 잡아 커서를 옮겨도 목록이 움직이지 않는다.
    let items = app.settings_items();
    let rows = settings_pane_rows(app, &items);
    let width = columns[2].width as usize;
    let descs: Vec<String> = rows.iter().map(|r| row_desc(app, &items, r)).collect();
    let desc_h = desc_height(&descs.iter().map(String::as_str).collect::<Vec<_>>(), width, DESC_MAX_LINES);
    let right_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(desc_h as u16)])
        .split(columns[2]);
    let right = right_rows[0];
    let mut lines: Vec<(Option<usize>, Line)> = Vec::new();
    for (ri, row) in rows.iter().enumerate() {
        let is_cursor = ri == st.row && st.focus == SettingsFocus::Rows;
        match row {
            PaneRow::Header(h) => lines.push((Some(ri), Line::from(Span::styled(h.clone(), Style::default().fg(theme().heading).add_modifier(Modifier::BOLD))))),
            PaneRow::Blank => lines.push((Some(ri), Line::from(""))),
            PaneRow::Text(t) => {
                for (k, l) in wrap_words(t, width.saturating_sub(1)).into_iter().enumerate() {
                    lines.push((if k == 0 { Some(ri) } else { None }, Line::from(Span::styled(l, Style::default().fg(theme().muted)))));
                }
            }
            PaneRow::Plugin(name) => {
                let p = st.plugins.iter().find(|p| &p.name == name);
                let (status, source) = match p {
                    Some(p) => (if p.installed { "installed" } else { "not installed" }, p.source.as_str()),
                    None => ("", ""),
                };
                let mark = if is_cursor { "> " } else { "  " };
                let style = if is_cursor { Style::default().fg(theme().accent) } else { Style::default() };
                lines.push((Some(ri), Line::from(vec![
                    Span::styled(format!("{}{:<14} {:<14}", mark, name, status), style),
                    Span::styled(source.to_string(), Style::default().fg(theme().muted)),
                ])));
            }
            PaneRow::Installed(name) => {
                let installed = st.plugins.iter().find(|p| &p.name == name).map(|p| p.installed).unwrap_or(false);
                let mark = if is_cursor { "> " } else { "  " };
                let style = if is_cursor { Style::default().fg(theme().accent) } else { Style::default() };
                lines.push((Some(ri), Line::from(Span::styled(format!("{}{:<18} [{}]", mark, "Installed", if installed { "yes" } else { "no" }), style))));
            }
            PaneRow::InstallFrom => {
                let mark = if is_cursor { "> " } else { "  " };
                let style = if is_cursor { Style::default().fg(theme().accent) } else { Style::default().fg(theme().heading) };
                lines.push((Some(ri), Line::from(Span::styled(format!("{}Install from…", mark), style))));
            }
            PaneRow::Item(i) => {
                let it = &items[*i];
                let mark = if is_cursor { "> " } else { "  " };
                let val = crate::settings::value(it, &app.config);
                let style = if is_cursor { Style::default().fg(theme().accent) } else { Style::default() };
                lines.push((Some(ri), Line::from(Span::styled(format!("{}{:<18} [{}]", mark, it.label, val), style))));
            }
        }
    }
    // 커서 줄이 보이도록 창을 민다
    let cursor_line = lines.iter().position(|(r, _)| *r == Some(st.row)).unwrap_or(0);
    let visible = right.height as usize;
    let start = if cursor_line >= visible { cursor_line + 1 - visible } else { 0 };
    let shown: Vec<Line> = lines.into_iter().skip(start).take(visible).map(|(_, l)| l).collect();
    f.render_widget(Paragraph::new(shown), right);

    // 설명 상자: 커서 행의 설명만
    if st.focus == SettingsFocus::Rows {
        if let Some(d) = descs.get(st.row) {
            let text: Vec<Line> = desc_lines(d, width, desc_h).into_iter().map(|l| Line::from(Span::styled(l, Style::default().fg(theme().muted)))).collect();
            f.render_widget(Paragraph::new(text), right_rows[1]);
        }
    }

    // 아래 줄: 알림 또는 키 안내
    let footer = match (&st.notice, &st.query) {
        (Some((text, is_err)), _) => Line::from(Span::styled(text.clone(), Style::default().fg(if *is_err { theme().error } else { theme().success }))),
        (None, Some(q)) => Line::from(vec![
            Span::styled("/ ", Style::default().fg(theme().heading)),
            Span::raw(format!("{}{}", q, if st.typing { "▏" } else { "" })),
            Span::styled(if st.typing { "   Enter done  Esc clear" } else { "   j/k move  h/l change  Esc clear" }, Style::default().fg(theme().muted)),
        ]),
        (None, None) => {
            let hint = if st.page.is_some() { "j/k move  h/l change  Esc back to list" } else { "Tab switch  j/k move  h/l change  Enter edit  / search  Esc close" };
            Line::from(Span::styled(hint, Style::default().fg(theme().muted)))
        }
    };
    f.render_widget(Paragraph::new(footer), vertical[1]);
}

fn draw_settings_input(f: &mut Frame, input: &SettingsInput, area: Rect) {
    let popup_area = centered_rect(60, 5, area);
    clear_area(f, popup_area);
    let text = Paragraph::new(vec![
        Line::from(Span::styled(input.title.clone(), Style::default().fg(theme().heading))),
        Line::from(format!("> {}▏", input.buf)),
    ])
    .block(Block::default().borders(Borders::ALL).title(" Settings "));
    f.render_widget(text, popup_area);
}

// ── Event loop ───────────────────────────────────────────────────────────────

fn handle_mouse(app: &mut App, mouse: MouseEvent) {
    let col = mouse.column;
    let row = mouse.row;

    if !matches!(app.mode, Mode::Normal | Mode::ContextMenu) {
        return;
    }

    let (scroll_up, scroll_down) = if app.config.natural_scroll {
        (MouseEventKind::ScrollDown, MouseEventKind::ScrollUp)
    } else {
        (MouseEventKind::ScrollUp, MouseEventKind::ScrollDown)
    };

    match mouse.kind {
        kind if kind == scroll_up => {
            if matches!(app.mode, Mode::ContextMenu) {
                app.context_menu.index = app.context_menu.index.saturating_sub(1);
            } else {
                match app.focus {
                    Panel::Collections => app.move_col_up(),
                    Panel::Entries => app.move_entry_up(),
                    Panel::Preview => app.preview_scroll = app.preview_scroll.saturating_sub(3),
                }
            }
            return;
        }
        kind if kind == scroll_down => {
            if matches!(app.mode, Mode::ContextMenu) {
                let max = ContextMenuState::ITEMS.len().saturating_sub(1);
                app.context_menu.index = (app.context_menu.index + 1).min(max);
            } else {
                match app.focus {
                    Panel::Collections => app.move_col_down(),
                    Panel::Entries => app.move_entry_down(),
                    Panel::Preview => app.preview_scroll = app.preview_scroll.saturating_add(3).min(app.preview_max_scroll),
                }
            }
            return;
        }
        MouseEventKind::Down(MouseButton::Left) | MouseEventKind::Down(MouseButton::Right) => {}
        _ => return,
    }

    let is_right = matches!(mouse.kind, MouseEventKind::Down(MouseButton::Right));

    // Context menu is open: handle clicks inside/outside
    if matches!(app.mode, Mode::ContextMenu) {
        let items = ContextMenuState::ITEMS;
        let menu_w: u16 = 20;
        let menu_h = items.len() as u16 + 2;
        let mx = app.context_menu.x;
        let my = app.context_menu.y;
        let in_menu = col >= mx && col < mx + menu_w && row >= my && row < my + menu_h;

        if in_menu && !is_right {
            let rel = (row.saturating_sub(my + 1)) as usize;
            if rel < items.len() {
                app.context_menu.index = rel;
                execute_context_action(app, rel);
                return;
            }
        }
        app.mode = Mode::Normal;
        if !is_right {
            return;
        }
        // Right-click outside menu: close old, open new below
    }

    // Panel hit-test
    let clicked_panel = app.panel_areas.iter().enumerate().find(|(_, area)| {
        col >= area.x && col < area.x + area.width
            && row >= area.y && row < area.y + area.height
    });
    let Some((panel_idx, &area)) = clicked_panel else { return };

    let panel = match panel_idx {
        0 => Panel::Collections,
        1 => Panel::Entries,
        _ => Panel::Preview,
    };
    // 탭 줄의 자리는 클릭 당시 그려진 대로(포커스 표시 " ● "가 있었는지) 계산해야 한다
    let preview_was_focused = app.focus == Panel::Preview;
    app.focus = panel;

    // Select the clicked item first (for both left and right click)
    let inner_y = area.y + 1;
    let inner_height = area.height.saturating_sub(2);
    let in_content = row >= inner_y && row < inner_y + inner_height;

    if in_content {
        let rel_row = row.saturating_sub(inner_y) as usize;
        match panel_idx {
            0 => {
                let clicked_idx = rel_row + app.col_list_state.offset();
                if clicked_idx < app.col_count() {
                    app.col_list_state.select(Some(clicked_idx));
                    app.list_state.select(Some(0));
                    app.apply_filters();
                }
            }
            1 => {
                let entry_idx = rel_row / 3 + app.list_state.offset();
                if entry_idx < app.filtered.len() {
                    app.list_state.select(Some(entry_idx));
                    app.update_preview();
                }
            }
            _ => {}
        }
    }

    // Right-click: open context menu at cursor
    if is_right && app.selected_entry().is_some() {
        app.context_menu.x = col;
        app.context_menu.y = row;
        app.context_menu.index = 0;
        app.mode = Mode::ContextMenu;
        return;
    }

    // Left-click on preview tab bar
    if !is_right && panel_idx == 2 && row == area.y {
        let rel_x = col.saturating_sub(area.x + 1);
        let modes = app.preview_modes();
        let labels: Vec<String> = modes.iter().map(|m| app.preview_label(*m)).collect();
        let mut x = if preview_was_focused { 3 } else { 1 };
        for (i, label) in labels.iter().enumerate() {
            let tab_width = label.chars().count() as u16 + 2;
            if rel_x >= x && rel_x < x + tab_width {
                app.set_preview_mode(modes[i]);
                return;
            }
            x += tab_width + 3;
        }
    }
}

fn handle_key(app: &mut App, key: crossterm::event::KeyEvent) -> Result<bool> {
    match &app.mode {
        Mode::Normal => handle_normal(app, key),
        Mode::Search => handle_search(app, key),
        Mode::Confirm(_) => handle_confirm(app, key),
        Mode::Message(_) => { app.mode = Mode::Normal; Ok(false) }
        Mode::PluginUi(_) => handle_plugin_ui(app, key),
        Mode::Loading(_) => {
            if key.code == KeyCode::Esc {
                if let Some(run) = app.plugin_run.take() {
                    app.host.kill(&run.plugin);
                    app.mode = Mode::Message(format!("{}: cancelled", run.plugin));
                } else {
                    app.bg_result = None;
                    app.bg_install = None;
                    app.mode = Mode::Normal;
                }
            }
            Ok(false)
        }
        Mode::Help => handle_help(app, key),
        Mode::SortMenu => handle_sort_menu(app, key),
        Mode::CollectionPicker => handle_picker(app, key, false),
        Mode::TagEditor => handle_picker(app, key, true),
        Mode::ExportMenu => handle_export_menu(app, key),
        Mode::Settings => handle_settings(app, key),
        Mode::SettingsInput(_) => handle_settings_input(app, key),
        Mode::FilePicker(_) => handle_file_picker(app, key),
        Mode::FetchPreview => handle_fetch_preview(app, key),
        Mode::SearchResultPicker => handle_search_result_picker(app, key),
        Mode::ContextMenu => handle_context_menu(app, key),
        Mode::CitationStyle(_) => handle_citation_style(app, key),
    }
}

fn handle_context_menu(app: &mut App, key: crossterm::event::KeyEvent) -> Result<bool> {
    let items = app.context_menu_items();
    let max = items.len().saturating_sub(1);
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => { app.mode = Mode::Normal; }
        KeyCode::Char('j') | KeyCode::Down => {
            app.context_menu.index = (app.context_menu.index + 1).min(max);
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.context_menu.index = app.context_menu.index.saturating_sub(1);
        }
        KeyCode::Enter => {
            let idx = app.context_menu.index;
            execute_context_action(app, idx);
        }
        KeyCode::Char(c) => {
            let pressed = crate::keymap::render_key(KeyPress::new(KeyCode::Char(c), KeyModifiers::NONE));
            if let Some(idx) = items
                .iter()
                .position(|(_, a)| shortcut_hint(&app.keymap.entries, *a) == pressed)
            {
                execute_context_action(app, idx);
            }
        }
        _ => {}
    }
    Ok(false)
}

fn execute_context_action(app: &mut App, idx: usize) {
    app.mode = Mode::Normal;
    let Some((_, action)) = app.context_menu_items().get(idx).cloned() else { return };
    match action {
        Action::Plugin(id) => app.start_plugin_command(id, "menu"),
        other => { let _ = execute(app, other); }
    }
}

/// 액션 하나를 실행한다. 바디는 옛 `handle_normal`의 32갈래에서 그대로 옮겨 왔다.
/// 포커스 분기가 있던 갈래는 패널별 액션으로 갈라졌으므로 여기에는 `app.focus`로
/// 갈라지는 이동 코드가 없다.
fn execute(app: &mut App, action: Action) -> Result<Flow> {
    match action {
        Action::Quit => return Ok(Flow::Quit),
        Action::Cancel => {
            if !app.selected_keys.is_empty() {
                app.selected_keys.clear();
            } else {
                return Ok(Flow::Quit);
            }
        }
        Action::Undo => { app.undo()?; }
        Action::Redo => { app.redo()?; }
        Action::Plugin(id) => { app.start_plugin_command(id, "key"); }

        // ── 포커스 이동과 미리보기 탭 ──
        Action::FocusCollections => { app.focus = Panel::Collections; }
        Action::FocusEntries => { app.focus = Panel::Entries; }
        Action::FocusPreview => { app.focus = Panel::Preview; }
        Action::PrevTabOrFocusEntries => {
            if let Some(prev) = app.preview_prev() {
                app.set_preview_mode(prev);
            } else {
                app.focus = Panel::Entries;
            }
        }
        Action::NextTab => {
            if let Some(next) = app.preview_next(false) {
                app.set_preview_mode(next);
            }
        }
        Action::PrevTab => {
            if let Some(prev) = app.preview_prev() {
                app.set_preview_mode(prev);
            }
        }

        // ── 한 칸 이동 ──
        Action::CollectionDown => app.move_col_down(),
        Action::CollectionUp => app.move_col_up(),
        Action::EntryDown => app.move_entry_down(),
        Action::EntryUp => app.move_entry_up(),
        Action::PreviewScrollDown => {
            if let Some((extent, view, _, _)) = app.tab_extents() {
                app.tab.scroll_down(app.tab_step(), extent, view);
            } else if !matches!(app.preview_mode, PreviewMode::Plugin(_)) {
                app.preview_scroll = app.preview_scroll.saturating_add(1).min(app.preview_max_scroll);
            }
        }
        Action::PreviewScrollUp => {
            if let Some((_, view, _, _)) = app.tab_extents() {
                app.tab.scroll_up(app.tab_step(), view);
            } else if !matches!(app.preview_mode, PreviewMode::Plugin(_)) {
                app.preview_scroll = app.preview_scroll.saturating_sub(1);
            }
        }

        // ── 끝으로 ──
        Action::CollectionTop => {
            app.col_list_state.select(Some(0));
            app.list_state.select(Some(0));
            app.apply_filters();
        }
        Action::EntryTop => {
            app.list_state.select(Some(0));
            app.update_preview();
        }
        Action::PreviewTop => {
            if matches!(app.preview_mode, PreviewMode::Plugin(_)) { app.tab.first_page(); } else { app.preview_scroll = 0; }
        }
        Action::CollectionBottom => {
            let last = app.col_count().saturating_sub(1);
            app.col_list_state.select(Some(last));
            app.list_state.select(Some(0));
            app.apply_filters();
        }
        Action::EntryBottom => {
            if !app.filtered.is_empty() {
                app.list_state.select(Some(app.filtered.len() - 1));
                app.update_preview();
            }
        }
        Action::PreviewBottom => {
            if matches!(app.preview_mode, PreviewMode::Plugin(_)) { app.tab.last_page(); } else { app.preview_scroll = app.preview_max_scroll; }
        }

        // ── 화면 상대 이동 (엔트리 패널) ──
        Action::EntryScreenTop => {
            // Jump to first visible item (approximate: just go to current - half_page)
            let visible_height = 10; // approximate
            let cur = app.list_state.selected().unwrap_or(0);
            let top = cur.saturating_sub(visible_height);
            app.list_state.select(Some(top));
            app.update_preview();
        }
        Action::EntryScreenMiddle => {
            if !app.filtered.is_empty() {
                app.list_state.select(Some(app.filtered.len() / 2));
                app.update_preview();
            }
        }
        Action::EntryScreenBottom => {
            if !app.filtered.is_empty() {
                app.list_state.select(Some(app.filtered.len() - 1));
                app.update_preview();
            }
        }

        // ── 반 페이지 ──
        Action::EntryHalfPageDown => { for _ in 0..10 { app.move_entry_down(); } }
        Action::EntryHalfPageUp => { for _ in 0..10 { app.move_entry_up(); } }
        Action::PreviewHalfPageDown => {
            if let Some((extent, view, _, _)) = app.tab_extents() {
                app.tab.scroll_down(view / 2, extent, view);
            } else if !matches!(app.preview_mode, PreviewMode::Plugin(_)) {
                app.preview_scroll = app.preview_scroll.saturating_add(10).min(app.preview_max_scroll);
            }
        }
        Action::PreviewHalfPageUp => {
            if let Some((_, view, _, _)) = app.tab_extents() {
                app.tab.scroll_up(view / 2, view);
            } else if !matches!(app.preview_mode, PreviewMode::Plugin(_)) {
                app.preview_scroll = app.preview_scroll.saturating_sub(10);
            }
        }
        Action::CollectionHalfPageDown => { for _ in 0..5 { app.move_col_down(); } }
        Action::CollectionHalfPageUp => { for _ in 0..5 { app.move_col_up(); } }

        // ── 선택 ──
        Action::ToggleSelect => {
            if let Some(entry) = app.selected_entry() {
                let key = entry.bibtex_key.clone();
                if app.selected_keys.contains(&key) {
                    app.selected_keys.remove(&key);
                } else {
                    app.selected_keys.insert(key);
                }
            }
        }
        Action::SelectAll => {
            let visible_keys: Vec<String> = app.filtered.iter()
                .map(|&idx| app.entries[idx].bibtex_key.clone()).collect();
            let all_selected = visible_keys.iter().all(|k| app.selected_keys.contains(k));
            if all_selected {
                for k in &visible_keys { app.selected_keys.remove(k); }
            } else {
                for k in visible_keys { app.selected_keys.insert(k); }
            }
        }

        Action::NextPreviewTab => {
            if let Some(m) = app.preview_next(true) {
                app.set_preview_mode(m);
            }
        }

        // 검색은 포커스에 따라 대상이 다르다. 이동이 아니라 한 액션의 문서화된 동작이다.
        Action::Search => {
            if app.focus == Panel::Collections {
                app.col_search_query.clear();
            }
            app.mode = Mode::Search;
        }

        Action::CopyCitekey => {
            if let Some(entry) = app.selected_entry() {
                let bkey = entry.bibtex_key.clone();
                if let Ok(mut ctx) = arboard::Clipboard::new() { let _ = ctx.set_text(&bkey); }
                app.mode = Mode::Message(format!("Copied: {}", bkey));
            }
        }

        Action::CopyCitation => {
            if app.selected_entry().is_some() {
                app.mode = Mode::CitationStyle(0);
            }
        }

        Action::OpenPdf => {
            if let Some(entry) = app.selected_entry() {
                let entry = entry.clone();
                if entry.file_path.is_some() {
                    let full_path = app.config.bibox_dir.join(entry.file_path.as_ref().unwrap());
                    if full_path.exists() {
                        app.open_pdf(&entry);
                    } else if entry.doi.is_some() || entry.url.is_some() {
                        let bkey = entry.bibtex_key.clone();
                        app.mode = Mode::Confirm(ConfirmAction::FetchPdf(bkey));
                    } else {
                        app.mode = Mode::Message("PDF file missing from disk.".into());
                    }
                } else if entry.doi.is_some() || entry.url.is_some() {
                    let bkey = entry.bibtex_key.clone();
                    app.mode = Mode::Confirm(ConfirmAction::FetchPdf(bkey));
                } else { app.mode = Mode::Message("No PDF attached.".into()); }
            }
        }

        Action::OpenWeb => {
            if let Some(entry) = app.selected_entry() {
                let url = if let Some(ref doi) = entry.doi {
                    Some(format!("https://doi.org/{}", doi))
                } else { entry.url.clone() };
                if let Some(url) = url {
                    #[cfg(target_os = "macos")]
                    let _ = std::process::Command::new("open").arg(&url).spawn();
                    #[cfg(not(target_os = "macos"))]
                    let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
                } else {
                    app.mode = Mode::Message("No DOI or URL for this entry.".into());
                }
            }
        }

        Action::FetchMetadata => {
            if let Some(entry) = app.selected_entry() {
                let key = entry.bibtex_key.clone();
                let doi = entry.doi.clone();
                let title = entry.title.clone();
                if let Some(doi) = doi {
                    // Fetch by DOI
                    let (tx, rx) = std::sync::mpsc::channel();
                    let key_clone = key.clone();
                    std::thread::spawn(move || {
                        let rt = tokio::runtime::Runtime::new().unwrap();
                        let result = rt.block_on(crate::crossref::fetch_metadata(&doi));
                        let _ = tx.send(result.map(|m| (key_clone, m)));
                    });
                    app.bg_meta_result = Some(rx);
                    app.spinner_tick = 0;
                    app.mode = Mode::Loading("Fetching metadata from Crossref...".into());
                } else if let Some(title) = title {
                    // No DOI - search by title
                    app.mode = Mode::Confirm(ConfirmAction::FetchMetaByTitle(key, title));
                } else {
                    app.mode = Mode::Message("No DOI or title to search.".into());
                }
            }
        }

        Action::ExportMenu => {
            let mut scope_options = vec![];
            if !app.selected_keys.is_empty() {
                scope_options.push((ExportScope::Selected, format!("{} selected entries", app.selected_keys.len())));
            }
            if let Some(col) = app.current_collection() {
                // 현재 컬렉션과 그 상위 전부. `gym/method`에서 `gym`째로 내보낼 수 있게
                for name in collection_and_ancestors(col) {
                    let count = keys_in_collection(&app.entries, &name).len();
                    let label = format!("{} collection ({})", name, count);
                    scope_options.push((ExportScope::Collection(name), label));
                }
            }
            scope_options.push((ExportScope::All, format!("All entries ({})", app.entries.len())));
            app.export_state = Some(ExportState {
                scope_options,
                scope_idx: 0,
                format_idx: 0,
                include_pdf: false,
                section: 0,
            });
            app.mode = Mode::ExportMenu;
        }

        Action::Delete => {
            if let Some(entry) = app.selected_entry() {
                let bkey = entry.bibtex_key.clone();
                app.mode = Mode::Confirm(ConfirmAction::Delete(bkey));
            }
        }

        Action::Help => {
            app.help_query.clear();
            app.help_filtering = false;
            app.help_scroll = 0;
            app.mode = Mode::Help;
        }

        Action::EditNote => {
            if app.selected_entry().is_some() {
                let quit = open_note_editor(app)?;
                return Ok(if quit { Flow::Quit } else { Flow::Continue });
            } else {
                app.mode = Mode::Message("No entry selected.".into());
            }
        }

        Action::SortMenu => {
            app.prev_sort_by = app.sort_by;
            app.prev_sort_ascending = app.sort_ascending;
            app.sort_menu_index = SortCriterion::all().iter().position(|c| *c == app.sort_by).unwrap_or(0);
            app.mode = Mode::SortMenu;
        }

        Action::Collections => {
            if !app.selected_keys.is_empty() {
                app.open_collection_picker_multi();
            } else if app.selected_entry().is_some() {
                app.open_collection_picker();
            } else {
                app.mode = Mode::Message("No entry selected.".into());
            }
        }

        Action::Tags => {
            if app.selected_entry().is_some() { app.open_tag_editor(); }
            else { app.mode = Mode::Message("No entry selected.".into()); }
        }

        Action::AttachPdf => {
            if let Some(entry) = app.selected_entry() {
                let key = entry.bibtex_key.clone();
                let start = dirs::download_dir()
                    .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from(".")));
                app.file_picker_state = Some(
                    ratatree::FilePickerState::builder()
                        .start_dir(start)
                        .mode(ratatree::PickerMode::FilesOnly)
                        .build()
                );
                app.mode = Mode::FilePicker(FilePickerContext::AttachPdf(key));
            } else {
                app.mode = Mode::Message("No entry selected.".into());
            }
        }

        Action::Settings => {
            app.open_settings(crate::settings::Section::General);
        }

        Action::Plugins => {
            app.open_settings(crate::settings::Section::Plugins);
        }

        // ── 플러그인 미리보기 탭 ──
        Action::TabNextPage => { if matches!(app.preview_mode, PreviewMode::Plugin(_)) { app.tab.next_page(); } }
        Action::TabPrevPage => { if matches!(app.preview_mode, PreviewMode::Plugin(_)) { app.tab.prev_page(); } }
        Action::TabZoomIn => {
            if app.tab_is_image() {
                let m = app.tab_max_zoom();
                app.tab.zoom(crate::preview_tabs::ZOOM_STEP as i32, m);
            }
        }
        Action::TabZoomOut => {
            if app.tab_is_image() {
                let m = app.tab_max_zoom();
                app.tab.zoom(-(crate::preview_tabs::ZOOM_STEP as i32), m);
            }
        }
        Action::TabZoomReset => { if app.tab_is_image() { app.tab.zoom_reset(); } }
        Action::TabPanLeft => {
            if let (true, Some((_, _, w, vw))) = (app.tab_is_image(), app.tab_extents()) {
                let step = 8 * app.tab_cell().0 as i32;
                app.tab.pan(-step, w, vw);
            }
        }
        Action::TabPanRight => {
            if let (true, Some((_, _, w, vw))) = (app.tab_is_image(), app.tab_extents()) {
                let step = 8 * app.tab_cell().0 as i32;
                app.tab.pan(step, w, vw);
            }
        }
        Action::Noop => {}
    }
    Ok(Flow::Continue)
}

fn layer_for(focus: Panel) -> LayerId {
    match focus {
        Panel::Collections => LayerId::Collections,
        Panel::Entries => LayerId::Entries,
        Panel::Preview => LayerId::Preview,
    }
}

fn handle_normal(app: &mut App, key: crossterm::event::KeyEvent) -> Result<bool> {
    // ① 시퀀스 해석. 숫자도 보통 키다(2026-09-15에 vim식 숫자 접두사를 없애고 1/2/3을 패널 이동에 줌)
    let press = KeyPress::new(key.code, key.modifiers);
    let resolution = resolve(app.keymap.layer(layer_for(app.focus)), &app.pending, press);

    match resolution {
        Resolution::Pending => {
            app.pending.push(press);
        }
        Resolution::Unbound => {
            app.pending.clear();
        }
        Resolution::Run(actions) => {
            app.pending.clear();

            // ② 실행. 배열은 순서대로 돌고, 종료나 모드 전환에서 중단한다.
            for action in actions {
                if execute(app, action)? == Flow::Quit {
                    return Ok(true);
                }
                if !matches!(app.mode, Mode::Normal) {
                    break;
                }
            }
            // 포커스가 바뀌면 레이어가 바뀌므로 대기 중 접두사의 의미가 사라진다.
            app.pending.clear();
        }
    }
    Ok(false)
}

fn handle_search(app: &mut App, key: crossterm::event::KeyEvent) -> Result<bool> {
    if app.focus == Panel::Collections {
        // Collection search
        match key.code {
            KeyCode::Esc => { app.col_search_query.clear(); app.mode = Mode::Normal; }
            KeyCode::Backspace => { app.col_search_query.pop(); filter_collections(app); }
            KeyCode::Char(c) => { app.col_search_query.push(c); filter_collections(app); }
            KeyCode::Enter => { app.mode = Mode::Normal; }
            _ => {}
        }
    } else {
        // Entry search
        match key.code {
            KeyCode::Esc => { app.search_query.clear(); app.apply_filters(); app.mode = Mode::Normal; }
            KeyCode::Backspace => { app.search_query.pop(); app.apply_filters(); }
            KeyCode::Char(c) => { app.search_query.push(c); app.apply_filters(); }
            KeyCode::Enter => { app.mode = Mode::Normal; }
            _ => {}
        }
    }
    Ok(false)
}

fn filter_collections(app: &mut App) {
    let query = app.col_search_query.to_lowercase();
    if query.is_empty() {
        // Reset to first collection
        app.col_list_state.select(Some(0));
        return;
    }
    // Find first matching collection (including "All")
    let col_count = app.col_count();
    for i in 0..col_count {
        let name = if i == 0 { "all" } else { match app.collections.get(i - 1) { Some(n) => n.as_str(), None => continue } };
        if name.to_lowercase().contains(&query) {
            app.col_list_state.select(Some(i));
            app.apply_filters();
            return;
        }
    }
}

fn handle_confirm(app: &mut App, key: crossterm::event::KeyEvent) -> Result<bool> {
    match key.code {
        KeyCode::Char('y') => {
            let action = std::mem::replace(&mut app.mode, Mode::Normal);
            match action {
                Mode::Confirm(ConfirmAction::Delete(_)) => {
                    app.delete_selected()?;
                    app.mode = Mode::Message("Entry deleted.".to_string());
                }
                Mode::Confirm(ConfirmAction::RevealExport(path, _)) => {
                    reveal_in_file_manager(&path);
                }
                Mode::Confirm(ConfirmAction::RemovePlugin(name)) => {
                    // 페이지를 먼저 닫아야 reload 뒤 커서가 목록 기준으로 놓인다
                    app.settings.page = None;
                    match crate::plugin::cli::remove_plugin_files(&crate::plugin::plugins_dir(), &name) {
                        Ok(()) => {
                            app.reload_plugins();
                            app.settings.notice = Some((app.config.msgs.plugin_removed(&name), false));
                        }
                        Err(e) => app.settings.notice = Some((e.to_string(), true)),
                    }
                    app.settings_fix_cursor();
                    app.mode = Mode::Settings;
                }
                Mode::Confirm(ConfirmAction::InstallStaged(staged)) => {
                    match crate::plugin::commit_staged(staged, &app.config.msgs) {
                        Ok((name, dest)) => {
                            app.reload_plugins();
                            app.settings.notice = Some((app.config.msgs.plugin_installed(&name, &dest.display().to_string()), false));
                        }
                        Err(e) => app.settings.notice = Some((app.config.msgs.plugin_install_failed(&e.to_string()), true)),
                    }
                    app.mode = Mode::Settings;
                }
                Mode::Confirm(ConfirmAction::FetchPdf(key)) => {
                    let entry = app.entries.iter().find(|e| e.bibtex_key == key).cloned();
                    if let Some(entry) = entry {
                        let bibox_dir = app.config.bibox_dir.clone();
                        let (tx, rx) = std::sync::mpsc::channel();
                        let key_clone = key.clone();
                        std::thread::spawn(move || {
                            let result = run_fetch_pdf(&entry, &bibox_dir);
                            let _ = tx.send(result.map(|(filename, full_path)| BgTaskResult {
                                key: key_clone, file_path: filename, full_path,
                            }));
                        });
                        app.bg_result = Some(rx);
                        app.bg_fetch_key = Some(key.clone());
                        app.spinner_tick = 0;
                        app.mode = Mode::Loading("Fetching PDF...".into());
                    }
                }
                Mode::Confirm(ConfirmAction::FetchMetaByTitle(key, title)) => {
                    let (tx, rx) = std::sync::mpsc::channel();
                    let key_clone = key.clone();
                    std::thread::spawn(move || {
                        let rt = tokio::runtime::Runtime::new().unwrap();
                        let result = rt.block_on(crate::crossref::search_by_title(&title, 5));
                        let _ = tx.send(result.map(|r| (key_clone, r)));
                    });
                    app.bg_search_result = Some(rx);
                    app.spinner_tick = 0;
                    app.mode = Mode::Loading("Searching Crossref by title...".into());
                }
                Mode::Confirm(ConfirmAction::OpenBrowser(_key, url)) => {
                    #[cfg(target_os = "macos")]
                    let _ = std::process::Command::new("open").arg(&url).spawn();
                    #[cfg(not(target_os = "macos"))]
                    let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
                    app.mode = Mode::Message(
                        "Opened in browser. Download the PDF, then use:\nbibox edit <key> --attach-pdf ~/Downloads/<file>.pdf".to_string()
                    );
                }
                _ => {}
            }
        }
        KeyCode::Char('n') | KeyCode::Esc => {
            let taken = std::mem::replace(&mut app.mode, Mode::Normal);
            app.mode = match taken {
                Mode::Confirm(ConfirmAction::InstallStaged(staged)) => {
                    crate::plugin::discard_staged(staged);
                    Mode::Settings
                }
                Mode::Confirm(ConfirmAction::RemovePlugin(_)) => Mode::Settings,
                _ => Mode::Normal,
            };
        }
        _ => {}
    }
    Ok(false)
}

fn handle_help(app: &mut App, key: crossterm::event::KeyEvent) -> Result<bool> {
    let all = help_rows(app.keymap.layer(layer_for(app.focus)), app.host.commands());
    let total_rows = filter_rows(&all, &app.help_query).len();
    let max_scroll = total_rows.saturating_sub(1);

    if app.help_filtering {
        match key.code {
            KeyCode::Esc => {
                app.help_query.clear();
                app.help_filtering = false;
                app.help_scroll = 0;
            }
            KeyCode::Enter => { app.help_filtering = false; }
            KeyCode::Backspace => { app.help_query.pop(); app.help_scroll = 0; }
            KeyCode::Char(c) => { app.help_query.push(c); app.help_scroll = 0; }
            _ => {}
        }
        return Ok(false);
    }

    match key.code {
        KeyCode::Char('/') => { app.help_filtering = true; }
        KeyCode::Char('j') | KeyCode::Down => {
            app.help_scroll = (app.help_scroll + 1).min(max_scroll);
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.help_scroll = app.help_scroll.saturating_sub(1);
        }
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.help_scroll = (app.help_scroll + 10).min(max_scroll);
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.help_scroll = app.help_scroll.saturating_sub(10);
        }
        KeyCode::Char('g') | KeyCode::Home => { app.help_scroll = 0; }
        KeyCode::Char('G') | KeyCode::End => { app.help_scroll = max_scroll; }
        // Esc clears an active filter first, and only closes once there is none.
        KeyCode::Esc if !app.help_query.is_empty() => {
            app.help_query.clear();
            app.help_scroll = 0;
        }
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?')
        | KeyCode::Char('`') | KeyCode::Char('~') | KeyCode::F(1) => {
            app.help_query.clear();
            app.help_filtering = false;
            app.help_scroll = 0;
            app.mode = Mode::Normal;
        }
        _ => {}
    }
    Ok(false)
}

fn handle_sort_menu(app: &mut App, key: crossterm::event::KeyEvent) -> Result<bool> {
    let criteria = SortCriterion::all();
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => { if app.sort_menu_index > 0 { app.sort_menu_index -= 1; } }
        KeyCode::Down | KeyCode::Char('j') => { if app.sort_menu_index < criteria.len() - 1 { app.sort_menu_index += 1; } }
        KeyCode::Char(' ') => {
            if let Some(&selected) = criteria.get(app.sort_menu_index) {
                if selected == app.sort_by { app.sort_ascending = !app.sort_ascending; }
                else { app.sort_by = selected; }
            }
        }
        KeyCode::Enter => {
            if let Some(&new_criterion) = criteria.get(app.sort_menu_index) {
                if new_criterion != app.sort_by { app.sort_ascending = new_criterion.default_ascending(); }
                app.sort_by = new_criterion;
            }
            app.apply_sort();
            app.mode = Mode::Normal;
        }
        KeyCode::Esc => {
            app.sort_by = app.prev_sort_by;
            app.sort_ascending = app.prev_sort_ascending;
            app.mode = Mode::Normal;
        }
        _ => {}
    }
    Ok(false)
}

fn handle_picker(app: &mut App, key: crossterm::event::KeyEvent, is_tags: bool) -> Result<bool> {
    let in_input = app.picker.as_ref().map(|p| p.in_input_mode()).unwrap_or(false);
    if in_input {
        match key.code {
            KeyCode::Enter => { if let Some(ref mut p) = app.picker { p.confirm_input(); } }
            KeyCode::Esc => { if let Some(ref mut p) = app.picker { p.cancel_input(); } }
            KeyCode::Backspace => { if let Some(ref mut p) = app.picker { p.backspace(); } }
            KeyCode::Char(c) => { if let Some(ref mut p) = app.picker { p.apply_char(c); } }
            _ => {}
        }
    } else {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => { if let Some(ref mut p) = app.picker { p.move_up(); } }
            KeyCode::Down | KeyCode::Char('j') => { if let Some(ref mut p) = app.picker { p.move_down(); } }
            KeyCode::Char(' ') => { if let Some(ref mut p) = app.picker { p.toggle(); } }
            KeyCode::Enter => {
                if is_tags {
                    app.apply_picker_tags()?;
                } else if !app.selected_keys.is_empty() {
                    app.apply_picker_collections_multi()?;
                    return Ok(false); // mode already set in the function
                } else {
                    app.apply_picker_collections()?;
                }
                app.mode = Mode::Normal;
            }
            KeyCode::Esc => { app.picker = None; app.mode = Mode::Normal; }
            _ => {}
        }
    }
    Ok(false)
}

fn handle_export_menu(app: &mut App, key: crossterm::event::KeyEvent) -> Result<bool> {
    let formats = [ExportFormat::BibTeX, ExportFormat::Yaml, ExportFormat::Ris];
    if let Some(ref mut es) = app.export_state {
        match key.code {
            KeyCode::Tab => {
                es.section = (es.section + 1) % 3;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                match es.section {
                    0 => { if es.scope_idx > 0 { es.scope_idx -= 1; } }
                    1 => { if es.format_idx > 0 { es.format_idx -= 1; } }
                    _ => {}
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                match es.section {
                    0 => { if es.scope_idx + 1 < es.scope_options.len() { es.scope_idx += 1; } }
                    1 => { if es.format_idx < formats.len() - 1 { es.format_idx += 1; } }
                    _ => {}
                }
            }
            KeyCode::Char(' ') => {
                if es.section == 2 { es.include_pdf = !es.include_pdf; }
            }
            KeyCode::Enter => {
                let scope = es.scope_options.get(es.scope_idx).map(|o| o.0.clone()).unwrap_or(ExportScope::All);
                let format = formats.get(es.format_idx).copied().unwrap_or(ExportFormat::BibTeX);
                let include_pdf = es.include_pdf;
                // 범위별 항목과 파일 이름의 밑동. CLI와 같은 이름 규칙(selected / <컬렉션> / references)
                let (keys, base): (Vec<String>, String) = match &scope {
                    ExportScope::Selected => (app.selected_keys.iter().cloned().collect(), "selected".to_string()),
                    ExportScope::Collection(name) => (keys_in_collection(&app.entries, name), name.replace('/', "_")),
                    ExportScope::All => (app.entries.iter().map(|e| e.bibtex_key.clone()).collect(), "references".to_string()),
                };
                let entries: Vec<&Entry> = keys.iter().filter_map(|k| app.entries.iter().find(|e| &e.bibtex_key == k)).collect();

                app.export_state = None;
                app.mode = Mode::Normal;

                // .bib은 Bib export dir, 나머지는 Export dir. 터미널에 아무것도 찍지 않는다(raw mode).
                let dir = if format == ExportFormat::BibTeX { &app.config.bib_export_dir } else { &app.config.export_dir };
                let dir = crate::config::expand_tilde(dir);
                let result = crate::commands::export_to_file(&entries, format.ext(), &dir, &base).and_then(|path| {
                    if include_pdf {
                        crate::commands::copy_pdfs_to_dir(&entries, &dir, &base, &app.config)?;
                    }
                    Ok(path)
                });
                match result {
                    Ok(path) => {
                        let n = entries.len();
                        app.mode = Mode::Confirm(ConfirmAction::RevealExport(path, n));
                    }
                    Err(e) => { app.mode = Mode::Message(format!("Export failed: {}", e)); }
                }
            }
            KeyCode::Esc => {
                app.export_state = None;
                app.mode = Mode::Normal;
            }
            _ => {}
        }
    }
    Ok(false)
}

fn handle_settings(app: &mut App, key: crossterm::event::KeyEvent) -> Result<bool> {
    use crate::settings::{Kind, Section};
    app.settings.notice = None;
    let items = app.settings_items();
    if app.settings.typing {
        match key.code {
            KeyCode::Esc => settings_end_search(app),
            KeyCode::Enter => { app.settings.typing = false; }
            KeyCode::Backspace => {
                if let Some(q) = app.settings.query.as_mut() { q.pop(); }
                let rows = settings_pane_rows(app, &items);
                app.settings.row = first_selectable(&rows);
            }
            KeyCode::Char(c) => {
                if let Some(q) = app.settings.query.as_mut() { q.push(c); }
                let rows = settings_pane_rows(app, &items);
                app.settings.row = first_selectable(&rows);
            }
            _ => {}
        }
        return Ok(false);
    }
    let rows = settings_pane_rows(app, &items);
    let focus = app.settings.focus;

    match (focus, key.code) {
        (_, KeyCode::Char('/')) => {
            if app.settings.query.is_none() {
                app.settings.saved = Some((app.settings.focus, app.settings.section, app.settings.row, app.settings.page.clone()));
            }
            app.settings.query = Some(String::new());
            app.settings.typing = true;
            app.settings.focus = SettingsFocus::Rows;
            let rows = settings_pane_rows(app, &items);
            app.settings.row = first_selectable(&rows);
        }
        (_, KeyCode::Esc) | (_, KeyCode::Char(',')) => {
            if app.settings.query.is_some() {
                settings_end_search(app);
            } else if let Some(name) = app.settings.page.take() {
                let rows = settings_pane_rows(app, &items);
                app.settings.row = rows.iter().position(|r| *r == PaneRow::Plugin(name.clone())).unwrap_or(0);
            } else {
                app.mode = Mode::Normal;
            }
        }
        (_, KeyCode::Tab) if app.settings.query.is_none() => {
            app.settings.focus = if focus == SettingsFocus::Sections { SettingsFocus::Rows } else { SettingsFocus::Sections };
            if app.settings.focus == SettingsFocus::Rows && !rows.get(app.settings.row).is_some_and(selectable) {
                app.settings.row = first_selectable(&rows);
            }
        }
        (SettingsFocus::Sections, KeyCode::Up) | (SettingsFocus::Sections, KeyCode::Char('k')) => {
            app.settings.page = None;
            app.settings.section = app.settings.section.saturating_sub(1);
            let rows = settings_pane_rows(app, &items);
            app.settings.row = first_selectable(&rows);
        }
        (SettingsFocus::Sections, KeyCode::Down) | (SettingsFocus::Sections, KeyCode::Char('j')) => {
            app.settings.page = None;
            app.settings.section = (app.settings.section + 1).min(Section::ALL.len() - 1);
            let rows = settings_pane_rows(app, &items);
            app.settings.row = first_selectable(&rows);
        }
        (SettingsFocus::Sections, KeyCode::Enter) | (SettingsFocus::Sections, KeyCode::Right) | (SettingsFocus::Sections, KeyCode::Char('l')) => {
            app.settings.focus = SettingsFocus::Rows;
            app.settings.row = first_selectable(&rows);
        }
        (SettingsFocus::Rows, KeyCode::Up) | (SettingsFocus::Rows, KeyCode::Char('k')) => {
            app.settings.row = move_cursor(&rows, app.settings.row, -1);
        }
        (SettingsFocus::Rows, KeyCode::Down) | (SettingsFocus::Rows, KeyCode::Char('j')) => {
            app.settings.row = move_cursor(&rows, app.settings.row, 1);
        }
        (SettingsFocus::Rows, KeyCode::Left) | (SettingsFocus::Rows, KeyCode::Char('h')) => settings_step(app, &items, &rows, -1),
        (SettingsFocus::Rows, KeyCode::Right) | (SettingsFocus::Rows, KeyCode::Char('l')) => settings_step(app, &items, &rows, 1),
        (SettingsFocus::Rows, KeyCode::Enter) => match rows.get(app.settings.row).cloned() {
            Some(PaneRow::Plugin(name)) => {
                // 검색 결과에서 왔으면 검색 전 절이 아니라 Plugins 절의 페이지다
                settings_end_search(app);
                app.settings.section = Section::ALL.iter().position(|s| *s == Section::Plugins).unwrap_or(0);
                app.settings.focus = SettingsFocus::Rows;
                app.settings.page = Some(name);
                let rows = settings_pane_rows(app, &items);
                app.settings.row = first_selectable(&rows);
            }
            Some(PaneRow::Installed(_)) => settings_step(app, &items, &rows, 1),
            Some(PaneRow::InstallFrom) => {
                app.mode = Mode::SettingsInput(SettingsInput {
                    title: app.config.msgs.plugin_install_from_title().to_string(),
                    buf: String::new(),
                    target: InputTarget::InstallSource,
                });
            }
            Some(PaneRow::Item(i)) => {
                let it = &items[i];
                match &it.kind {
                    Kind::Bool { .. } | Kind::Choice(_) => {
                        if crate::settings::step(it, &mut app.config, 1) {
                            app.save_settings();
                        }
                    }
                    Kind::Int | Kind::Str => {
                        app.mode = Mode::SettingsInput(SettingsInput {
                            title: it.label.clone(),
                            buf: crate::settings::value(it, &app.config),
                            target: InputTarget::Item(it.id.clone()),
                        });
                    }
                    Kind::Path { .. } => {
                        let current = crate::settings::value(it, &app.config);
                        let start = if current.starts_with('(') {
                            app.config.home.as_ref().map(|h| crate::config::expand_tilde(h)).unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from(".")))
                        } else {
                            crate::config::expand_tilde(std::path::Path::new(&current))
                        };
                        app.file_picker_state = Some(
                            ratatree::FilePickerState::builder()
                                .start_dir(start)
                                .mode(ratatree::PickerMode::DirsOnly)
                                .build(),
                        );
                        app.mode = Mode::FilePicker(FilePickerContext::Setting(it.id.clone()));
                    }
                }
            }
            _ => {}
        },
        _ => {}
    }
    Ok(false)
}

/// 검색을 끝내고 검색 전 위치로. 검색어가 비어 있어도 같다.
fn settings_end_search(app: &mut App) {
    app.settings.query = None;
    app.settings.typing = false;
    if let Some((focus, section, row, page)) = app.settings.saved.take() {
        app.settings.focus = focus;
        app.settings.section = section;
        app.settings.row = row;
        app.settings.page = page;
    }
}

/// h/l/Enter가 값을 바꾸는 자리. Installed 행은 내장이면 바로, 외부면 확인 팝업.
fn settings_step(app: &mut App, items: &[crate::settings::Item], rows: &[PaneRow], delta: i32) {
    match rows.get(app.settings.row).cloned() {
        Some(PaneRow::Item(i)) => {
            if crate::settings::step(&items[i], &mut app.config, delta) {
                app.save_settings();
                if items[i].id == "appearance.images" {
                    app.settings.notice = Some((app.config.msgs.takes_effect_next_start().to_string(), false));
                }
            }
        }
        Some(PaneRow::Installed(name)) => {
            let Some(p) = app.settings.plugins.iter().find(|p| p.name == name).cloned() else { return };
            let dir = crate::plugin::plugins_dir();
            match (p.builtin, p.installed) {
                (true, true) => match crate::plugin::cli::remove_plugin_files(&dir, &name) {
                    Ok(()) => {
                        app.reload_plugins();
                        app.settings.notice = Some((app.config.msgs.plugin_removed(&name), false));
                    }
                    Err(e) => app.settings.notice = Some((e.to_string(), true)),
                },
                (true, false) => match crate::plugin::write_stub(&dir, &name) {
                    Ok(dest) => {
                        app.reload_plugins();
                        app.settings.notice = Some((app.config.msgs.plugin_installed(&name, &dest.display().to_string()), false));
                    }
                    Err(e) => app.settings.notice = Some((e.to_string(), true)),
                },
                (false, true) => app.mode = Mode::Confirm(ConfirmAction::RemovePlugin(name)),
                (false, false) => {}
            }
        }
        _ => {}
    }
}

fn handle_settings_input(app: &mut App, key: crossterm::event::KeyEvent) -> Result<bool> {
    let done: Option<Option<String>> = match &mut app.mode {
        Mode::SettingsInput(input) => match key.code {
            KeyCode::Char(c) => { input.buf.push(c); None }
            KeyCode::Backspace => { input.buf.pop(); None }
            KeyCode::Enter => Some(Some(input.buf.clone())),
            KeyCode::Esc => Some(None),
            _ => None,
        },
        _ => None,
    };
    let Some(result) = done else { return Ok(false) };
    let Mode::SettingsInput(input) = std::mem::replace(&mut app.mode, Mode::Settings) else { return Ok(false) };
    if let Some(text) = result {
        match input.target {
            InputTarget::Item(id) => {
                let items = app.settings_items();
                if let Some(it) = items.iter().find(|i| i.id == id) {
                    match crate::settings::set(it, &mut app.config, &text) {
                        Ok(()) => {
                            app.save_settings();
                            if it.id == "appearance.images" {
                                app.settings.notice = Some((app.config.msgs.takes_effect_next_start().to_string(), false));
                            }
                        }
                        Err(e) => app.settings.notice = Some((e, true)),
                    }
                }
            }
            InputTarget::InstallSource => {
                let text = text.trim().to_string();
                if !text.is_empty() {
                    app.start_install(text);
                }
            }
        }
    }
    Ok(false)
}

fn handle_file_picker(app: &mut App, key: crossterm::event::KeyEvent) -> Result<bool> {
    if let Some(picker_state) = &mut app.file_picker_state {
        picker_state.handle_event(crossterm::event::Event::Key(key));
    }

    let result = app.file_picker_state.as_ref().map(|s| s.result());
    match result {
        Some(ratatree::PickerResult::Selected(paths)) => {
            let path = paths.into_iter().next();
            let mode = std::mem::replace(&mut app.mode, Mode::Normal);
            app.file_picker_state = None;
            if let (Some(path), Mode::FilePicker(ctx)) = (path, mode) {
                match ctx {
                    FilePickerContext::AttachPdf(key) => {
                        let db_path = crate::config::resolve_db_path(&app.config);
                        let bibox_dir = app.config.bibox_dir.clone();
                        match (|| -> anyhow::Result<String> {
                            let mut db = load_db(&db_path)?;
                            let entry = find_by_key_mut(&mut db, &key)
                                .ok_or_else(|| anyhow::anyhow!("Entry not found"))?;
                            let filename = format!("{}.pdf", entry.bibtex_key);
                            std::fs::create_dir_all(&bibox_dir)?;
                            let dest = bibox_dir.join(&filename);
                            std::fs::copy(&path, &dest)?;
                            entry.file_path = Some(filename.clone());
                            save_db(&db, &db_path)?;
                            Ok(dest.to_string_lossy().to_string())
                        })() {
                            Ok(dest) => {
                                if let Some(e) = app.entries.iter_mut().find(|e| e.bibtex_key == key) {
                                    e.file_path = Some(format!("{}.pdf", key));
                                }
                                if let Some(e) = app.entries.iter().find(|e| e.bibtex_key == key).cloned() {
                                    app.fire_after_write(crate::events::WriteReason::Edit, vec![e]);
                                }
                                app.mode = Mode::Message(format!("PDF attached: {}", dest));
                            }
                            Err(e) => { app.mode = Mode::Message(format!("Attach failed: {}", e)); }
                        }
                    }
                    FilePickerContext::Setting(id) => {
                        let items = app.settings_items();
                        if let Some(it) = items.iter().find(|i| i.id == id) {
                            crate::settings::set_path(it, &mut app.config, path);
                            app.save_settings();
                        }
                        app.mode = Mode::Settings;
                    }
                }
            }
        }
        Some(ratatree::PickerResult::Cancelled) => {
            let back_to_settings = matches!(app.mode, Mode::FilePicker(FilePickerContext::Setting(_)));
            app.file_picker_state = None;
            app.mode = if back_to_settings { Mode::Settings } else { Mode::Normal };
        }
        _ => {}
    }
    Ok(false)
}

fn open_note_editor(app: &mut App) -> Result<bool> {
    let entry = match app.selected_entry() { Some(e) => e.clone(), None => return Ok(false) };
    let notes_dir = &app.config.notes_dir;
    std::fs::create_dir_all(notes_dir)?;
    let note_path = notes_dir.join(format!("{}.md", entry.bibtex_key));
    if !note_path.exists() {
        let header = format!("# {}\ncitekey: {}\n\n", entry.title.as_deref().unwrap_or("Untitled"), entry.bibtex_key);
        std::fs::write(&note_path, &header)?;
    }
    app.pending_editor = Some(note_path);
    app.mode = Mode::Normal;
    Ok(false)
}

// ── Entry point ──────────────────────────────────────────────────────────────

pub fn run_tui(config: &Config) -> Result<()> {
    // 플러그인 매니페스트와 키맵은 raw mode 앞에서 읽는다. 진단이 평범한 stdout에 찍히고,
    // 사용자가 Enter로 확인한 뒤에 TUI가 뜬다.
    let (host, plugin_problems) = PluginHost::discover(config);
    let host = Arc::new(host);
    let keymap_report = crate::keymap::load_keymap(host.commands());
    let keymap_noisy = !keymap_report.errors.is_empty() || !keymap_report.warnings.is_empty();
    // 테마는 데이터라 여기서 읽어 전역에 둔다. 깨졌으면 terminal로 뜨고 이유를 한 줄 보인다
    let theme_problem = match crate::theme::load(&config.theme, &crate::config::themes_dir()) {
        Ok(t) => {
            crate::theme::set_theme(t);
            None
        }
        Err(e) => Some(e),
    };
    if keymap_noisy || !plugin_problems.is_empty() || theme_problem.is_some() {
        if let Some(e) = &theme_problem {
            println!("{}", config.msgs.theme_problem(e));
            println!();
        }
        if !plugin_problems.is_empty() {
            println!("{}", config.msgs.plugin_problem_header());
            println!();
            for p in &plugin_problems {
                println!("{}", config.msgs.plugin_problem(p));
            }
            println!();
        }
        if keymap_noisy {
            if keymap_report.errors.is_empty() {
                println!("{}", config.msgs.keymap_warning_header());
            } else {
                println!("{}", config.msgs.keymap_fallback_header());
            }
            println!();
            for p in keymap_report.errors.iter().chain(keymap_report.warnings.iter()) {
                println!("{}", config.msgs.keymap_problem(p));
            }
            println!();
        }
        println!("{}", config.msgs.keymap_press_enter());
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
    }

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let config_clone = Config {
        home: config.home.clone(),
        pdf_dir: config.pdf_dir.clone(),
        bibox_dir: config.bibox_dir.clone(),
        pdf_viewer: config.pdf_viewer.clone(),
        default_collection: config.default_collection.clone(),
        search_case_sensitive: config.search_case_sensitive,
        default_page_size: config.default_page_size,
        language: config.language.clone(),
        git: config.git,
        notes_dir: config.notes_dir.clone(),
        templates_dir: config.templates_dir.clone(),
        line_numbers: config.line_numbers.clone(),
        panel_ratio: config.panel_ratio,
        bib_export_dir: config.bib_export_dir.clone(),
        export_dir: config.export_dir.clone(),
        citekey_format: config.citekey_format.clone(),
        natural_scroll: config.natural_scroll,
        status_bar: config.status_bar,
        images: config.images,
        theme: config.theme.clone(),
        plugin_status_bar: config.plugin_status_bar,
        plugins: config.plugins.clone(),
        msgs: crate::i18n::Msgs::new(&config.language),
    };

    let mut app = App::new(config_clone, Arc::clone(&host))?;
    app.images = detect_images(config.images);
    // 플러그인은 initialize에서 이걸 받는다. 그림 지원은 여기서야 안다
    app.host.set_capabilities(Capabilities { images: app.images.is_some(), status_bar: config.plugin_status_bar });
    app.keymap = keymap_report.keymap;
    app.apply_filters();

    let result = run_loop(&mut terminal, &mut app);

    // 올려 둔 그림을 터미널에서 거둔다. 안 하면 창을 닫을 때까지 남는다
    let gone = app.tab_cache.clear();
    app.kitty_deletes.extend(gone);
    let _ = flush_kitty_deletes(&mut app);
    host.shutdown();
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
    terminal.show_cursor()?;
    result
}

/// 설정과 환경으로 이미지 그리기를 정한다. 대체 화면에 들어간 뒤, 이벤트를 읽기 전에 불러야 한다.
/// auto는 터미널에 묻는다. 강제 프로토콜은 묻지 않는다. 그때 칸 크기는 ratatui-image의 기본값(10x20)인데,
/// 요청 폭과 자르기가 모두 그 값으로 계산되므로 폭 맞춤은 그대로 맞고 해상도만 실제 칸과 조금 다르다.
fn detect_images(images: crate::config::Images) -> Option<ratatui_image::picker::Picker> {
    use crate::config::Images;
    use ratatui_image::picker::{Picker, ProtocolType};
    let forced = |p: ProtocolType| {
        let mut picker = Picker::halfblocks();
        picker.set_protocol_type(p);
        Some(picker)
    };
    match images {
        Images::Off => None,
        Images::Kitty => forced(ProtocolType::Kitty),
        Images::Iterm2 => forced(ProtocolType::Iterm2),
        Images::Sixel => forced(ProtocolType::Sixel),
        Images::Halfblocks => forced(ProtocolType::Halfblocks),
        Images::Auto if crate::config::in_multiplexer() => None,
        Images::Auto => Picker::from_query_stdio().ok(),
    }
}

/// kitty 쪽을 플레이스홀더로 그린다. 처음 한 번은 전송 시퀀스를 첫 칸에 같이 싣는다(ratatui-image와 같은 방식).
/// 나머지 칸은 diff에서 건너뛰어 ratatui가 덮어쓰지 않게 한다.
struct KittyPage<'a> {
    page: &'a crate::kitty::Page,
    scroll_rows: u16,
    pan_cols: u16,
    cell_w: u16,
    cell_h: u16,
}

impl ratatui::widgets::Widget for KittyPage<'_> {
    fn render(self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        use ratatui::buffer::CellDiffOption;
        if area.width == 0 || area.height == 0 {
            return;
        }
        let page_cols = (self.page.width.div_ceil(self.cell_w.max(1) as u32)).min(u16::MAX as u32) as u16;
        let page_rows = (self.page.height.div_ceil(self.cell_h.max(1) as u32)).min(u16::MAX as u32) as u16;
        let cols = area.width.min(page_cols.saturating_sub(self.pan_cols));
        let rows = area.height.min(page_rows.saturating_sub(self.scroll_rows));
        if cols == 0 {
            return;
        }
        let mut transmit = self.page.take_transmit();
        for y in 0..rows {
            let mut symbol = transmit.take().unwrap_or_default();
            symbol.push_str(&self.page.row(self.scroll_rows + y, self.pan_cols, cols, area.width.saturating_sub(1), area.height.saturating_sub(1)));
            for x in 1..cols {
                if let Some(cell) = buf.cell_mut((area.x + x, area.y + y)) {
                    cell.set_diff_option(CellDiffOption::Skip);
                }
            }
            if let Some(cell) = buf.cell_mut((area.x, area.y + y)) {
                cell.set_symbol(&symbol).set_diff_option(CellDiffOption::ForcedWidth(std::num::NonZeroU16::new(1).unwrap()));
            }
        }
    }
}

/// 쪽 하나를 통째로 올린다. 이미지가 창보다 넓으면(줌) pan만큼 열만 잘라 낸다.
/// kitty는 한 번 전송한 뒤 플레이스홀더 행만 바꿔 스크롤하고, halfblocks/sixel도 한 번 인코딩한 것을 행 단위로 쓴다.
fn page_protocol(picker: &ratatui_image::picker::Picker, img: &image::DynamicImage, vp: &crate::preview_tabs::Viewport, pan_px: u32) -> Option<ratatui_image::sliced::SlicedProtocol> {
    let (x, w) = crate::preview_tabs::columns(vp, img.width(), pan_px);
    let page = if w < img.width() { img.crop_imm(x, 0, w, img.height()) } else { img.clone() };
    ratatui_image::sliced::SlicedProtocol::new(picker, page, None).ok()
}

/// 올려 둔 쪽을 `scroll_rows`행 위로 밀어 그린다.
fn page_widget(sliced: &ratatui_image::sliced::SlicedProtocol, scroll_rows: u16) -> ratatui_image::sliced::SlicedImage<'_> {
    let y = -(scroll_rows.min(i16::MAX as u16) as i16);
    ratatui_image::sliced::SlicedImage::new(sliced, ratatui_image::sliced::SignedPosition { x: 0, y })
}

fn draw_search_result_picker(f: &mut Frame, state: &SearchResultPickerState, area: Rect) {
    let popup_area = centered_rect(80, 50, area);
    clear_area(f, popup_area);

    let mut lines = vec![
        Line::from(Span::styled(
            format!("Search results for [{}]:", state.key),
            Style::default().fg(theme().heading).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];

    for (i, r) in state.results.iter().enumerate() {
        let arrow = if i == state.index { "▶ " } else { "  " };
        let style = if i == state.index { Style::default().fg(theme().accent) } else { Style::default() };

        let author = if r.authors.is_empty() { "Unknown".into() } else {
            let first = r.authors[0].split(',').next().unwrap_or(&r.authors[0]).trim().to_string();
            if r.authors.len() > 1 { format!("{} et al.", first) } else { first }
        };
        let year = r.year.map(|y| y.to_string()).unwrap_or_default();

        lines.push(Line::from(vec![
            Span::styled(arrow, Style::default().fg(theme().heading)),
            Span::styled(format!("{} ({}) ", author, year), style.add_modifier(Modifier::BOLD)),
        ]));

        // Title (truncate if needed)
        let max_t = 60;
        let title_display = if r.title.len() > max_t {
            format!("    {}...", &r.title[..max_t])
        } else {
            format!("    {}", r.title)
        };
        lines.push(Line::from(Span::styled(title_display, style)));

        // Venue
        if let Some(ref v) = r.venue {
            lines.push(Line::from(Span::styled(
                format!("    {}", v), Style::default().fg(theme().muted),
            )));
        }
        lines.push(Line::from(""));
    }

    lines.push(Line::from(Span::styled(
        "↑↓ navigate  Enter select  Esc cancel",
        Style::default().fg(theme().muted),
    )));

    let popup = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(" Select paper "));
    f.render_widget(popup, popup_area);
}

fn handle_search_result_picker(app: &mut App, key: crossterm::event::KeyEvent) -> Result<bool> {
    let num = app.search_picker.as_ref().map(|s| s.results.len()).unwrap_or(0);
    if let Some(ref mut state) = app.search_picker {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                if state.index > 0 { state.index -= 1; }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if state.index < num.saturating_sub(1) { state.index += 1; }
            }
            KeyCode::Enter => {
                let selected = &state.results[state.index];
                let doi = selected.doi.clone();
                let entry_key = state.key.clone();
                app.search_picker = None;
                // Now fetch full metadata by DOI
                let (tx, rx) = std::sync::mpsc::channel();
                std::thread::spawn(move || {
                    let rt = tokio::runtime::Runtime::new().unwrap();
                    let result = rt.block_on(crate::crossref::fetch_metadata(&doi));
                    let _ = tx.send(result.map(|m| (entry_key, m)));
                });
                app.bg_meta_result = Some(rx);
                app.spinner_tick = 0;
                app.mode = Mode::Loading("Fetching metadata...".into());
            }
            KeyCode::Esc => {
                app.search_picker = None;
                app.mode = Mode::Normal;
            }
            _ => {}
        }
    }
    Ok(false)
}

fn draw_fetch_preview(f: &mut Frame, state: &FetchPreviewState, area: Rect) {
    let popup_area = centered_rect(80, 60, area);
    clear_area(f, popup_area);

    let mut lines = vec![
        Line::from(Span::styled(
            format!("Fetch results for [{}]:", state.key),
            Style::default().fg(theme().heading).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];

    for (i, change) in state.changes.iter().enumerate() {
        let arrow = if i == state.index { "▶ " } else { "  " };
        let check = if state.selected[i] { "[x]" } else { "[ ]" };

        let style = if !change.changed {
            Style::default().fg(theme().muted)
        } else if i == state.index {
            Style::default().fg(theme().accent)
        } else {
            Style::default()
        };

        // Truncate long values
        let max_val = 35;
        let old_display = if change.old_val.len() > max_val {
            format!("{}...", &change.old_val[..max_val])
        } else if change.old_val.is_empty() {
            "(empty)".into()
        } else {
            change.old_val.clone()
        };
        let new_display = if change.new_val.len() > max_val {
            format!("{}...", &change.new_val[..max_val])
        } else if change.new_val.is_empty() {
            "(empty)".into()
        } else {
            change.new_val.clone()
        };

        if change.changed {
            lines.push(Line::from(vec![
                Span::styled(arrow, Style::default().fg(theme().heading)),
                Span::styled(format!("{} ", check), style),
                Span::styled(format!("{:<10} ", change.field), style.add_modifier(Modifier::BOLD)),
                Span::styled(old_display.clone(), Style::default().fg(theme().error)),
            ]));
            lines.push(Line::from(vec![
                Span::raw("               "),
                Span::styled(format!(" -> {}", new_display), Style::default().fg(theme().success)),
            ]));
        } else {
            lines.push(Line::from(vec![
                Span::styled(arrow, Style::default().fg(theme().heading)),
                Span::styled(format!("{} ", check), style),
                Span::styled(format!("{:<10} ", change.field), style),
                Span::styled(old_display.clone(), style),
                Span::styled("  (unchanged)", Style::default().fg(theme().muted)),
            ]));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "↑↓ navigate  Space toggle  Enter apply  Esc cancel",
        Style::default().fg(theme().muted),
    )));

    let popup = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(" Fetch Preview "));
    f.render_widget(popup, popup_area);
}

fn handle_fetch_preview(app: &mut App, key: crossterm::event::KeyEvent) -> Result<bool> {
    let num_items = app.fetch_preview.as_ref().map(|s| s.changes.len()).unwrap_or(0);
    if let Some(ref mut state) = app.fetch_preview {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                if state.index > 0 { state.index -= 1; }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if state.index < num_items.saturating_sub(1) { state.index += 1; }
            }
            KeyCode::Char(' ') => {
                state.selected[state.index] = !state.selected[state.index];
            }
            KeyCode::Enter => {
                // Apply selected changes
                app.push_undo();
                let Some(preview) = app.fetch_preview.take() else { return Ok(false); };
                let db_path = crate::config::resolve_db_path(&app.config);
                let mut db = load_db(&db_path)?;

                // Check if citekey is changing
                let old_key = preview.key.clone();
                let mut new_key = old_key.clone();
                for (i, change) in preview.changes.iter().enumerate() {
                    if preview.selected[i] && change.field == "Citekey" && change.changed {
                        new_key = change.new_val.clone();
                    }
                }

                // Apply field changes to DB entry
                let mut key_after = old_key.clone();
                if let Some(entry) = db.entries.iter_mut().find(|e| e.bibtex_key == old_key) {
                    for (i, change) in preview.changes.iter().enumerate() {
                        if !preview.selected[i] || !change.changed { continue; }
                        match change.field.as_str() {
                            "Title" => entry.title = Some(change.new_val.clone()),
                            "Author" => {
                                entry.author = change.new_val.split("; ")
                                    .map(|s| s.to_string()).collect();
                            }
                            "Year" => entry.year = change.new_val.parse().ok(),
                            "Journal" => entry.journal = if change.new_val.is_empty() { None } else { Some(change.new_val.clone()) },
                            "Publisher" => entry.publisher = if change.new_val.is_empty() { None } else { Some(change.new_val.clone()) },
                            "DOI" => entry.doi = Some(change.new_val.clone()),
                            "Citekey" => {
                                // Rename PDF file if exists
                                if let Some(ref fp) = entry.file_path {
                                    let old_path = app.config.bibox_dir.join(fp);
                                    let new_fp = format!("{}.pdf", new_key);
                                    let new_path = app.config.bibox_dir.join(&new_fp);
                                    if old_path.exists() {
                                        let _ = std::fs::rename(&old_path, &new_path);
                                    }
                                    entry.file_path = Some(new_fp);
                                }
                                // Rename note file if exists
                                let old_note = app.config.notes_dir.join(format!("{}.md", old_key));
                                if old_note.exists() {
                                    let new_note = app.config.notes_dir.join(format!("{}.md", new_key));
                                    let _ = std::fs::rename(&old_note, &new_note);
                                }
                                entry.bibtex_key = new_key.clone();
                            }
                            _ => {}
                        }
                    }
                    entry.updated_at = Some(chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string());
                    key_after = entry.bibtex_key.clone();
                }
                save_db(&db, &db_path)?;

                // Reload in-memory entries
                app.entries = db.entries;
                app.rebuild_collections();
                app.apply_filters();
                if let Some(e) = app.entries.iter().find(|e| e.bibtex_key == key_after).cloned() {
                    app.fire_after_write(crate::events::WriteReason::Edit, vec![e]);
                }

                let applied = preview.selected.iter().filter(|s| **s).count();
                app.mode = Mode::Message(format!("Updated {} field(s).", applied));
            }
            KeyCode::Esc => {
                app.fetch_preview = None;
                app.mode = Mode::Normal;
            }
            _ => {}
        }
    }
    Ok(false)
}

/// 캐시에서 빠진 kitty 그림을 터미널에서 지운다. 프레임 사이에 쓰므로 ratatui 출력과 섞이지 않는다.
fn flush_kitty_deletes(app: &mut App) -> Result<()> {
    if app.kitty_deletes.is_empty() {
        return Ok(());
    }
    use std::io::Write as _;
    let mut out = io::stdout().lock();
    for id in app.kitty_deletes.drain(..) {
        out.write_all(crate::kitty::delete(id).as_bytes())?;
    }
    out.flush()?;
    Ok(())
}

fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> Result<()> {
    loop {
        let t_frame = std::time::Instant::now();
        terminal.draw(|f| draw(f, app))?;
        let frame_ms = t_frame.elapsed().as_secs_f64() * 1000.0;
        flush_kitty_deletes(app)?;
        if frame_ms >= 20.0 {
            crate::trace::log(|| format!("frame {:.0}ms plugin_tab={} pending={}", frame_ms, matches!(app.preview_mode, PreviewMode::Plugin(_)), app.tab.pending.is_some()));
        }

        if event::poll(std::time::Duration::from_millis(16))? {
            let ev = event::read()?;

            // Drain pending events to avoid input lag from buffered mouse scrolls.
            let mut batch = vec![ev];
            while event::poll(std::time::Duration::ZERO)? {
                batch.push(event::read()?);
            }

            let (keys, mouse_event) = coalesce_events(&batch);

            if keys.is_empty() {
                if let Some(mouse) = mouse_event {
                    handle_mouse(app, mouse);
                }
            } else {
                let mut quit = false;
                for key in keys {
                    crate::trace::log(|| format!("key {:?}", key.code));
                    if handle_key(app, key)? { quit = true; break; }
                }
                if quit { break; }
            }
        }

        // fields/get 답 거두기
        let mut done = Vec::new();
        for (i, rx) in app.fields_rx.iter().enumerate() {
            match rx.try_recv() {
                Ok((plugin, r)) => {
                    app.fields.set_many(&plugin, &r.fields);
                    done.push(i);
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => done.push(i),
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        for i in done.into_iter().rev() {
            app.fields_rx.remove(i);
        }
        // 보이는 항목의 필드. wanted가 이미 물은 키를 거르므로 매 프레임 불러도 싸다
        let keys = app.visible_keys();
        app.pull_fields(keys);
        // 커서가 SETTLE_MS 멈춘 뒤 한 번
        let sel_key = app.selected_entry().map(|e| e.bibtex_key.clone());
        if sel_key != app.last_sel_key {
            app.last_sel_key = sel_key;
            app.sel_since = Some(std::time::Instant::now());
        }
        if let Some(since) = app.sel_since {
            if crate::preview_tabs::settled(since, std::time::Instant::now()) {
                app.sel_since = None;
                app.on_cursor_settled();
            }
        }

        // Handle pending editor
        if let Some(note_path) = app.pending_editor.take() {
            let editor = std::env::var("EDITOR").unwrap_or_else(|_| {
                if std::process::Command::new("which").arg("nano").output()
                    .map(|o| o.status.success()).unwrap_or(false)
                { "nano".to_string() } else { "vi".to_string() }
            });
            disable_raw_mode()?;
            execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture)?;
            let status = std::process::Command::new(&editor).arg(&note_path).status();
            enable_raw_mode()?;
            execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
            terminal.clear()?;
            if let Err(e) = status {
                app.mode = Mode::Message(format!("Editor failed: {}", e));
            }
            if let Some(e) = app.selected_entry().cloned() {
                app.fire_after_note_save(e, note_path.clone());
            }
            // Reload note if in note preview mode
            app.note_citekey.clear();
        }

        // 플러그인이 보낸 요청·알림·종료
        while let Some(ev) = app.host.try_recv_event() {
            app.handle_host_event(ev);
        }
        if matches!(app.mode, Mode::Normal) {
            if let Some(m) = app.pending_messages.pop_front() {
                app.mode = Mode::Message(m);
            }
        }
        while let Some(a) = app.queued_actions.pop_front() {
            if execute(app, a)? == Flow::Quit {
                return Ok(());
            }
        }
        if !app.started {
            app.started = true;
            let host = Arc::clone(&app.host);
            std::thread::spawn(move || {
                for p in host.startup_plugins() {
                    let _ = host.notify(&p, "lifecycle/started", serde_json::json!({}));
                }
            });
        }

        // Poll plugin command
        if app.plugin_run.is_some() {
            let (plugin, ev) = {
                let run = app.plugin_run.as_ref().expect("checked");
                (run.plugin.clone(), run.rx.try_recv())
            };
            match ev {
                Ok(result) => {
                    app.plugin_run = None;
                    app.finish_plugin(&plugin, result);
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => { app.spinner_tick = app.spinner_tick.wrapping_add(1); }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    app.plugin_run = None;
                    app.mode = Mode::Message(format!("{}: worker disconnected", plugin));
                }
            }
        }

        // Poll background task
        if let Some(ref rx) = app.bg_result {
            match rx.try_recv() {
                Ok(Ok(result)) => {
                    let db_path = crate::config::resolve_db_path(&app.config);
                    if let Ok(mut db) = load_db(&db_path) {
                        if let Some(db_entry) = find_by_key_mut(&mut db, &result.key) {
                            db_entry.file_path = Some(result.file_path.clone());
                        }
                        let _ = save_db(&db, &db_path);
                    }
                    if let Some(mem_entry) = app.entries.iter_mut().find(|e| e.bibtex_key == result.key) {
                        mem_entry.file_path = Some(result.file_path);
                    }
                    app.bg_result = None;
                    app.bg_fetch_key = None;
                    if let Some(e) = app.entries.iter().find(|e| e.bibtex_key == result.key).cloned() {
                        app.fire_after_write(crate::events::WriteReason::Edit, vec![e]);
                    }
                    app.mode = Mode::Message(format!("PDF saved: {}", result.full_path));
                }
                Ok(Err(e)) => {
                    app.bg_result = None;
                    let err_str = e.to_string();
                    let is_access_denied = err_str.contains("403") || err_str.contains("Forbidden");
                    if is_access_denied {
                        if let Some(key) = app.bg_fetch_key.take() {
                            if let Some(entry) = app.entries.iter().find(|en| en.bibtex_key == key).cloned() {
                                let url = entry.doi.as_ref().map(|d| format!("https://doi.org/{}", d))
                                    .or_else(|| entry.url.clone());
                                if let Some(url) = url {
                                    app.mode = Mode::Confirm(ConfirmAction::OpenBrowser(key, url));
                                } else {
                                    app.mode = Mode::Message(format!("Fetch failed: {}", err_str));
                                }
                            } else {
                                app.mode = Mode::Message(format!("Fetch failed: {}", err_str));
                            }
                        } else {
                            app.mode = Mode::Message(format!("Fetch failed: {}", err_str));
                        }
                    } else {
                        app.bg_fetch_key = None;
                        app.mode = Mode::Message(format!("Fetch failed: {}", err_str));
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => { app.spinner_tick += 1; }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    app.bg_result = None;
                    app.mode = Mode::Message("Fetch failed: thread disconnected.".into());
                }
            }
        }

        // Poll metadata fetch result
        if let Some(ref rx) = app.bg_meta_result {
            match rx.try_recv() {
                Ok(Ok((key, meta))) => {
                    app.bg_meta_result = None;
                    // Build FetchPreview from current entry vs fetched metadata
                    if let Some(entry) = app.entries.iter().find(|e| e.bibtex_key == key) {
                        let mut changes = vec![];
                        let old_title = entry.title.clone().unwrap_or_default();
                        let new_title = meta.title.clone().unwrap_or_default();
                        changes.push(FieldChange {
                            field: "Title".into(), old_val: old_title.clone(), new_val: new_title.clone(),
                            changed: old_title != new_title,
                        });
                        let old_author = entry.author.join("; ");
                        let new_author = meta.authors.join("; ");
                        changes.push(FieldChange {
                            field: "Author".into(), old_val: old_author.clone(), new_val: new_author.clone(),
                            changed: old_author != new_author,
                        });
                        let old_year = entry.year.map(|y| y.to_string()).unwrap_or_default();
                        let new_year = meta.year.map(|y| y.to_string()).unwrap_or_default();
                        changes.push(FieldChange {
                            field: "Year".into(), old_val: old_year.clone(), new_val: new_year.clone(),
                            changed: old_year != new_year,
                        });
                        let old_journal = entry.journal.clone().unwrap_or_default();
                        let new_journal = meta.journal.clone().unwrap_or_default();
                        changes.push(FieldChange {
                            field: "Journal".into(), old_val: old_journal.clone(), new_val: new_journal.clone(),
                            changed: old_journal != new_journal,
                        });
                        let old_publisher = entry.publisher.clone().unwrap_or_default();
                        let new_publisher = meta.publisher.clone().unwrap_or_default();
                        changes.push(FieldChange {
                            field: "Publisher".into(), old_val: old_publisher.clone(), new_val: new_publisher.clone(),
                            changed: old_publisher != new_publisher,
                        });
                        let old_doi = entry.doi.clone().unwrap_or_default();
                        let new_doi = meta.doi.clone();
                        changes.push(FieldChange {
                            field: "DOI".into(), old_val: old_doi.clone(), new_val: new_doi.clone(),
                            changed: old_doi != new_doi,
                        });
                        // Citekey: generate from new data using config format
                        let new_authors = meta.authors.clone();
                        let new_key = crate::storage::generate_bibtex_key_fmt(
                            &new_authors, meta.year,
                            meta.title.as_deref().unwrap_or("unknown"),
                            &app.config.citekey_format,
                        );
                        let new_key_unique = crate::storage::generate_unique_key_excluding(
                            &app.entries.iter().map(|e| e.bibtex_key.as_str()).collect::<Vec<_>>(),
                            &new_key, &key,
                        );
                        changes.push(FieldChange {
                            field: "Citekey".into(), old_val: key.clone(), new_val: new_key_unique.clone(),
                            changed: key != new_key_unique,
                        });
                        // Default: select changed fields
                        let selected: Vec<bool> = changes.iter().map(|c| c.changed).collect();
                        app.fetch_preview = Some(FetchPreviewState {
                            key,
                            changes,
                            selected,
                            index: 0,
                        });
                        app.mode = Mode::FetchPreview;
                    } else {
                        app.mode = Mode::Message("Entry not found in memory.".into());
                    }
                }
                Ok(Err(e)) => {
                    app.bg_meta_result = None;
                    app.mode = Mode::Message(format!("Crossref fetch failed: {}", e));
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => { app.spinner_tick += 1; }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    app.bg_meta_result = None;
                    app.mode = Mode::Message("Metadata fetch failed: thread disconnected.".into());
                }
            }
        }

        // Poll a staged plugin install (clone finished)
        if let Some(ref rx) = app.bg_install {
            match rx.try_recv() {
                Ok(Ok(staged)) => {
                    app.bg_install = None;
                    app.mode = Mode::Confirm(ConfirmAction::InstallStaged(staged));
                }
                Ok(Err(e)) => {
                    app.bg_install = None;
                    app.settings.notice = Some((app.config.msgs.plugin_install_failed(&e.to_string()), true));
                    app.mode = Mode::Settings;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => { app.spinner_tick += 1; }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    app.bg_install = None;
                    app.settings.notice = Some((app.config.msgs.plugin_install_failed("thread disconnected"), true));
                    app.mode = Mode::Settings;
                }
            }
        }

        // Poll a preview tab render
        if let Some(ref rx) = app.bg_tab {
            match rx.try_recv() {
                Ok((tab_index, key, result)) => {
                    crate::trace::log(|| format!("tab.recv p{}", key.page));
                    app.bg_tab = None;
                    // 다른 탭으로 옮긴 뒤 온 답은 버린다(캐시 키에 플러그인이 없다)
                    if app.preview_mode != PreviewMode::Plugin(tab_index) {
                        app.tab.pending = None;
                    } else {
                        app.finish_tab(key, result);
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    app.bg_tab = None;
                    app.tab.pending = None;
                }
            }
        }

        // Poll search results (title-based Crossref search)
        if let Some(ref rx) = app.bg_search_result {
            match rx.try_recv() {
                Ok(Ok((key, results))) => {
                    app.bg_search_result = None;
                    if results.is_empty() {
                        app.mode = Mode::Message("No results found on Crossref.".into());
                    } else {
                        app.search_picker = Some(SearchResultPickerState {
                            key,
                            results,
                            index: 0,
                        });
                        app.mode = Mode::SearchResultPicker;
                    }
                }
                Ok(Err(e)) => {
                    app.bg_search_result = None;
                    app.mode = Mode::Message(format!("Crossref search failed: {}", e));
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => { app.spinner_tick += 1; }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    app.bg_search_result = None;
                    app.mode = Mode::Message("Search failed: thread disconnected.".into());
                }
            }
        }
    }
    Ok(())
}

/// Fetch PDF via Unpaywall (DOI) or direct URL. Runs on background thread.
fn run_fetch_pdf(entry: &Entry, bibox_dir: &std::path::Path) -> Result<(String, String)> {
    let rt = tokio::runtime::Runtime::new()?;
    let tmp = std::env::temp_dir().join("bibox_download.pdf");

    rt.block_on(async {
        if let Some(ref doi) = entry.doi {
            match crate::unpaywall::find_open_access(doi).await {
                Ok(Some(oa)) => { crate::unpaywall::download_pdf(&oa.pdf_url, &tmp).await?; }
                Ok(None) => {
                    if let Some(ref url) = entry.url {
                        let pdf_url = if url.contains("arxiv.org/abs/") {
                            url.replace("arxiv.org/abs/", "arxiv.org/pdf/")
                        } else {
                            url.clone()
                        };
                        crate::unpaywall::download_pdf(&pdf_url, &tmp).await?;
                    } else { anyhow::bail!("No open-access PDF found."); }
                }
                Err(e) => return Err(e),
            }
        } else if let Some(ref url) = entry.url {
            let pdf_url = if url.contains("arxiv.org/abs/") {
                url.replace("arxiv.org/abs/", "arxiv.org/pdf/")
            } else {
                url.clone()
            };
            crate::unpaywall::download_pdf(&pdf_url, &tmp).await?;
        } else { anyhow::bail!("No DOI or URL to fetch from."); }
        Ok(())
    })?;

    std::fs::create_dir_all(bibox_dir)?;
    let filename = entry_to_filename(entry);
    let dest = bibox_dir.join(&filename);
    std::fs::copy(&tmp, &dest)?;
    let _ = std::fs::remove_file(&tmp);
    Ok((filename, dest.to_string_lossy().to_string()))
}

#[cfg(test)]
mod tests {
    use super::{collection_paths, plugin_page_title};
    use crate::models::{Entry, EntryType};

    /// 쪽 그림은 한 번만 전송되고, 스크롤은 유니코드 플레이스홀더의 행 번호만 바꾼다.
    /// (창마다 잘라 새로 보내던 옛 방식은 j 한 번에 수 MB를 다시 보냈다.)
    #[test]
    fn a_page_is_transmitted_once_and_scrolling_only_moves_placeholder_rows() {
        use super::{page_protocol, page_widget};
        use crate::preview_tabs::Viewport;
        use ratatui::{buffer::Buffer, layout::Rect, widgets::Widget};
        use ratatui_image::picker::{Picker, ProtocolType};

        let mut picker = Picker::halfblocks(); // 10x20 칸
        picker.set_protocol_type(ProtocolType::Kitty);
        let vp = Viewport { cols: 40, rows: 10, cell_w: 10, cell_h: 20 };
        // 패널 폭 그대로, 60행짜리 쪽
        let img = image::DynamicImage::new_rgb8(400, 1200);
        let sliced = page_protocol(&picker, &img, &vp, 0).expect("kitty protocol");
        let area = Rect::new(0, 0, 40, 10);
        let dump = |b: &Buffer| b.content().iter().map(|c| c.symbol().to_string()).collect::<String>();

        let mut first = Buffer::empty(area);
        page_widget(&sliced, 0).render(area, &mut first);
        let d1 = dump(&first);
        assert!(d1.contains("\x1b_Gq=2,i=") && d1.contains("a=T,U=1"), "the first frame transmits the page");

        let mut second = Buffer::empty(area);
        page_widget(&sliced, 3).render(area, &mut second);
        let d2 = dump(&second);
        assert!(!d2.contains("\x1b_G"), "scrolling three rows sends no image data");
        assert_eq!(second.cell((0, 0)).unwrap().symbol(), first.cell((0, 3)).unwrap().symbol(), "the top row now shows the page's fourth row");
    }

    /// kitty 경로: 전송 시퀀스는 첫 프레임의 첫 칸에 한 번, 그 뒤 스크롤·pan은 플레이스홀더의 행·열 diacritic만 바뀐다.
    #[test]
    fn a_kitty_page_is_sent_once_and_then_only_placeholders_move() {
        use super::KittyPage;
        use crate::kitty::{diacritic, encode, PLACEHOLDER};
        use ratatui::{buffer::Buffer, layout::Rect, widgets::Widget};

        // 10x20 칸, 40열 60행짜리 쪽
        let page = encode(&image::DynamicImage::new_rgb8(400, 1200), 5);
        let area = Rect::new(0, 0, 40, 10);
        let mut first = Buffer::empty(area);
        KittyPage { page: &page, scroll_rows: 0, pan_cols: 0, cell_w: 10, cell_h: 20 }.render(area, &mut first);
        let top = first.cell((0, 0)).unwrap().symbol().to_string();
        assert!(top.contains("\x1b_Gq=2,i=5,a=T,U=1,f=24,o=z,t=d,s=400,v=1200,m="), "first cell carries the transmit");
        assert!(top.ends_with("\x1b[u\x1b[39C\x1b[9B"), "cursor restore to the area's far corner");
        assert_eq!(first.cell((0, 3)).unwrap().symbol().matches(PLACEHOLDER).count(), 40, "a full row of placeholders");
        assert!(!first.cell((0, 1)).unwrap().symbol().contains("\x1b_G"), "only the first row transmits");

        let mut second = Buffer::empty(area);
        KittyPage { page: &page, scroll_rows: 7, pan_cols: 2, cell_w: 10, cell_h: 20 }.render(area, &mut second);
        let top = second.cell((0, 0)).unwrap().symbol().to_string();
        assert!(!top.contains("\x1b_G"), "no image data on a scroll");
        let expect = format!("{}{}{}{}", PLACEHOLDER, diacritic(7), diacritic(2), diacritic(0));
        assert!(top.contains(&expect), "row 7, column 2: {:?}", &top[..40.min(top.len())]);
        assert_eq!(second.cell((0, 0)).unwrap().symbol().matches(PLACEHOLDER).count(), 38, "two columns panned away leave 38 of the 40");
    }

    /// 줌으로 쪽이 창보다 넓으면 pan만큼 열을 잘라 낸 것을 올린다.
    #[test]
    fn page_protocol_crops_columns_to_the_pan() {
        use super::page_protocol;
        use crate::preview_tabs::Viewport;
        use ratatui_image::picker::Picker;

        let picker = Picker::halfblocks();
        let vp = Viewport { cols: 40, rows: 10, cell_w: 10, cell_h: 20 };
        let wide = image::DynamicImage::new_rgb8(800, 400);
        let s = page_protocol(&picker, &wide, &vp, 100).unwrap();
        assert_eq!((s.size().width, s.size().height), (40, 20), "40 columns of the 80, all 20 rows");
        let narrow = image::DynamicImage::new_rgb8(200, 400);
        let s = page_protocol(&picker, &narrow, &vp, 100).unwrap();
        assert_eq!(s.size().width, 20, "a narrow page is not cropped");
    }

    #[test]
    fn plugin_page_title_skips_an_empty_version() {
        assert_eq!(plugin_page_title("pdf-view", "0.4.0", "built-in"), "pdf-view  0.4.0  built-in");
        assert_eq!(plugin_page_title("localplug", "", "local"), "localplug  local");
        assert_eq!(plugin_page_title("broken", "", "error"), "broken  error");
    }

    fn entry_in(collections: &[&str]) -> Entry {
        Entry {
            id: "id".to_string(),
            bibtex_key: "key".to_string(),
            entry_type: EntryType::Article,
            title: None,
            author: vec![],
            year: None,
            journal: None,
            volume: None,
            number: None,
            pages: None,
            publisher: None,
            editor: None,
            edition: None,
            isbn: None,
            booktitle: None,
            doi: None,
            url: None,
            abstract_text: None,
            tags: vec![],
            howpublished: None,
            month: None,
            note: None,
            collections: collections.iter().map(|s| s.to_string()).collect(),
            file_path: None,
            created_at: String::new(),
            updated_at: None,
        }
    }

    fn key(c: char) -> crossterm::event::Event {
        crossterm::event::Event::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char(c),
            crossterm::event::KeyModifiers::NONE,
        ))
    }

    fn scroll(row: u16) -> crossterm::event::Event {
        crossterm::event::Event::Mouse(crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::ScrollDown,
            column: 0,
            row,
            modifiers: crossterm::event::KeyModifiers::NONE,
        })
    }

    #[test]
    fn coalesce_events_keeps_every_keystroke_in_order() {
        let batch = vec![key('c'), key('o'), key('p'), key('y')];
        let (keys, _) = super::coalesce_events(&batch);
        let typed: String = keys
            .iter()
            .filter_map(|k| match k.code {
                crossterm::event::KeyCode::Char(c) => Some(c),
                _ => None,
            })
            .collect();
        assert_eq!(typed, "copy");
    }

    #[test]
    fn coalesce_events_keeps_keys_that_arrive_alongside_mouse_scrolls() {
        let batch = vec![scroll(1), key('a'), scroll(2), key('b')];
        let (keys, mouse) = super::coalesce_events(&batch);
        assert_eq!(keys.len(), 2);
        assert_eq!(mouse.map(|m| m.row), Some(2));
    }

    #[test]
    fn coalesce_events_collapses_mouse_scrolls_to_the_newest() {
        let batch = vec![scroll(1), scroll(2), scroll(3)];
        let (keys, mouse) = super::coalesce_events(&batch);
        assert!(keys.is_empty());
        assert_eq!(mouse.map(|m| m.row), Some(3));
    }

    fn entries_help() -> Vec<super::HelpRow> {
        super::help_rows(&default_keymap().entries, &crate::plugin::PluginCommands::default())
    }

    #[test]
    fn filter_rows_returns_everything_for_an_empty_query() {
        let rows = entries_help();
        assert_eq!(super::filter_rows(&rows, "").len(), rows.len());
    }

    #[test]
    fn filter_rows_matches_on_the_key_name() {
        // "gg" also appears inside "Toggle"/"toggle" in two descriptions, which is
        // correct for a substring search, so assert the key row is found rather
        // than pinning an exact count.
        let rows = entries_help();
        let hits = super::filter_rows(&rows, "gg");
        let top = hits.iter().find(|r| r.keys == "gg").expect("gg binding should be found");
        assert_eq!(top.action, "EntryTop");
        assert_eq!(top.section, "Navigation");
    }

    #[test]
    fn filter_rows_matches_text_that_only_appears_in_the_description() {
        // "clipboard" appears in no key and no action name, only in descriptions.
        let rows = entries_help();
        let hits = super::filter_rows(&rows, "clipboard");
        let actions: Vec<&str> = hits.iter().map(|r| r.action.as_str()).collect();
        assert_eq!(actions, vec!["CopyCitekey", "CopyCitation"]);
    }

    #[test]
    fn filter_rows_ignores_case() {
        let rows = entries_help();
        let upper = super::filter_rows(&rows, "UNDO");
        let lower = super::filter_rows(&rows, "undo");
        assert!(!upper.is_empty());
        assert_eq!(upper.len(), lower.len());
    }

    #[test]
    fn filter_rows_returns_nothing_when_no_row_matches() {
        let rows = entries_help();
        assert!(super::filter_rows(&rows, "zzzznotakey").is_empty());
    }

    #[test]
    fn collection_paths_synthesizes_missing_parent_row() {
        let entries = vec![
            entry_in(&["gym/seed"]),
            entry_in(&["gym/method"]),
            entry_in(&["displays-2026"]),
            entry_in(&["big"]),
        ];
        assert_eq!(
            collection_paths(&entries),
            vec!["big", "displays-2026", "gym", "gym/method", "gym/seed"]
        );
    }

    #[test]
    fn collection_paths_synthesizes_every_ancestor_of_deep_path() {
        let entries = vec![entry_in(&["digest/2026/04"])];
        assert_eq!(
            collection_paths(&entries),
            vec!["digest", "digest/2026", "digest/2026/04"]
        );
    }

    #[test]
    fn collection_paths_does_not_duplicate_parent_that_has_direct_entries() {
        let entries = vec![entry_in(&["gym"]), entry_in(&["gym/seed"])];
        assert_eq!(collection_paths(&entries), vec!["gym", "gym/seed"]);
    }

    #[test]
    fn collection_paths_is_empty_when_no_entry_has_a_collection() {
        let entries = vec![entry_in(&[]), entry_in(&[])];
        assert_eq!(collection_paths(&entries), Vec::<String>::new());
    }

    use super::ContextMenuState;
    use crate::keymap::{default_keymap, parse_key, Action, Binding as KmBinding, Layer as KmLayer, LayerId};

    #[test]
    fn help_rows_come_from_the_keymap() {
        let rows = super::help_rows(&default_keymap().entries, &crate::plugin::PluginCommands::default());
        // 별칭이 한 행으로 합쳐지므로 키 칸은 "j  <Down>"이다.
        assert!(rows.iter().any(|r| r.keys.starts_with("j") && r.desc.contains("down one entry")));
    }

    #[test]
    fn help_rows_show_a_custom_binding() {
        let layer = KmLayer {
            bindings: vec![KmBinding {
                keys: vec![parse_key("Z").unwrap()],
                actions: vec![Action::Quit],
                desc: None,
            }],
        };
        let rows = super::help_rows(&layer, &crate::plugin::PluginCommands::default());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].keys, "Z");
        assert_eq!(rows[0].desc, Action::Quit.desc());
    }

    #[test]
    fn a_binding_desc_wins_over_the_action_desc_in_help() {
        let layer = KmLayer {
            bindings: vec![KmBinding {
                keys: vec![parse_key("Z").unwrap()],
                actions: vec![Action::Quit],
                desc: Some("나가기".to_string()),
            }],
        };
        assert_eq!(super::help_rows(&layer, &crate::plugin::PluginCommands::default())[0].desc, "나가기");
    }

    #[test]
    fn help_rows_render_a_key_sequence() {
        let layer = KmLayer {
            bindings: vec![KmBinding {
                keys: vec![parse_key("g").unwrap(), parse_key("g").unwrap()],
                actions: vec![Action::EntryTop],
                desc: None,
            }],
        };
        assert_eq!(super::help_rows(&layer, &crate::plugin::PluginCommands::default())[0].keys, "gg");
    }

    #[test]
    fn help_rows_are_grouped_by_section_in_a_fixed_order() {
        // draw_help_popup은 섹션이 바뀔 때마다 제목을 끼워 넣는다. 행이 섹션순으로
        // 정렬돼 있지 않으면 같은 제목이 여러 번 나온다.
        let rows = super::help_rows(&default_keymap().entries, &crate::plugin::PluginCommands::default());
        let mut seen: Vec<&str> = Vec::new();
        for r in &rows {
            if seen.last() != Some(&r.section) {
                assert!(!seen.contains(&r.section), "section {} appears twice", r.section);
                seen.push(r.section);
            }
        }
        assert_eq!(seen.first(), Some(&"Navigation"));
    }

    #[test]
    fn entry_only_keys_do_not_appear_in_the_collections_help() {
        let rows = super::help_rows(&default_keymap().collections, &crate::plugin::PluginCommands::default());
        assert!(!rows.iter().any(|r| r.keys == "H"), "H does nothing in the collections panel");
    }

    #[test]
    fn alias_keys_share_one_help_row() {
        // j와 <Down>은 같은 동작이므로 한 행이어야 한다. 나누면 표가 알맹이 없이 길어진다.
        let rows = super::help_rows(&default_keymap().entries, &crate::plugin::PluginCommands::default());
        let down: Vec<&super::HelpRow> = rows.iter().filter(|r| r.action == "EntryDown").collect();
        assert_eq!(down.len(), 1, "EntryDown should occupy one row");
        assert_eq!(down[0].keys, "j  <Down>");
    }

    #[test]
    fn context_menu_items_carry_actions_not_characters() {
        let actions: Vec<Action> = super::ContextMenuState::ITEMS.iter().map(|(_, a)| *a).collect();
        assert!(actions.contains(&Action::OpenPdf));
        assert!(actions.contains(&Action::Delete));
        assert_eq!(actions.len(), 10);
    }

    #[test]
    fn shortcut_hint_reads_the_active_keymap() {
        assert_eq!(super::shortcut_hint(&default_keymap().entries, Action::OpenPdf), "o");
    }

    #[test]
    fn shortcut_hint_follows_a_remapped_key() {
        // 이 역조회가 없으면 사용자가 o를 리맵했을 때 메뉴가 옛 키를 계속 보여주고,
        // 위조된 KeyEvent 방식이었다면 조용히 다른 동작을 실행했다.
        let mut km = default_keymap();
        km.entries.bindings.insert(0, KmBinding {
            keys: vec![parse_key("P").unwrap()],
            actions: vec![Action::OpenPdf],
            desc: None,
        });
        assert_eq!(super::shortcut_hint(&km.entries, Action::OpenPdf), "P");
    }

    #[test]
    fn status_bar_follows_the_default_keymap() {
        let bar = super::status_bar_text(&default_keymap(), super::Panel::Entries);
        assert!(bar.contains("j/k navigate"), "got {}", bar);
        assert!(bar.contains("? help"), "got {}", bar);
        assert!(bar.contains("q quit"), "got {}", bar);
    }

    #[test]
    fn status_bar_follows_a_remapped_key() {
        // 하드코딩이던 시절 조용히 틀리던 지점이 정확히 여기다.
        let mut km = default_keymap();
        km.entries.bindings.insert(0, KmBinding {
            keys: vec![parse_key("<C-n>").unwrap()],
            actions: vec![Action::EntryDown],
            desc: None,
        });
        let bar = super::status_bar_text(&km, super::Panel::Entries);
        assert!(bar.contains("<C-n>/k navigate"), "got {}", bar);
    }

    #[test]
    fn status_bar_differs_per_panel() {
        let km = default_keymap();
        let entries = super::status_bar_text(&km, super::Panel::Entries);
        let preview = super::status_bar_text(&km, super::Panel::Preview);
        assert!(entries.contains("navigate"));
        assert!(preview.contains("scroll"), "got {}", preview);
        assert_ne!(entries, preview);
    }

    #[test]
    fn status_bar_omits_an_action_that_has_no_binding() {
        // 사용자가 clear_defaults로 키를 다 비우면 빈 힌트가 남아 "  search"처럼
        // 앞이 비어 보이면 안 된다.
        let km = crate::keymap::Keymap {
            entries: KmLayer { bindings: vec![KmBinding {
                keys: vec![parse_key("q").unwrap()],
                actions: vec![Action::Quit],
                desc: None,
            }] },
            ..Default::default()
        };
        let bar = super::status_bar_text(&km, super::Panel::Entries);
        assert!(bar.contains("q quit"), "got {}", bar);
        assert!(!bar.contains(" search"), "unbound actions must be dropped, got {}", bar);
        assert!(!bar.contains("  navigate"), "got {}", bar);
    }

    // ── Plugins ──

    use super::{plugin_ui_step, PluginUiKind};
    use crate::plugin::protocol::{UiAnswer, UiRequest};
    use crossterm::event::KeyCode;

    #[test]
    fn pick_moves_with_jk_and_answers_on_enter_or_esc() {
        let mut k = PluginUiKind::from_request("p", UiRequest::Pick { title: None, items: vec!["a".into(), "b".into(), "c".into()] }).unwrap();
        assert_eq!(plugin_ui_step(&mut k, KeyCode::Char('j')), None);
        assert_eq!(plugin_ui_step(&mut k, KeyCode::Char('j')), None);
        assert_eq!(plugin_ui_step(&mut k, KeyCode::Char('j')), None); // 끝에서 멈춤
        assert_eq!(plugin_ui_step(&mut k, KeyCode::Enter), Some(UiAnswer::Index { index: Some(2) }));
        assert_eq!(plugin_ui_step(&mut k, KeyCode::Char('k')), None);
        assert_eq!(plugin_ui_step(&mut k, KeyCode::Esc), Some(UiAnswer::Index { index: None }));
    }

    #[test]
    fn pick_title_falls_back_to_the_plugin_name() {
        match PluginUiKind::from_request("summarize", UiRequest::Pick { title: None, items: vec!["a".into()] }).unwrap() {
            PluginUiKind::Pick { title, .. } => assert_eq!(title, "summarize"),
            _ => panic!(),
        }
    }

    #[test]
    fn an_empty_pick_is_answered_immediately_with_null() {
        assert_eq!(
            PluginUiKind::from_request("p", UiRequest::Pick { title: None, items: vec![] }).unwrap_err(),
            UiAnswer::Index { index: None }
        );
    }

    #[test]
    fn progress_never_becomes_a_popup() {
        assert_eq!(
            PluginUiKind::from_request("p", UiRequest::Progress { text: "x".into() }).unwrap_err(),
            UiAnswer::Ack {}
        );
    }

    #[test]
    fn prompt_edits_a_buffer_seeded_with_the_default() {
        let mut k = PluginUiKind::from_request("p", UiRequest::Prompt { title: None, default: Some("ab".into()) }).unwrap();
        assert_eq!(plugin_ui_step(&mut k, KeyCode::Char('c')), None);
        assert_eq!(plugin_ui_step(&mut k, KeyCode::Backspace), None);
        assert_eq!(plugin_ui_step(&mut k, KeyCode::Char('q')), None); // q는 글자다
        assert_eq!(plugin_ui_step(&mut k, KeyCode::Enter), Some(UiAnswer::Text { text: Some("abq".into()) }));
        assert_eq!(plugin_ui_step(&mut k, KeyCode::Esc), Some(UiAnswer::Text { text: None }));
    }

    #[test]
    fn confirm_answers_y_and_n() {
        let mut k = PluginUiKind::from_request("p", UiRequest::Confirm { title: Some("Sure?".into()) }).unwrap();
        assert_eq!(plugin_ui_step(&mut k, KeyCode::Char('x')), None);
        assert_eq!(plugin_ui_step(&mut k, KeyCode::Char('y')), Some(UiAnswer::Yes { yes: true }));
        assert_eq!(plugin_ui_step(&mut k, KeyCode::Char('n')), Some(UiAnswer::Yes { yes: false }));
        assert_eq!(plugin_ui_step(&mut k, KeyCode::Esc), Some(UiAnswer::Yes { yes: false }));
    }

    #[test]
    fn export_scope_offers_the_collection_and_every_ancestor_with_subcollection_counts() {
        assert_eq!(super::collection_and_ancestors("gym/method/x"), vec!["gym/method/x", "gym/method", "gym"]);
        assert_eq!(super::collection_and_ancestors("gym"), vec!["gym"]);
        let mut a = entry_in(&["gym/method"]);
        a.bibtex_key = "a".into();
        let mut b = entry_in(&["gym"]);
        b.bibtex_key = "b".into();
        let mut c = entry_in(&["gymnastics"]);
        c.bibtex_key = "c".into();
        let entries = vec![a, b, c];
        assert_eq!(super::keys_in_collection(&entries, "gym/method"), vec!["a"]);
        assert_eq!(super::keys_in_collection(&entries, "gym"), vec!["a", "b"], "children included, gymnastics is not a child");
    }

    #[test]
    fn selectable_rows_are_items_plugins_installed_and_install_from() {
        use super::PaneRow::*;
        use super::selectable;
        assert!(selectable(&Item(0)) && selectable(&Plugin("a".into())) && selectable(&Installed("a".into())) && selectable(&InstallFrom));
        assert!(!selectable(&Header("h".into())) && !selectable(&Text("t".into())) && !selectable(&Blank));
    }

    #[test]
    fn settings_cursor_skips_headers_and_stops_at_the_ends() {
        use super::PaneRow::{self, *};
        use super::{first_selectable, move_cursor};
        let rows = vec![Header("General".into()), Item(0), Item(1), Blank, Header("Export".into()), Item(2)];
        assert_eq!(first_selectable(&rows), 1);
        assert_eq!(move_cursor(&rows, 1, 1), 2);
        assert_eq!(move_cursor(&rows, 2, 1), 5, "skips the blank and the header");
        assert_eq!(move_cursor(&rows, 5, 1), 5, "stays at the end");
        assert_eq!(move_cursor(&rows, 5, -1), 2);
        assert_eq!(move_cursor(&rows, 1, -1), 1, "stays at the start");
        let none: Vec<PaneRow> = vec![Header("x".into())];
        assert_eq!(first_selectable(&none), 0);
        assert_eq!(move_cursor(&none, 0, 1), 0);
    }

    fn plugin_table() -> crate::plugin::PluginCommands {
        use crate::plugin::manifest::{Command, Manifest};
        let m = Manifest {
            name: "tidy".into(), version: None, description: None, run: vec!["sh".into()],
            commands: vec![
                Command { id: "run".into(), desc: "Normalize the entry".into(), key: Some(vec![parse_key("=").unwrap()]), layers: vec![LayerId::Entries], menus: vec!["context".into()] },
                Command { id: "quiet".into(), desc: "No menu".into(), key: None, layers: vec![LayerId::Entries], menus: vec![] },
            ],
            activation: crate::plugin::manifest::Activation::Lazy, fields: vec![], views: vec![], events: vec![], cli: None, settings: vec![], builtin: None, dir: "/tmp".into(), guide: None,
        };
        let env = crate::plugin::PluginEnv { bin: "/bin/true".into(), config_dir: "/tmp".into(), db: "/tmp/db.json".into(), notes: "/tmp/n".into(), pdfs: "/tmp/p".into(), home: None, extra: Default::default() };
        crate::plugin::PluginHost::new(vec![m], Default::default(), env).commands().clone()
    }

    #[test]
    fn help_shows_plugin_commands_in_a_last_plugins_section_by_full_name() {
        let commands = plugin_table();
        let r = crate::keymap::load_keymap_from_str("", &commands);
        let rows = super::help_rows(&r.keymap.entries, &commands);
        let row = rows.iter().find(|r| r.action == "tidy.run").expect("plugin row");
        assert_eq!(row.section, "Plugins");
        assert_eq!(row.keys, "=");
        assert_eq!(row.desc, "Normalize the entry");
        assert_eq!(rows.last().unwrap().section, "Plugins");
    }

    #[test]
    fn help_falls_back_to_the_table_desc_when_a_user_rebinding_has_none() {
        let commands = plugin_table();
        let toml = "[normal.entries]\nprepend_keymap = [{ on = \"<C-t>\", run = \"tidy.quiet\" }]\n";
        let r = crate::keymap::load_keymap_from_str(toml, &commands);
        let rows = super::help_rows(&r.keymap.entries, &commands);
        let row = rows.iter().find(|r| r.action == "tidy.quiet").unwrap();
        assert_eq!(row.desc, "No menu");
    }

    #[test]
    fn context_menu_appends_only_menu_commands_after_the_builtins() {
        let commands = plugin_table();
        let items = super::context_menu_items(&commands);
        assert_eq!(items.len(), ContextMenuState::ITEMS.len() + 1);
        let (label, action) = items.last().unwrap();
        assert_eq!(label, "Normalize the entry");
        assert_eq!(*action, Action::Plugin(commands.find("tidy.run").unwrap()));
    }

    /// 팝업이 열려 있으면 뒤에 온 요청은 줄을 선다. 답이 가면 다음 것이 열린다.
    #[test]
    fn a_second_window_request_waits_until_the_first_is_answered() {
        use super::{open_or_queue, PluginUiState};
        use crate::plugin::protocol::UiRequest;
        use crate::plugin::rpc::Id;
        use std::collections::VecDeque;
        let mut q: VecDeque<(String, Id, UiRequest)> = Default::default();
        let mut open: Option<PluginUiState> = None;
        let pick = UiRequest::Pick { title: None, items: vec!["a".into()] };
        open_or_queue(&mut open, &mut q, "p1", 7, pick.clone());
        open_or_queue(&mut open, &mut q, "p2", 9, pick.clone());
        assert_eq!(open.as_ref().map(|s| (s.plugin.as_str(), s.id)), Some(("p1", 7)));
        assert_eq!(q.len(), 1);
        drop(open);
        let next = q.pop_front().unwrap();
        assert_eq!((next.0.as_str(), next.1), ("p2", 9));
    }

    /// commands/execute의 이름은 keymap.toml의 액션 이름과 같다.
    #[test]
    fn commands_execute_names_are_keymap_action_names() {
        use super::action_by_name;
        assert_eq!(action_by_name("export_menu"), Some(Action::ExportMenu));
        assert_eq!(action_by_name("nope"), None);
        assert_eq!(action_by_name("plugin"), None, "plugin commands are not addressable this way");
    }

    /// 행 오른쪽 조각은 남는 폭에 오른쪽 정렬, 왼쪽 내용은 지킨다.
    #[test]
    fn row_suffix_right_aligns_within_the_remaining_width() {
        use super::row_suffix;
        let s = row_suffix(30, "kim2025 ◆", &[("★ 312".into(), 6), ("3 refs".into(), 8)]);
        assert_eq!(s.chars().count(), 30 - "kim2025 ◆".chars().count());
        assert!(s.ends_with("★ 312  3 refs"), "{:?}", s);
        let s = row_suffix(12, "kim2025 ◆", &[("★ 312".into(), 6)]);
        assert_eq!(s, "", "no room: the plugin cell goes, the key stays");
    }

    /// 상태 조각은 (plugin, field)로 놓이고 빈 text는 조각을 지운다.
    #[test]
    fn status_set_replaces_and_empty_text_removes() {
        use super::apply_status_set;
        use crate::plugin::protocol::FieldValue;
        let mut segs: std::collections::BTreeMap<(String, String), FieldValue> = Default::default();
        apply_status_set(&mut segs, "git-sync", serde_json::json!({"field": "ahead", "text": "↑2 unpushed"}));
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[&("git-sync".to_string(), "ahead".to_string())].text, "↑2 unpushed");
        apply_status_set(&mut segs, "git-sync", serde_json::json!({"field": "ahead", "text": ""}));
        assert!(segs.is_empty());
    }
}
