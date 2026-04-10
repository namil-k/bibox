use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
    Frame, Terminal,
};
use std::io;

use crate::bibtex::{entries_to_bibtex, entry_to_filename};
use crate::config::Config;
use crate::models::Entry;
use crate::storage::{find_by_key_mut, load_db, save_db};

// ── State ────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
enum Panel {
    Collections,
    Entries,
    Preview,
}

#[derive(Clone, Copy, PartialEq)]
enum PreviewMode {
    Info,
    Note,
    Pdf,
}

impl PreviewMode {
    fn next(self) -> Self {
        match self {
            PreviewMode::Info => PreviewMode::Note,
            PreviewMode::Note => PreviewMode::Pdf,
            PreviewMode::Pdf => PreviewMode::Info,
        }
    }

    fn label(&self) -> &'static str {
        match self {
            PreviewMode::Info => "Info",
            PreviewMode::Note => "Note",
            PreviewMode::Pdf => "PDF",
        }
    }
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
    FilePicker(FilePickerContext),
    FetchPreview,
    SearchResultPicker,
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

#[derive(Clone, Copy, PartialEq)]
enum ExportScope {
    Selected,
    Collection,
    All,
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
    Delete(String),
    FetchPdf(String),
    OpenBrowser(String, String), // (citekey, url)
    FetchMetaByTitle(String, String), // (citekey, title)
}

enum FilePickerContext {
    AttachPdf(String),  // citekey
    BibExportDir,
    ExportDir,
    Home,
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
    // Note cache
    note_content: String,
    note_citekey: String,
    // Background tasks
    pending_editor: Option<std::path::PathBuf>,
    bg_result: Option<std::sync::mpsc::Receiver<Result<BgTaskResult>>>,
    bg_fetch_key: Option<String>,
    bg_meta_result: Option<std::sync::mpsc::Receiver<Result<(String, crate::crossref::Metadata)>>>,
    bg_search_result: Option<std::sync::mpsc::Receiver<Result<(String, Vec<crate::crossref::SearchResult>)>>>,
    fetch_preview: Option<FetchPreviewState>,
    search_picker: Option<SearchResultPickerState>,
    file_picker_state: Option<ratatree::FilePickerState>,
    spinner_tick: usize,
    // Sort
    sort_by: SortCriterion,
    sort_ascending: bool,
    sort_menu_index: usize,
    prev_sort_by: SortCriterion,
    prev_sort_ascending: bool,
    // Picker
    picker: Option<ChecklistPicker>,
    // Vim key buffer (for gg, {number}j etc.)
    key_buf: String,
    // Undo/Redo stacks (DB snapshots)
    undo_stack: Vec<Vec<Entry>>,
    redo_stack: Vec<Vec<Entry>>,
    // Multi-select
    selected_keys: std::collections::HashSet<String>,
    // Export menu state
    export_state: Option<ExportState>,
    // Settings state
    settings_idx: usize,
    git_status_cache: String,
    git_status_checked: bool,
    git_fetching: bool,
    git_fetch_result: std::sync::Arc<std::sync::Mutex<Option<String>>>,
    git_syncing: bool,
    git_sync_result: std::sync::Arc<std::sync::Mutex<Option<Result<String, String>>>>,
}

impl App {
    pub fn new(config: Config) -> Result<Self> {
        let db_path = crate::config::resolve_db_path(&config);
        let db = load_db(&db_path)?;
        let entries = db.entries;

        let mut col_set: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for e in &entries {
            for c in &e.collections { col_set.insert(c.clone()); }
        }
        let collections: Vec<String> = col_set.into_iter().collect();

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
            note_content: String::new(),
            note_citekey: String::new(),
            pending_editor: None,
            bg_result: None,
            bg_fetch_key: None,
            bg_meta_result: None,
            bg_search_result: None,
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
            key_buf: String::new(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            selected_keys: std::collections::HashSet::new(),
            export_state: None,
            settings_idx: 0,
            git_status_cache: String::new(),
            git_status_checked: false,
            git_fetching: false,
            git_fetch_result: std::sync::Arc::new(std::sync::Mutex::new(None)),
            git_syncing: false,
            git_sync_result: std::sync::Arc::new(std::sync::Mutex::new(None)),
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

    fn export_collection_bib(&self) -> String {
        let col = self.current_collection();
        let entries: Vec<&Entry> = self.entries.iter()
            .filter(|e| match col {
                None => true,
                Some(c) => e.collections.iter().any(|ec| ec == c),
            })
            .collect();
        let bib = entries_to_bibtex(&entries);
        let col_name = col.unwrap_or("all");
        let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S").to_string();
        let filename = format!("{}_{}.bib", col_name, timestamp);
        let _ = std::fs::write(&filename, &bib);
        filename
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
            self.entries.retain(|e| e.bibtex_key != key);
            let db_path = crate::config::resolve_db_path(&self.config);
            let mut db = load_db(&db_path)?;
            db.entries = self.entries.clone();
            save_db(&db, &db_path)?;
            self.rebuild_collections();
            self.apply_filters();
        }
        Ok(())
    }

    fn rebuild_collections(&mut self) {
        let mut col_set: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for e in &self.entries {
            for c in &e.collections { col_set.insert(c.clone()); }
        }
        self.collections = col_set.into_iter().collect();
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
            let db_path = crate::config::resolve_db_path(&self.config);
            let mut db = load_db(&db_path)?;
            db.entries = self.entries.clone();
            save_db(&db, &db_path)?;
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
            let db_path = crate::config::resolve_db_path(&self.config);
            let mut db = load_db(&db_path)?;
            db.entries = self.entries.clone();
            save_db(&db, &db_path)?;
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
        let db_path = crate::config::resolve_db_path(&self.config);
        let mut db = load_db(&db_path)?;
        db.entries = self.entries.clone();
        save_db(&db, &db_path)?;

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
        let db_path = crate::config::resolve_db_path(&self.config);
        let mut db = load_db(&db_path)?;
        db.entries = self.entries.clone();
        save_db(&db, &db_path)?;
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
        let db_path = crate::config::resolve_db_path(&self.config);
        let mut db = load_db(&db_path)?;
        db.entries = self.entries.clone();
        save_db(&db, &db_path)?;
        self.apply_filters();
        Ok(())
    }
}

// ── Drawing ──────────────────────────────────────────────────────────────────

fn draw(f: &mut Frame, app: &mut App) {
    let size = f.area();

    // Main layout: content | status bar
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
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

    draw_collections_panel(f, app, panels[0]);
    draw_entries_panel(f, app, panels[1]);
    draw_preview_panel(f, app, panels[2]);
    draw_status_bar(f, app, outer[1]);

    // File picker takes over full screen
    if let Mode::FilePicker(_) = &app.mode {
        if let Some(picker_state) = &mut app.file_picker_state {
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
        Mode::Help => draw_help_popup(f, size),
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
        _ => {}
    }
}

fn draw_collections_panel(f: &mut Frame, app: &App, area: Rect) {
    let focused = app.focus == Panel::Collections;
    let border_style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let mut items: Vec<ListItem> = vec![];

    // "All" item
    let all_count = app.entries.len();
    items.push(ListItem::new(format!("All ({})", all_count)));

    // Tree-style collection items: indent by depth, show only the last segment as label
    for col in &app.collections {
        let depth = col.matches('/').count();
        let label = col.split('/').last().unwrap_or(col.as_str());
        let prefix_str = format!("{}/", col);
        let count = app.entries.iter().filter(|e| {
            e.collections.iter().any(|c| c == col || c.starts_with(&prefix_str))
        }).count();
        let indent = "  ".repeat(depth);
        let connector = if depth > 0 { "└ " } else { "" };
        items.push(ListItem::new(format!("{}{}{} ({})", indent, connector, label, count)));
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(" Collections ");

    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");

    let mut state = app.col_list_state.clone();
    f.render_stateful_widget(list, area, &mut state);
}

fn draw_entries_panel(f: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focus == Panel::Entries;
    let border_style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
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
            Style::default().fg(Color::Yellow)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        let pad = " ".repeat(line_num.len());
        let is_selected = app.selected_keys.contains(&e.bibtex_key);
        let sel_mark = if is_selected { "✓ " } else { "  " };
        let sel_style = if is_selected { Style::default().fg(Color::Green) } else { Style::default() };

        let line1 = Line::from(vec![
            Span::styled(line_num, num_style),
            Span::styled(sel_mark, sel_style),
            Span::styled(e.bibtex_key.clone(), Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::styled(pdf_mark.to_string(), Style::default().fg(Color::Green)),
        ]);
        let line2 = Line::from(Span::raw(format!("{}  {}", pad, title)));
        let line3 = Line::from(Span::styled(
            format!("{}  {} · {}", pad, author, year),
            Style::default().fg(Color::DarkGray),
        ));

        let mut item = ListItem::new(Text::from(vec![line1, line2, line3]));
        if is_selected {
            item = item.style(Style::default().bg(Color::Rgb(30, 50, 30)));
        }
        item
    }).collect();

    let sel_count = app.selected_keys.len();
    let title = if !app.search_query.is_empty() {
        format!(" Search: {} ({}) ", app.search_query, app.filtered.len())
    } else if sel_count > 0 {
        format!(" Entries ({}) — {} selected ", app.filtered.len(), sel_count)
    } else {
        format!(" Entries ({}) ", app.filtered.len())
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(title);

    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");

    f.render_stateful_widget(list, area, &mut app.list_state);
}

fn draw_preview_panel(f: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focus == Panel::Preview;
    let border_style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    // Tab bar for preview modes
    let modes = [PreviewMode::Info, PreviewMode::Note, PreviewMode::Pdf];
    let tab_spans: Vec<Span> = modes.iter().map(|m| {
        if *m == app.preview_mode {
            Span::styled(
                format!(" {} ", m.label()),
                Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD),
            )
        } else {
            Span::styled(format!(" {} ", m.label()), Style::default().fg(Color::DarkGray))
        }
    }).collect();

    let mut title_spans = vec![Span::raw(" ")];
    for (i, s) in tab_spans.into_iter().enumerate() {
        title_spans.push(s);
        if i < modes.len() - 1 {
            title_spans.push(Span::styled(" │ ", Style::default().fg(Color::DarkGray)));
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
        PreviewMode::Pdf => draw_preview_pdf(f, app, inner),
    }
}

fn draw_preview_info(f: &mut Frame, app: &App, area: Rect) {
    let entry = match app.selected_entry() {
        Some(e) => e,
        None => {
            f.render_widget(Paragraph::new("No entry selected.").style(Style::default().fg(Color::DarkGray)), area);
            return;
        }
    };

    let mut lines: Vec<Line<'_>> = vec![];

    if let Some(t) = &entry.title {
        lines.push(Line::from(Span::styled(t.as_str(), Style::default().add_modifier(Modifier::BOLD))));
        lines.push(Line::from(""));
    }

    macro_rules! field {
        ($label:expr, $value:expr) => {
            lines.push(Line::from(vec![
                Span::styled(format!("{:<12}", $label), Style::default().fg(Color::Cyan)),
                Span::raw($value.to_string()),
            ]));
        };
    }

    field!("Key:", entry.bibtex_key.as_str());
    field!("Type:", entry.entry_type.to_string());
    if !entry.author.is_empty() { field!("Author:", entry.author.join("; ")); }
    if let Some(y) = entry.year { field!("Year:", y.to_string()); }
    if let Some(j) = &entry.journal { field!("Journal:", j.as_str()); }
    if let Some(bt) = &entry.booktitle { field!("Booktitle:", bt.as_str()); }
    if let Some(p) = &entry.publisher { field!("Publisher:", p.as_str()); }
    if let Some(doi) = &entry.doi { field!("DOI:", doi.as_str()); }
    if let Some(url) = &entry.url { field!("URL:", url.as_str()); }
    if !entry.tags.is_empty() { field!("Tags:", entry.tags.join(", ")); }
    if !entry.collections.is_empty() {
        field!("Collections:", entry.collections.join(", "));
    } else {
        lines.push(Line::from(vec![
            Span::styled(format!("{:<12}", "Collections:"), Style::default().fg(Color::Cyan)),
            Span::styled("(none)", Style::default().fg(Color::DarkGray)),
        ]));
    }
    if let Some(fp) = &entry.file_path {
        field!("File:", fp.as_str());
    } else {
        lines.push(Line::from(vec![
            Span::styled(format!("{:<12}", "File:"), Style::default().fg(Color::Cyan)),
            Span::styled("No PDF", Style::default().fg(Color::DarkGray)),
        ]));
    }
    if let Some(ref abs) = entry.abstract_text {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("Abstract:", Style::default().fg(Color::Cyan))));
        // Word-wrap abstract text (simple approach: just push as-is, TUI wraps)
        lines.push(Line::from(Span::styled(abs.as_str(), Style::default().fg(Color::DarkGray))));
    }

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
            Paragraph::new("No entry selected.").style(Style::default().fg(Color::DarkGray)),
            area,
        );
        return;
    }

    let lines = render_markdown_to_lines(&app.note_content);

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
                    Some(HeadingLevel::H1) => Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                    Some(HeadingLevel::H2) => Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                    _ => Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
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
                let style = Style::default().fg(Color::Green).bg(Color::Rgb(40, 40, 40));
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
                    let style = Style::default().fg(Color::White).bg(Color::Rgb(40, 40, 40));
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
                                Style::default().fg(Color::DarkGray),
                            ));
                            list_index += 1;
                        } else {
                            current_spans.push(Span::styled(
                                format!("{}  • ", prefix),
                                Style::default().fg(Color::DarkGray),
                            ));
                        }
                        list_item_started = false;
                    } else if in_blockquote && current_spans.is_empty() {
                        current_spans.push(Span::styled(
                            "│ ".to_string(),
                            Style::default().fg(Color::Cyan),
                        ));
                    }

                    let mut style = Style::default();
                    if bold { style = style.add_modifier(Modifier::BOLD); }
                    if italic { style = style.add_modifier(Modifier::ITALIC); }
                    if in_blockquote { style = style.fg(Color::DarkGray).add_modifier(Modifier::ITALIC); }
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
                    Style::default().fg(Color::DarkGray),
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
            Style::default().fg(Color::DarkGray),
        )));
    }

    lines
}

fn draw_preview_pdf(f: &mut Frame, app: &App, area: Rect) {
    let entry = match app.selected_entry() {
        Some(e) => e,
        None => {
            f.render_widget(Paragraph::new("No entry selected.").style(Style::default().fg(Color::DarkGray)), area);
            return;
        }
    };

    if entry.file_path.is_none() {
        f.render_widget(
            Paragraph::new("No PDF attached.\nPress o to fetch or open.").style(Style::default().fg(Color::DarkGray)),
            area,
        );
        return;
    }

    // PDF preview placeholder — Kitty protocol rendering to be implemented
    let fp = entry.file_path.as_ref().unwrap();
    let full_path = app.config.bibox_dir.join(fp);

    // Try pdftotext fallback for now
    let text = std::process::Command::new("pdftotext")
        .args(["-l", "1", "-layout", &full_path.to_string_lossy(), "-"])
        .output()
        .ok()
        .and_then(|o| if o.status.success() { String::from_utf8(o.stdout).ok() } else { None });

    match text {
        Some(content) => {
            let lines: Vec<Line> = content.lines()
                .map(|l| Line::from(l.to_string()))
                .collect();
            let p = Paragraph::new(lines)
                .scroll((app.preview_scroll, 0))
                .wrap(ratatui::widgets::Wrap { trim: false });
            f.render_widget(p, area);
        }
        None => {
            f.render_widget(
                Paragraph::new(format!("PDF: {}\n\nInstall pdftotext for text preview:\n  brew install poppler", fp))
                    .style(Style::default().fg(Color::DarkGray)),
                area,
            );
        }
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
        _ => {
            let panel_hint = match app.focus {
                Panel::Collections => "j/k navigate  l→entries",
                Panel::Entries => "h←collections  l→preview  j/k navigate",
                Panel::Preview => "h←entries  Tab switch mode  j/k scroll",
            };
            format!(
                "{}  │  / search  s sort  o open  w web  e export  d delete  c collect  t tag  N note  , settings  ? help  q quit",
                panel_hint
            )
        }
    };
    let status_widget = Paragraph::new(status).style(Style::default().fg(Color::DarkGray));
    f.render_widget(status_widget, area);
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

fn draw_confirm_popup(f: &mut Frame, msg: &str, area: Rect) {
    let popup_area = centered_rect(60, 5, area);
    f.render_widget(Clear, popup_area);
    let text = Paragraph::new(msg)
        .block(Block::default().borders(Borders::ALL).title(" Confirm "))
        .style(Style::default().fg(Color::Red));
    f.render_widget(text, popup_area);
}

fn draw_message_popup(f: &mut Frame, msg: &str, area: Rect) {
    let popup_area = centered_rect(60, 5, area);
    f.render_widget(Clear, popup_area);
    let text = Paragraph::new(msg)
        .block(Block::default().borders(Borders::ALL).title(" Info "))
        .style(Style::default().fg(Color::Green));
    f.render_widget(text, popup_area);
}

fn draw_help_popup(f: &mut Frame, area: Rect) {
    let popup_area = centered_rect(80, 28, area);
    f.render_widget(Clear, popup_area);

    let help_text = vec![
        Line::from(Span::styled("bibox — Keyboard Shortcuts", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))),
        Line::from(""),
        Line::from(Span::styled("Navigation", Style::default().fg(Color::Cyan))),
        Line::from("  h/←   Focus left panel    l/→   Focus right panel"),
        Line::from("  j/↓   Move down           k/↑   Move up"),
        Line::from("  gg    Jump to top          G     Jump to bottom"),
        Line::from("  H/M/L Screen top/mid/bot   {n}j  Move n lines"),
        Line::from("  C-d   Half page down       C-u   Half page up"),
        Line::from("  Tab   Switch preview mode (Info / Note / PDF)"),
        Line::from(""),
        Line::from(Span::styled("Selection", Style::default().fg(Color::Cyan))),
        Line::from("  Space Toggle select         V     Select/deselect all"),
        Line::from("  Esc   Clear selection"),
        Line::from(""),
        Line::from(Span::styled("Actions", Style::default().fg(Color::Cyan))),
        Line::from("  f     Fetch metadata        o     Open PDF / fetch"),
        Line::from("  y     Copy citekey          w     Open web page"),
        Line::from("  A     Attach PDF            e     Export menu"),
        Line::from("  d     Delete entry          N     Edit note ($EDITOR)"),
        Line::from(""),
        Line::from(Span::styled("Edit", Style::default().fg(Color::Cyan))),
        Line::from("  c     Manage collections    t     Edit tags"),
        Line::from("  s     Sort menu             C-z   Undo    C-y  Redo"),
        Line::from(""),
        Line::from(Span::styled("Other", Style::default().fg(Color::Cyan))),
        Line::from("  /     Search                q     Quit"),
        Line::from("  ,     Settings              ?     This help"),
        Line::from(""),
        Line::from(Span::styled("Press ? or Esc to close", Style::default().fg(Color::DarkGray))),
    ];

    let popup = Paragraph::new(help_text)
        .block(Block::default().borders(Borders::ALL).title(" Help "))
        .wrap(ratatui::widgets::Wrap { trim: false });
    f.render_widget(popup, popup_area);
}

fn draw_sort_popup(f: &mut Frame, app: &App, area: Rect) {
    let popup_area = centered_rect(55, 14, area);
    f.render_widget(Clear, popup_area);
    let criteria = SortCriterion::all();
    let mut lines = vec![
        Line::from(Span::styled("Sort by:", Style::default().fg(Color::Yellow))),
        Line::from(""),
    ];
    for (i, c) in criteria.iter().enumerate() {
        let selected = *c == app.sort_by;
        let arrow = if i == app.sort_menu_index { "▶ " } else { "  " };
        let dir = if selected { if app.sort_ascending { "↑ asc" } else { "↓ desc" } } else { "     " };
        let style = if selected { Style::default().fg(Color::Cyan) } else { Style::default() };
        lines.push(Line::from(vec![
            Span::styled(arrow, Style::default().fg(Color::Yellow)),
            Span::styled(format!("{:<12}", c.label()), style),
            Span::styled(dir, Style::default().fg(Color::DarkGray)),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("↑↓ select  Enter apply  Space toggle ↑↓  Esc cancel", Style::default().fg(Color::DarkGray))));
    let popup = Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(" Sort "));
    f.render_widget(popup, popup_area);
}

fn draw_checklist_popup(f: &mut Frame, picker: &ChecklistPicker, area: Rect) {
    let item_count = picker.items.len() + 3;
    let height = (item_count as u16 + 4).min(20);
    let popup_area = centered_rect(60, height, area);
    f.render_widget(Clear, popup_area);

    let mut lines = vec![
        Line::from(Span::styled(&picker.title, Style::default().fg(Color::Yellow))),
        Line::from(""),
    ];
    for (i, (name, checked)) in picker.items.iter().enumerate() {
        let arrow = if i == picker.index { "▶ " } else { "  " };
        let check = if *checked { "[x]" } else { "[ ]" };
        let check_style = if *checked { Style::default().fg(Color::Green) } else { Style::default().fg(Color::DarkGray) };
        lines.push(Line::from(vec![
            Span::styled(arrow, Style::default().fg(Color::Yellow)),
            Span::styled(format!("{} ", check), check_style),
            Span::raw(name.as_str()),
        ]));
    }
    lines.push(Line::from(Span::styled("  ─────────────────", Style::default().fg(Color::DarkGray))));
    let new_arrow = if picker.is_on_new_item() { "▶ " } else { "  " };
    if let Some(ref input) = picker.new_item_input {
        lines.push(Line::from(vec![
            Span::styled(new_arrow, Style::default().fg(Color::Yellow)),
            Span::styled(format!("{}▏", input), Style::default().fg(Color::Cyan)),
        ]));
    } else {
        lines.push(Line::from(vec![
            Span::styled(new_arrow, Style::default().fg(Color::Yellow)),
            Span::styled(&picker.new_item_label, Style::default().fg(Color::Cyan)),
        ]));
    }
    lines.push(Line::from(""));
    let footer = if picker.in_input_mode() { "Enter confirm  Esc cancel" } else { "↑↓ navigate  Space toggle  Enter done  Esc cancel" };
    lines.push(Line::from(Span::styled(footer, Style::default().fg(Color::DarkGray))));
    let popup = Paragraph::new(lines).block(Block::default().borders(Borders::ALL));
    f.render_widget(popup, popup_area);
}

fn draw_export_popup(f: &mut Frame, es: &ExportState, area: Rect) {
    let height = (es.scope_options.len() + 10) as u16;
    let popup_area = centered_rect(60, height.min(18), area);
    f.render_widget(Clear, popup_area);

    let formats = [ExportFormat::BibTeX, ExportFormat::Yaml, ExportFormat::Ris];
    let mut lines = vec![
        Line::from(Span::styled("Scope:", Style::default().fg(Color::Yellow))),
    ];
    for (i, (_, label)) in es.scope_options.iter().enumerate() {
        let arrow = if es.section == 0 && i == es.scope_idx { "▶ " } else { "  " };
        let style = if i == es.scope_idx { Style::default().fg(Color::Cyan) } else { Style::default() };
        lines.push(Line::from(vec![
            Span::styled(arrow, Style::default().fg(Color::Yellow)),
            Span::styled(label.as_str(), style),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("Format:", Style::default().fg(Color::Yellow))));
    for (i, fmt) in formats.iter().enumerate() {
        let arrow = if es.section == 1 && i == es.format_idx { "▶ " } else { "  " };
        let style = if i == es.format_idx { Style::default().fg(Color::Cyan) } else { Style::default() };
        lines.push(Line::from(vec![
            Span::styled(arrow, Style::default().fg(Color::Yellow)),
            Span::styled(fmt.label(), style),
        ]));
    }
    lines.push(Line::from(""));
    let pdf_arrow = if es.section == 2 { "▶ " } else { "  " };
    let pdf_check = if es.include_pdf { "[x]" } else { "[ ]" };
    let pdf_style = if es.include_pdf { Style::default().fg(Color::Green) } else { Style::default().fg(Color::DarkGray) };
    lines.push(Line::from(vec![
        Span::styled(pdf_arrow, Style::default().fg(Color::Yellow)),
        Span::styled(pdf_check, pdf_style),
        Span::raw(" Include PDFs"),
    ]));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "↑↓ navigate  Tab section  Space toggle  Enter export  Esc cancel",
        Style::default().fg(Color::DarkGray),
    )));

    let popup = Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(" Export "));
    f.render_widget(popup, popup_area);
}

fn get_git_status(home: &std::path::Path) -> String {
    // Check if it's a git repo
    let is_git = std::process::Command::new("git")
        .args(["-C", &home.to_string_lossy(), "rev-parse", "--is-inside-work-tree"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !is_git { return "not a git repo".into(); }

    // Check for remote
    let remote = std::process::Command::new("git")
        .args(["-C", &home.to_string_lossy(), "remote"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_default();
    if remote.trim().is_empty() { return "no remote".into(); }

    // Check for uncommitted changes
    let status = std::process::Command::new("git")
        .args(["-C", &home.to_string_lossy(), "status", "--porcelain"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_default();
    if !status.trim().is_empty() {
        return format!("{} ● uncommitted changes", remote.trim());
    }

    // Fetch and check ahead/behind
    let _ = std::process::Command::new("git")
        .args(["-C", &home.to_string_lossy(), "fetch", "--quiet"])
        .output();
    let behind = std::process::Command::new("git")
        .args(["-C", &home.to_string_lossy(), "rev-list", "--count", "HEAD..@{upstream}"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse::<usize>().ok())
        .unwrap_or(0);

    let branch = std::process::Command::new("git")
        .args(["-C", &home.to_string_lossy(), "branch", "--show-current"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_else(|| "master".into());

    if behind > 0 {
        format!("{}/{} ⚠ {} behind", remote.trim(), branch.trim(), behind)
    } else {
        format!("{}/{} ✓ up to date", remote.trim(), branch.trim())
    }
}

fn draw_settings_popup(f: &mut Frame, app: &App, area: Rect) {
    use crate::config::LineNumbers;
    let popup_area = centered_rect(70, 20, area);
    f.render_widget(Clear, popup_area);

    let ln_label = match app.config.line_numbers {
        LineNumbers::Absolute => "absolute",
        LineNumbers::Relative => "relative",
        LineNumbers::None => "none",
    };
    let ratio = app.config.panel_ratio;
    let bib_dir = app.config.bib_export_dir.display().to_string();
    let exp_dir = app.config.export_dir.display().to_string();

    let home_label = match &app.config.home {
        Some(h) => h.display().to_string(),
        None => "(not set — use `bibox init <path>`)".into(),
    };

    const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    let git_label = if app.git_fetching {
        let frame = SPINNER_FRAMES[(app.spinner_tick as usize / 3) % SPINNER_FRAMES.len()];
        format!("{} checking...", frame)
    } else if app.git_syncing {
        let frame = SPINNER_FRAMES[(app.spinner_tick as usize / 3) % SPINNER_FRAMES.len()];
        format!("{} syncing...", frame)
    } else {
        app.git_status_cache.clone()
    };

    let ck_fmt = &app.config.citekey_format;

    let mut items: Vec<(String, bool)> = vec![
        (format!("Line numbers     [{}]", ln_label), true),
        (format!("Panel ratio      [{}, {}, {}]", ratio[0], ratio[1], ratio[2]), true),
        (format!("Bib export dir   [{}]", bib_dir), true),
        (format!("Export dir       [{}]", exp_dir), true),
        (format!("Citekey format   [{}]", ck_fmt), true),
    ];
    // separator + read-only items
    let readonly_start = items.len();
    items.push((format!("Home             {}", home_label), false));
    items.push((format!("Git              {}", git_label), false));

    let mut lines = vec![
        Line::from(Span::styled("Settings", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))),
        Line::from(""),
    ];
    for (i, (label, editable)) in items.iter().enumerate() {
        if i == readonly_start {
            lines.push(Line::from(Span::styled("  ─────────────────────────────", Style::default().fg(Color::DarkGray))));
        }
        let arrow = if i == app.settings_idx { "▶ " } else { "  " };
        let style = if !editable {
            Style::default().fg(Color::DarkGray)
        } else if i == app.settings_idx {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default()
        };
        lines.push(Line::from(vec![
            Span::styled(arrow, Style::default().fg(Color::Yellow)),
            Span::styled(label.as_str(), style),
        ]));
    }

    // Show git hint when on Git row
    if app.settings_idx == readonly_start + 1 && app.config.home.is_some() {
        let hint = if app.git_status_checked {
            "           [Enter: sync (pull → commit → push)]"
        } else {
            "           [Enter: check status]"
        };
        lines.push(Line::from(Span::styled(hint, Style::default().fg(Color::Cyan))));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "↑↓ navigate  ←→ change value  Enter save/sync  Esc cancel",
        Style::default().fg(Color::DarkGray),
    )));

    let popup = Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(" Settings "));
    f.render_widget(popup, popup_area);
}

// ── Event loop ───────────────────────────────────────────────────────────────

fn handle_key(app: &mut App, key: crossterm::event::KeyEvent) -> Result<bool> {
    match &app.mode {
        Mode::Normal => handle_normal(app, key),
        Mode::Search => handle_search(app, key),
        Mode::Confirm(_) => handle_confirm(app, key),
        Mode::Message(_) => { app.mode = Mode::Normal; Ok(false) }
        Mode::Loading(_) => {
            if key.code == KeyCode::Esc { app.bg_result = None; app.mode = Mode::Normal; }
            Ok(false)
        }
        Mode::Help => handle_help(app, key),
        Mode::SortMenu => handle_sort_menu(app, key),
        Mode::CollectionPicker => handle_picker(app, key, false),
        Mode::TagEditor => handle_picker(app, key, true),
        Mode::ExportMenu => handle_export_menu(app, key),
        Mode::Settings => handle_settings(app, key),
        Mode::FilePicker(_) => handle_file_picker(app, key),
        Mode::FetchPreview => handle_fetch_preview(app, key),
        Mode::SearchResultPicker => handle_search_result_picker(app, key),
    }
}

fn handle_normal(app: &mut App, key: crossterm::event::KeyEvent) -> Result<bool> {
    // ── Key buffer for vim motions (digits + g) ──
    match key.code {
        KeyCode::Char(c @ '0'..='9') => {
            // Only buffer digits if we already have digits or it's not '0' at start
            if !app.key_buf.is_empty() || c != '0' {
                app.key_buf.push(c);
                return Ok(false);
            }
        }
        KeyCode::Char('g') => {
            if app.key_buf == "g" {
                // gg → go to top
                match app.focus {
                    Panel::Collections => { app.col_list_state.select(Some(0)); app.list_state.select(Some(0)); app.apply_filters(); }
                    Panel::Entries => { app.list_state.select(Some(0)); app.update_preview(); }
                    Panel::Preview => { app.preview_scroll = 0; }
                }
                app.key_buf.clear();
                return Ok(false);
            } else if app.key_buf.is_empty() {
                app.key_buf.push('g');
                return Ok(false);
            }
        }
        _ => {}
    }

    // Parse count from buffer
    let count: usize = app.key_buf.chars().take_while(|c| c.is_ascii_digit())
        .collect::<String>().parse().unwrap_or(1);
    app.key_buf.clear();

    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => return Ok(true),
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => return Ok(true),
        KeyCode::Char('z') if key.modifiers.contains(KeyModifiers::CONTROL) => { app.undo()?; }
        KeyCode::Char('y') if key.modifiers.contains(KeyModifiers::CONTROL) => { app.redo()?; }

        // Panel navigation
        KeyCode::Char('h') | KeyCode::Left => {
            app.focus = match app.focus {
                Panel::Collections => Panel::Collections,
                Panel::Entries => Panel::Collections,
                Panel::Preview => Panel::Entries,
            };
        }
        KeyCode::Char('l') | KeyCode::Right => {
            app.focus = match app.focus {
                Panel::Collections => Panel::Entries,
                Panel::Entries => Panel::Preview,
                Panel::Preview => Panel::Preview,
            };
        }

        // Vertical movement with count
        KeyCode::Char('j') | KeyCode::Down => {
            for _ in 0..count {
                match app.focus {
                    Panel::Collections => app.move_col_down(),
                    Panel::Entries => app.move_entry_down(),
                    Panel::Preview => { app.preview_scroll += 1; }
                }
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            for _ in 0..count {
                match app.focus {
                    Panel::Collections => app.move_col_up(),
                    Panel::Entries => app.move_entry_up(),
                    Panel::Preview => { app.preview_scroll = app.preview_scroll.saturating_sub(1); }
                }
            }
        }

        // G → go to bottom
        KeyCode::Char('G') => {
            match app.focus {
                Panel::Collections => {
                    let last = app.col_count().saturating_sub(1);
                    app.col_list_state.select(Some(last));
                    app.list_state.select(Some(0));
                    app.apply_filters();
                }
                Panel::Entries => {
                    if !app.filtered.is_empty() {
                        app.list_state.select(Some(app.filtered.len() - 1));
                        app.update_preview();
                    }
                }
                Panel::Preview => { app.preview_scroll = u16::MAX; }
            }
        }

        // H/M/L — screen-relative jumps (entries panel only)
        KeyCode::Char('H') if app.focus == Panel::Entries => {
            // Jump to first visible item (approximate: just go to current - half_page)
            let visible_height = 10; // approximate
            let cur = app.list_state.selected().unwrap_or(0);
            let top = cur.saturating_sub(visible_height);
            app.list_state.select(Some(top));
            app.update_preview();
        }
        KeyCode::Char('M') if app.focus == Panel::Entries => {
            // Middle of list
            if !app.filtered.is_empty() {
                app.list_state.select(Some(app.filtered.len() / 2));
                app.update_preview();
            }
        }
        KeyCode::Char('L') if app.focus == Panel::Entries => {
            if !app.filtered.is_empty() {
                app.list_state.select(Some(app.filtered.len() - 1));
                app.update_preview();
            }
        }

        // Ctrl+d/u — half page
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            match app.focus {
                Panel::Entries => { for _ in 0..10 { app.move_entry_down(); } }
                Panel::Preview => { app.preview_scroll += 10; }
                Panel::Collections => { for _ in 0..5 { app.move_col_down(); } }
            }
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            match app.focus {
                Panel::Entries => { for _ in 0..10 { app.move_entry_up(); } }
                Panel::Preview => { app.preview_scroll = app.preview_scroll.saturating_sub(10); }
                Panel::Collections => { for _ in 0..5 { app.move_col_up(); } }
            }
        }

        // Space — toggle selection
        KeyCode::Char(' ') if app.focus == Panel::Entries => {
            if let Some(entry) = app.selected_entry() {
                let key = entry.bibtex_key.clone();
                if app.selected_keys.contains(&key) {
                    app.selected_keys.remove(&key);
                } else {
                    app.selected_keys.insert(key);
                }
                app.move_entry_down(); // move to next after toggle
            }
        }

        // V — select/deselect all visible
        KeyCode::Char('V') if app.focus == Panel::Entries => {
            let visible_keys: Vec<String> = app.filtered.iter()
                .map(|&idx| app.entries[idx].bibtex_key.clone()).collect();
            let all_selected = visible_keys.iter().all(|k| app.selected_keys.contains(k));
            if all_selected {
                for k in &visible_keys { app.selected_keys.remove(k); }
            } else {
                for k in visible_keys { app.selected_keys.insert(k); }
            }
        }

        // Esc — clear selection
        KeyCode::Esc => {
            if !app.selected_keys.is_empty() {
                app.selected_keys.clear();
            }
        }

        // Preview mode switch
        KeyCode::Tab => {
            app.preview_mode = app.preview_mode.next();
            app.preview_scroll = 0;
            if app.preview_mode == PreviewMode::Note { app.load_note_for_preview(); }
        }

        // Search (context-aware: collections panel vs entries panel)
        KeyCode::Char('/') => {
            if app.focus == Panel::Collections {
                app.col_search_query.clear();
            }
            app.mode = Mode::Search;
        }

        // Copy citekey
        KeyCode::Char('y') => {
            if let Some(entry) = app.selected_entry() {
                let bkey = entry.bibtex_key.clone();
                if let Ok(mut ctx) = arboard::Clipboard::new() { let _ = ctx.set_text(&bkey); }
                app.mode = Mode::Message(format!("Copied: {}", bkey));
            }
        }

        // Open PDF
        KeyCode::Char('o') => {
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

        // Open web page
        KeyCode::Char('w') => {
            if let Some(entry) = app.selected_entry() {
                let url = if let Some(ref doi) = entry.doi {
                    Some(format!("https://doi.org/{}", doi))
                } else if let Some(ref u) = entry.url {
                    Some(u.clone())
                } else {
                    None
                };
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

        // Export menu
        KeyCode::Char('f') => {
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
        KeyCode::Char('e') => {
            let mut scope_options = vec![];
            if !app.selected_keys.is_empty() {
                scope_options.push((ExportScope::Selected, format!("{} selected entries", app.selected_keys.len())));
            }
            if let Some(col) = app.current_collection() {
                let count = app.filtered.len();
                scope_options.push((ExportScope::Collection, format!("{} collection ({})", col, count)));
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

        // Delete
        KeyCode::Char('d') => {
            if let Some(entry) = app.selected_entry() {
                let bkey = entry.bibtex_key.clone();
                app.mode = Mode::Confirm(ConfirmAction::Delete(bkey));
            }
        }

        KeyCode::Char('?') => { app.mode = Mode::Help; }

        // Edit note in $EDITOR
        KeyCode::Char('N') => {
            if app.selected_entry().is_some() { return open_note_editor(app); }
            else { app.mode = Mode::Message("No entry selected.".into()); }
        }

        KeyCode::Char('s') => {
            app.prev_sort_by = app.sort_by;
            app.prev_sort_ascending = app.sort_ascending;
            app.sort_menu_index = SortCriterion::all().iter().position(|c| *c == app.sort_by).unwrap_or(0);
            app.mode = Mode::SortMenu;
        }

        KeyCode::Char('c') => {
            if !app.selected_keys.is_empty() {
                app.open_collection_picker_multi();
            } else if app.selected_entry().is_some() {
                app.open_collection_picker();
            } else {
                app.mode = Mode::Message("No entry selected.".into());
            }
        }

        KeyCode::Char('t') => {
            if app.selected_entry().is_some() { app.open_tag_editor(); }
            else { app.mode = Mode::Message("No entry selected.".into()); }
        }

        // Attach PDF
        KeyCode::Char('A') => {
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

        // Settings
        KeyCode::Char(',') => {
            app.settings_idx = 0;
            app.git_status_cache = "Press Enter to check".into();
            app.git_status_checked = false;
            app.mode = Mode::Settings;
        }

        KeyCode::Enter => {}

        _ => {}
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
        let name = if i == 0 { "all" } else { &app.collections[i - 1] };
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
                        format!("Opened in browser. Download the PDF, then use:\nbibox edit <key> --attach-pdf ~/Downloads/<file>.pdf")
                    );
                }
                _ => {}
            }
        }
        KeyCode::Char('n') | KeyCode::Esc => { app.mode = Mode::Normal; }
        _ => {}
    }
    Ok(false)
}

fn handle_help(app: &mut App, key: crossterm::event::KeyEvent) -> Result<bool> {
    match key.code {
        KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') => { app.mode = Mode::Normal; }
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
            let selected = criteria[app.sort_menu_index];
            if selected == app.sort_by { app.sort_ascending = !app.sort_ascending; }
            else { app.sort_by = selected; }
        }
        KeyCode::Enter => {
            let new_criterion = criteria[app.sort_menu_index];
            if new_criterion != app.sort_by { app.sort_ascending = new_criterion.default_ascending(); }
            app.sort_by = new_criterion;
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
                    0 => { if es.scope_idx < es.scope_options.len() - 1 { es.scope_idx += 1; } }
                    1 => { if es.format_idx < formats.len() - 1 { es.format_idx += 1; } }
                    _ => {}
                }
            }
            KeyCode::Char(' ') => {
                if es.section == 2 { es.include_pdf = !es.include_pdf; }
            }
            KeyCode::Enter => {
                let scope = es.scope_options[es.scope_idx].0;
                let format = formats[es.format_idx];
                let include_pdf = es.include_pdf;
                let col = app.current_collection().map(|s| s.to_string());

                // Collect entries based on scope
                let keys: Vec<String> = match scope {
                    ExportScope::Selected => app.selected_keys.iter().cloned().collect(),
                    ExportScope::Collection => {
                        app.filtered.iter().map(|&i| app.entries[i].bibtex_key.clone()).collect()
                    }
                    ExportScope::All => app.entries.iter().map(|e| e.bibtex_key.clone()).collect(),
                };

                let col_name = match scope {
                    ExportScope::Selected => "selected".to_string(),
                    ExportScope::Collection => col.unwrap_or_else(|| "all".to_string()),
                    ExportScope::All => "all".to_string(),
                };

                app.export_state = None;
                app.mode = Mode::Normal;

                // Run export via cmd_export
                let result = crate::commands::cmd_export(
                    keys, None, None, false, None, None, false, include_pdf, false,
                    format.ext().to_string(), false, &app.config,
                );
                match result {
                    Ok(()) => { app.mode = Mode::Message("Export complete.".into()); }
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
    use crate::config::LineNumbers;
    let num_settings: usize = 7; // 0-3 editable, 4 citekey format, 5 home (ro), 6 git

    let download_dir = dirs::download_dir()
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from(".")));
    let home_dir = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    let cwd = std::path::PathBuf::from(".");

    let dir_presets: Vec<std::path::PathBuf> = vec![
        cwd.clone(),
        download_dir.clone(),
        home_dir.join("Documents"),
        home_dir.join("Desktop"),
    ];

    match key.code {
        KeyCode::Up | KeyCode::Char('k') => {
            if app.settings_idx > 0 { app.settings_idx -= 1; }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if app.settings_idx < num_settings - 1 { app.settings_idx += 1; }
        }
        KeyCode::Right | KeyCode::Char('l') => {
            match app.settings_idx {
                0 => {
                    app.config.line_numbers = match app.config.line_numbers {
                        LineNumbers::Absolute => LineNumbers::Relative,
                        LineNumbers::Relative => LineNumbers::None,
                        LineNumbers::None => LineNumbers::Absolute,
                    };
                }
                1 => {
                    app.config.panel_ratio = match app.config.panel_ratio {
                        [2, 4, 4] => [1, 5, 4],
                        [1, 5, 4] => [2, 3, 5],
                        [2, 3, 5] => [1, 4, 5],
                        [1, 4, 5] => [3, 4, 3],
                        _ => [2, 4, 4],
                    };
                }
                2 => {
                    let cur = &app.config.bib_export_dir;
                    let pos = dir_presets.iter().position(|d| d == cur).unwrap_or(0);
                    app.config.bib_export_dir = dir_presets[(pos + 1) % dir_presets.len()].clone();
                }
                3 => {
                    let cur = &app.config.export_dir;
                    let pos = dir_presets.iter().position(|d| d == cur).unwrap_or(0);
                    app.config.export_dir = dir_presets[(pos + 1) % dir_presets.len()].clone();
                }
                4 => {
                    let presets = crate::config::CITEKEY_PRESETS;
                    let pos = presets.iter().position(|p| *p == app.config.citekey_format).unwrap_or(0);
                    app.config.citekey_format = presets[(pos + 1) % presets.len()].to_string();
                }
                _ => {} // 5,6 are read-only
            }
        }
        KeyCode::Left | KeyCode::Char('h') => {
            match app.settings_idx {
                0 => {
                    app.config.line_numbers = match app.config.line_numbers {
                        LineNumbers::Absolute => LineNumbers::None,
                        LineNumbers::Relative => LineNumbers::Absolute,
                        LineNumbers::None => LineNumbers::Relative,
                    };
                }
                1 => {
                    app.config.panel_ratio = match app.config.panel_ratio {
                        [2, 4, 4] => [3, 4, 3],
                        [3, 4, 3] => [1, 4, 5],
                        [1, 4, 5] => [2, 3, 5],
                        [2, 3, 5] => [1, 5, 4],
                        _ => [2, 4, 4],
                    };
                }
                2 => {
                    let cur = &app.config.bib_export_dir;
                    let pos = dir_presets.iter().position(|d| d == cur).unwrap_or(0);
                    let prev = if pos == 0 { dir_presets.len() - 1 } else { pos - 1 };
                    app.config.bib_export_dir = dir_presets[prev].clone();
                }
                3 => {
                    let cur = &app.config.export_dir;
                    let pos = dir_presets.iter().position(|d| d == cur).unwrap_or(0);
                    let prev = if pos == 0 { dir_presets.len() - 1 } else { pos - 1 };
                    app.config.export_dir = dir_presets[prev].clone();
                }
                4 => {
                    let presets = crate::config::CITEKEY_PRESETS;
                    let pos = presets.iter().position(|p| *p == app.config.citekey_format).unwrap_or(0);
                    let prev = if pos == 0 { presets.len() - 1 } else { pos - 1 };
                    app.config.citekey_format = presets[prev].to_string();
                }
                _ => {}
            }
        }
        KeyCode::Enter => {
            // Dir picker for path settings (2=bib_export_dir, 3=export_dir, 5=home)
            if app.settings_idx == 2 {
                let start = app.config.bib_export_dir.clone();
                app.file_picker_state = Some(
                    ratatree::FilePickerState::builder()
                        .start_dir(start)
                        .mode(ratatree::PickerMode::DirsOnly)
                        .build()
                );
                app.mode = Mode::FilePicker(FilePickerContext::BibExportDir);
                return Ok(false);
            } else if app.settings_idx == 3 {
                let start = app.config.export_dir.clone();
                app.file_picker_state = Some(
                    ratatree::FilePickerState::builder()
                        .start_dir(start)
                        .mode(ratatree::PickerMode::DirsOnly)
                        .build()
                );
                app.mode = Mode::FilePicker(FilePickerContext::ExportDir);
                return Ok(false);
            } else if app.settings_idx == 5 {
                let start = app.config.home.clone()
                    .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from(".")));
                app.file_picker_state = Some(
                    ratatree::FilePickerState::builder()
                        .start_dir(start)
                        .mode(ratatree::PickerMode::DirsOnly)
                        .build()
                );
                app.mode = Mode::FilePicker(FilePickerContext::Home);
                return Ok(false);
            } else if app.settings_idx == 6 {
                if let Some(ref home) = app.config.home {
                    let home = crate::config::expand_tilde(home);
                    if !app.git_status_checked {
                        // First Enter: check status in background
                        app.git_status_cache = String::new();
                        app.git_fetching = true;
                        let result_slot = std::sync::Arc::clone(&app.git_fetch_result);
                        let home_clone = home.clone();
                        std::thread::spawn(move || {
                            let status = get_git_status(&home_clone);
                            if let Ok(mut slot) = result_slot.lock() {
                                *slot = Some(status);
                            }
                        });
                        app.git_status_checked = true;
                    } else {
                        // Second Enter: sync in background
                        app.git_status_cache = String::new();
                        app.git_syncing = true;
                        let result_slot = std::sync::Arc::clone(&app.git_sync_result);
                        let home_str = home.to_string_lossy().to_string();
                        std::thread::spawn(move || {
                            let result = (|| -> Result<String, String> {
                                // 1. Stage and commit local changes first
                                let _ = std::process::Command::new("git")
                                    .args(["-C", &home_str, "add", "."])
                                    .output();
                                let diff = std::process::Command::new("git")
                                    .args(["-C", &home_str, "diff", "--cached", "--quiet"])
                                    .output()
                                    .map_err(|e| e.to_string())?;
                                if !diff.status.success() {
                                    let _ = std::process::Command::new("git")
                                        .args(["-C", &home_str, "commit", "-m", "bibox sync"])
                                        .output();
                                }
                                // 2. Pull with rebase (safe now - no unstaged changes)
                                let pull = std::process::Command::new("git")
                                    .args(["-C", &home_str, "pull", "--rebase"])
                                    .output()
                                    .map_err(|e| e.to_string())?;
                                if !pull.status.success() {
                                    return Err(format!("git pull failed: {}", String::from_utf8_lossy(&pull.stderr).trim()));
                                }
                                // 3. Push
                                let push = std::process::Command::new("git")
                                    .args(["-C", &home_str, "push"])
                                    .output()
                                    .map_err(|e| e.to_string())?;
                                if !push.status.success() {
                                    return Err(format!("git push failed: {}", String::from_utf8_lossy(&push.stderr).trim()));
                                }
                                Ok("Sync complete.".into())
                            })();
                            if let Ok(mut slot) = result_slot.lock() {
                                *slot = Some(result);
                            }
                        });
                    }
                } else {
                    app.mode = Mode::Message("No home set. Run `bibox init <path>` first.".into());
                }
            } else {
                // Save config
                let _ = crate::config::save_config(&app.config);
                app.mode = Mode::Message("Settings saved.".into());
            }
        }
        KeyCode::Esc => {
            if let Ok(cfg) = crate::config::load_config() {
                app.config.line_numbers = cfg.line_numbers;
                app.config.panel_ratio = cfg.panel_ratio;
                app.config.bib_export_dir = cfg.bib_export_dir;
                app.config.export_dir = cfg.export_dir;
                app.config.citekey_format = cfg.citekey_format;
            }
            app.mode = Mode::Normal;
        }
        _ => {}
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
                                app.mode = Mode::Message(format!("PDF attached: {}", dest));
                            }
                            Err(e) => { app.mode = Mode::Message(format!("Attach failed: {}", e)); }
                        }
                    }
                    FilePickerContext::BibExportDir => {
                        app.config.bib_export_dir = path;
                    }
                    FilePickerContext::ExportDir => {
                        app.config.export_dir = path;
                    }
                    FilePickerContext::Home => {
                        app.config.home = Some(path.clone());
                        let expanded = crate::config::expand_tilde(&path);
                        app.config.bibox_dir = expanded.join("pdfs");
                        app.config.notes_dir = expanded.join("notes");
                    }
                }
            }
        }
        Some(ratatree::PickerResult::Cancelled) => {
            app.file_picker_state = None;
            app.mode = Mode::Normal;
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
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let config_clone = Config {
        home: config.home.clone(),
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
        msgs: crate::i18n::Msgs::new(&config.language),
    };

    let mut app = App::new(config_clone)?;
    app.apply_filters();

    let result = run_loop(&mut terminal, &mut app);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
    terminal.show_cursor()?;
    result
}

fn draw_search_result_picker(f: &mut Frame, state: &SearchResultPickerState, area: Rect) {
    let popup_area = centered_rect(80, 50, area);
    f.render_widget(Clear, popup_area);

    let mut lines = vec![
        Line::from(Span::styled(
            format!("Search results for [{}]:", state.key),
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];

    for (i, r) in state.results.iter().enumerate() {
        let arrow = if i == state.index { "▶ " } else { "  " };
        let style = if i == state.index { Style::default().fg(Color::Cyan) } else { Style::default() };

        let author = if r.authors.is_empty() { "Unknown".into() } else {
            let first = r.authors[0].split(',').next().unwrap_or(&r.authors[0]).trim().to_string();
            if r.authors.len() > 1 { format!("{} et al.", first) } else { first }
        };
        let year = r.year.map(|y| y.to_string()).unwrap_or_default();

        lines.push(Line::from(vec![
            Span::styled(arrow, Style::default().fg(Color::Yellow)),
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
                format!("    {}", v), Style::default().fg(Color::DarkGray),
            )));
        }
        lines.push(Line::from(""));
    }

    lines.push(Line::from(Span::styled(
        "↑↓ navigate  Enter select  Esc cancel",
        Style::default().fg(Color::DarkGray),
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
    f.render_widget(Clear, popup_area);

    let mut lines = vec![
        Line::from(Span::styled(
            format!("Fetch results for [{}]:", state.key),
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];

    for (i, change) in state.changes.iter().enumerate() {
        let arrow = if i == state.index { "▶ " } else { "  " };
        let check = if state.selected[i] { "[x]" } else { "[ ]" };

        let style = if !change.changed {
            Style::default().fg(Color::DarkGray)
        } else if i == state.index {
            Style::default().fg(Color::Cyan)
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
                Span::styled(arrow, Style::default().fg(Color::Yellow)),
                Span::styled(format!("{} ", check), style),
                Span::styled(format!("{:<10} ", change.field), style.add_modifier(Modifier::BOLD)),
                Span::styled(old_display.clone(), Style::default().fg(Color::Red)),
            ]));
            lines.push(Line::from(vec![
                Span::raw("               "),
                Span::styled(format!(" -> {}", new_display), Style::default().fg(Color::Green)),
            ]));
        } else {
            lines.push(Line::from(vec![
                Span::styled(arrow, Style::default().fg(Color::Yellow)),
                Span::styled(format!("{} ", check), style),
                Span::styled(format!("{:<10} ", change.field), style),
                Span::styled(old_display.clone(), style),
                Span::styled("  (unchanged)", Style::default().fg(Color::DarkGray)),
            ]));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "↑↓ navigate  Space toggle  Enter apply  Esc cancel",
        Style::default().fg(Color::DarkGray),
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
                let preview = app.fetch_preview.take().unwrap();
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
                }
                save_db(&db, &db_path)?;

                // Reload in-memory entries
                app.entries = db.entries;
                app.rebuild_collections();
                app.apply_filters();

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

fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> Result<()> {
    loop {
        terminal.draw(|f| draw(f, app))?;

        if event::poll(std::time::Duration::from_millis(16))? {
            if let Event::Key(key) = event::read()? {
                if handle_key(app, key)? { break; }
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
            // Reload note if in note preview mode
            app.note_citekey.clear();
        }

        // Poll git sync background result
        let sync_result_arc = std::sync::Arc::clone(&app.git_sync_result);
        if app.git_syncing {
            app.spinner_tick = app.spinner_tick.wrapping_add(1);
            if let Ok(mut slot) = sync_result_arc.try_lock() {
                if let Some(result) = slot.take() {
                    app.git_syncing = false;
                    app.git_status_checked = false;
                    match result {
                        Ok(msg) => {
                            let db_path = crate::config::resolve_db_path(&app.config);
                            // Check for merge conflict markers before reloading
                            let has_conflicts = std::fs::read_to_string(&db_path)
                                .map(|s| s.contains("<<<<<<<"))
                                .unwrap_or(false);
                            if has_conflicts {
                                app.mode = Mode::Message(
                                    "Sync done but db.json has merge conflicts! Run `bibox doctor` to inspect.".into()
                                );
                            } else if let Ok(db) = load_db(&db_path) {
                                app.entries = db.entries;
                                app.rebuild_collections();
                                app.apply_filters();
                                app.mode = Mode::Message(msg);
                            } else {
                                app.mode = Mode::Message(
                                    "Sync done but db.json could not be loaded. Run `bibox doctor`.".into()
                                );
                            }
                        }
                        Err(e) => { app.mode = Mode::Message(format!("Sync failed: {}", e)); }
                    }
                }
            }
        }

        // Poll git fetch background result
        let fetch_result_arc = std::sync::Arc::clone(&app.git_fetch_result);
        if app.git_fetching {
            app.spinner_tick = app.spinner_tick.wrapping_add(1);
            if let Ok(mut slot) = fetch_result_arc.try_lock() {
                if let Some(status) = slot.take() {
                    app.git_status_cache = status;
                    app.git_fetching = false;
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
