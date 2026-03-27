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
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Tabs},
    Frame, Terminal,
};
use std::io;

use crate::bibtex::entries_to_bibtex;
use crate::config::Config;
use crate::models::Entry;
use crate::storage::{load_db, save_db};

// ── State ────────────────────────────────────────────────────────────────────

enum Mode {
    Normal,
    Search,
    Confirm(ConfirmAction),
    Detail,
    Message(String),
    Help,
    NoteView,
    SortMenu,
    CollectionPicker,
    TagEditor,
}

enum ConfirmAction {
    Delete(String), // bibtex_key
}

#[derive(Clone, Copy, PartialEq)]
enum SortCriterion {
    Year,
    Author,
    Title,
    Created,
}

impl SortCriterion {
    fn label(&self) -> &'static str {
        match self {
            SortCriterion::Year => "Year",
            SortCriterion::Author => "Author",
            SortCriterion::Title => "Title",
            SortCriterion::Created => "Created",
        }
    }

    fn default_ascending(&self) -> bool {
        match self {
            SortCriterion::Year => false,
            SortCriterion::Author => true,
            SortCriterion::Title => true,
            SortCriterion::Created => false,
        }
    }

    fn all() -> [SortCriterion; 4] {
        [SortCriterion::Year, SortCriterion::Author, SortCriterion::Title, SortCriterion::Created]
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

    fn move_up(&mut self) {
        if self.index > 0 { self.index -= 1; }
    }

    fn move_down(&mut self) {
        let max = self.items.len();
        if self.index < max { self.index += 1; }
    }

    fn is_on_new_item(&self) -> bool {
        self.index == self.items.len()
    }

    fn toggle(&mut self) {
        if self.is_on_new_item() {
            self.new_item_input = Some(String::new());
        } else if let Some(item) = self.items.get_mut(self.index) {
            item.1 = !item.1;
        }
    }

    fn in_input_mode(&self) -> bool {
        self.new_item_input.is_some()
    }

    fn apply_char(&mut self, c: char) {
        if let Some(ref mut input) = self.new_item_input {
            input.push(c);
        }
    }

    fn backspace(&mut self) {
        if let Some(ref mut input) = self.new_item_input {
            input.pop();
        }
    }

    fn confirm_input(&mut self) {
        if let Some(input) = self.new_item_input.take() {
            let name = input.trim().to_string();
            if !name.is_empty() && !self.items.iter().any(|(n, _)| n == &name) {
                self.items.push((name, true));
            }
        }
    }

    fn cancel_input(&mut self) {
        self.new_item_input = None;
    }

    fn checked_names(&self) -> Vec<String> {
        self.items.iter().filter(|(_, c)| *c).map(|(n, _)| n.clone()).collect()
    }
}

pub struct App {
    entries: Vec<Entry>,
    filtered: Vec<usize>, // indices into entries
    list_state: ListState,
    tab_index: usize,     // 0 = All, 1..N = collections
    collections: Vec<String>,
    search_query: String,
    mode: Mode,
    config: Config,
    note_content: String,
    note_scroll: u16,
    note_citekey: String,
    pending_editor: Option<std::path::PathBuf>,
    sort_by: SortCriterion,
    sort_ascending: bool,
    sort_menu_index: usize,
    prev_sort_by: SortCriterion,
    prev_sort_ascending: bool,
    picker: Option<ChecklistPicker>,
}

impl App {
    pub fn new(config: Config) -> Result<Self> {
        let db_path = crate::config::db_path();
        let db = load_db(&db_path)?;
        let entries = db.entries;

        // Collect unique collections, sorted
        let mut col_set: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for e in &entries {
            for c in &e.collections {
                col_set.insert(c.clone());
            }
        }
        let collections: Vec<String> = col_set.into_iter().collect();

        let filtered: Vec<usize> = (0..entries.len()).collect();
        let mut list_state = ListState::default();
        if !filtered.is_empty() {
            list_state.select(Some(0));
        }

        Ok(Self {
            entries,
            filtered,
            list_state,
            tab_index: 0,
            collections,
            search_query: String::new(),
            mode: Mode::Normal,
            config,
            note_content: String::new(),
            note_scroll: 0,
            note_citekey: String::new(),
            pending_editor: None,
            sort_by: SortCriterion::Created,
            sort_ascending: false,
            sort_menu_index: 0,
            prev_sort_by: SortCriterion::Created,
            prev_sort_ascending: false,
            picker: None,
        })
    }

    fn current_collection(&self) -> Option<&str> {
        if self.tab_index == 0 {
            None
        } else {
            self.collections.get(self.tab_index - 1).map(|s| s.as_str())
        }
    }

    fn apply_filters(&mut self) {
        let col = self.current_collection().map(|s| s.to_string());
        let query = self.search_query.to_lowercase();

        self.filtered = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                // Collection filter
                let col_ok = match &col {
                    None => true,
                    Some(c) => e.collections.contains(c),
                };
                if !col_ok {
                    return false;
                }
                // Search filter
                if query.is_empty() {
                    return true;
                }
                let title = e.title.as_deref().unwrap_or("").to_lowercase();
                let author = e.author.join(" ").to_lowercase();
                let key = e.bibtex_key.to_lowercase();
                let tags = e.tags.join(" ").to_lowercase();
                title.contains(&query)
                    || author.contains(&query)
                    || key.contains(&query)
                    || tags.contains(&query)
            })
            .map(|(i, _)| i)
            .collect();

        // Adjust selection
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
            // None/empty always sort LAST regardless of ascending/descending
            match sort_by {
                SortCriterion::Year => {
                    match (ea.year, eb.year) {
                        (Some(a), Some(b)) => {
                            let cmp = a.cmp(&b);
                            if ascending { cmp } else { cmp.reverse() }
                        }
                        (Some(_), None) => std::cmp::Ordering::Less,
                        (None, Some(_)) => std::cmp::Ordering::Greater,
                        (None, None) => std::cmp::Ordering::Equal,
                    }
                }
                SortCriterion::Author => {
                    let a_name = ea.author.first()
                        .and_then(|a| a.split(',').next())
                        .unwrap_or("");
                    let b_name = eb.author.first()
                        .and_then(|a| a.split(',').next())
                        .unwrap_or("");
                    match (a_name.is_empty(), b_name.is_empty()) {
                        (true, false) => std::cmp::Ordering::Greater,
                        (false, true) => std::cmp::Ordering::Less,
                        _ => {
                            let cmp = a_name.to_lowercase().cmp(&b_name.to_lowercase());
                            if ascending { cmp } else { cmp.reverse() }
                        }
                    }
                }
                SortCriterion::Title => {
                    let a_title = ea.title.as_deref().unwrap_or("");
                    let b_title = eb.title.as_deref().unwrap_or("");
                    match (a_title.is_empty(), b_title.is_empty()) {
                        (true, false) => std::cmp::Ordering::Greater,
                        (false, true) => std::cmp::Ordering::Less,
                        _ => {
                            let cmp = a_title.to_lowercase().cmp(&b_title.to_lowercase());
                            if ascending { cmp } else { cmp.reverse() }
                        }
                    }
                }
                SortCriterion::Created => {
                    let cmp = ea.created_at.cmp(&eb.created_at);
                    if ascending { cmp } else { cmp.reverse() }
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

    fn move_down(&mut self) {
        if self.filtered.is_empty() {
            return;
        }
        let next = match self.list_state.selected() {
            Some(i) => (i + 1).min(self.filtered.len() - 1),
            None => 0,
        };
        self.list_state.select(Some(next));
    }

    fn move_up(&mut self) {
        if self.filtered.is_empty() {
            return;
        }
        let prev = match self.list_state.selected() {
            Some(i) if i > 0 => i - 1,
            _ => 0,
        };
        self.list_state.select(Some(prev));
    }

    fn tab_count(&self) -> usize {
        self.collections.len() + 1
    }

    fn next_tab(&mut self) {
        self.tab_index = (self.tab_index + 1) % self.tab_count();
        self.list_state.select(Some(0));
        self.apply_filters();
    }

    fn prev_tab(&mut self) {
        let n = self.tab_count();
        self.tab_index = (self.tab_index + n - 1) % n;
        self.list_state.select(Some(0));
        self.apply_filters();
    }

    fn open_pdf(&self, entry: &Entry) {
        if let Some(fp) = &entry.file_path {
            let full_path = self.config.bibox_dir.join(fp);
            if !full_path.exists() {
                return;
            }
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
        let entries: Vec<&Entry> = self
            .entries
            .iter()
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

    fn load_note(&mut self) {
        let citekey = match self.selected_entry() {
            Some(entry) => entry.bibtex_key.clone(),
            None => return,
        };
        let note_path = self.config.notes_dir.join(format!("{}.md", citekey));
        self.note_content = if note_path.exists() {
            std::fs::read_to_string(&note_path).unwrap_or_else(|_| "Error reading note.".into())
        } else {
            "No note yet. Press N to create one.".into()
        };
        self.note_citekey = citekey;
        self.note_scroll = 0;
    }

    fn delete_selected(&mut self) -> Result<()> {
        if let Some(idx) = self.selected_entry_idx() {
            let entry = &self.entries[idx];

            // Delete PDF if present
            if let Some(fp) = &entry.file_path {
                let path = self.config.bibox_dir.join(fp);
                if path.exists() {
                    let _ = std::fs::remove_file(&path);
                }
            }

            let key = entry.bibtex_key.clone();
            self.entries.retain(|e| e.bibtex_key != key);

            // Save DB
            let db_path = crate::config::db_path();
            let mut db = load_db(&db_path)?;
            db.entries = self.entries.clone();
            save_db(&db, &db_path)?;

            // Re-collect collections
            let mut col_set: std::collections::BTreeSet<String> =
                std::collections::BTreeSet::new();
            for e in &self.entries {
                for c in &e.collections {
                    col_set.insert(c.clone());
                }
            }
            self.collections = col_set.into_iter().collect();
            if self.tab_index > self.tab_count().saturating_sub(1) {
                self.tab_index = 0;
            }

            self.apply_filters();
        }
        Ok(())
    }

    fn open_collection_picker(&mut self) {
        if let Some(entry) = self.selected_entry() {
            let entry_cols: std::collections::HashSet<&String> = entry.collections.iter().collect();
            let all_cols: std::collections::BTreeSet<String> = self.entries.iter()
                .flat_map(|e| e.collections.iter().cloned())
                .collect();
            let items: Vec<(String, bool)> = all_cols.into_iter()
                .map(|c| { let checked = entry_cols.contains(&c); (c, checked) })
                .collect();
            let key = entry.bibtex_key.clone();
            self.picker = Some(ChecklistPicker::new(
                format!("Collections for [{}]:", key),
                items,
                "+ New collection...".into(),
            ));
            self.mode = Mode::CollectionPicker;
        }
    }

    fn open_tag_editor(&mut self) {
        if let Some(entry) = self.selected_entry() {
            let entry_tags: std::collections::HashSet<&String> = entry.tags.iter().collect();
            let all_tags: std::collections::BTreeSet<String> = self.entries.iter()
                .flat_map(|e| e.tags.iter().cloned())
                .collect();
            let items: Vec<(String, bool)> = all_tags.into_iter()
                .map(|t| { let checked = entry_tags.contains(&t); (t, checked) })
                .collect();
            let key = entry.bibtex_key.clone();
            self.picker = Some(ChecklistPicker::new(
                format!("Tags for [{}]:", key),
                items,
                "+ New tag...".into(),
            ));
            self.mode = Mode::TagEditor;
        }
    }

    fn apply_picker_collections(&mut self) -> Result<()> {
        let (new_cols, idx) = match (&self.picker, self.selected_entry_idx()) {
            (Some(picker), Some(idx)) => (picker.checked_names(), idx),
            _ => { self.picker = None; return Ok(()); }
        };
        self.picker = None;

        self.entries[idx].collections = new_cols;

        let db_path = crate::config::db_path();
        let mut db = load_db(&db_path)?;
        db.entries = self.entries.clone();
        save_db(&db, &db_path)?;

        let mut col_set: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for e in &self.entries {
            for c in &e.collections { col_set.insert(c.clone()); }
        }
        self.collections = col_set.into_iter().collect();
        if self.tab_index > self.tab_count().saturating_sub(1) {
            self.tab_index = 0;
        }
        self.apply_filters();
        Ok(())
    }

    fn apply_picker_tags(&mut self) -> Result<()> {
        let (new_tags, idx) = match (&self.picker, self.selected_entry_idx()) {
            (Some(picker), Some(idx)) => (picker.checked_names(), idx),
            _ => { self.picker = None; return Ok(()); }
        };
        self.picker = None;

        self.entries[idx].tags = new_tags;

        let db_path = crate::config::db_path();
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

    // Main layout: title/tabs | list | status bar
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .split(size);

    // ── Tabs ──
    let tab_titles: Vec<Line> = {
        let mut titles = vec![Line::from("All")];
        for c in &app.collections {
            titles.push(Line::from(c.as_str()));
        }
        titles
    };
    let tabs = Tabs::new(tab_titles)
        .block(Block::default().borders(Borders::ALL).title(" bibox "))
        .select(app.tab_index)
        .style(Style::default().fg(Color::Gray))
        .highlight_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );
    f.render_widget(tabs, chunks[0]);

    // ── Entry list ──
    let items: Vec<ListItem> = app
        .filtered
        .iter()
        .map(|&idx| {
            let e = &app.entries[idx];
            let title = e.title.as_deref().unwrap_or("(no title)");
            let author = e.author_display();
            let year = e
                .year
                .map(|y| y.to_string())
                .unwrap_or_else(|| "n.d.".to_string());
            let tags = if e.tags.is_empty() {
                String::new()
            } else {
                e.tags.join(", ")
            };
            let pdf_mark = if e.file_path.is_some() { " [pdf]" } else { "" };

            let line1 = Line::from(vec![
                Span::styled("[", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    e.bibtex_key.as_str(),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("] ", Style::default().fg(Color::DarkGray)),
                Span::raw(title),
                Span::styled(pdf_mark, Style::default().fg(Color::Green)),
            ]);

            let meta = if tags.is_empty() {
                format!("{} | {} | {}", e.entry_type, author, year)
            } else {
                format!("{} | {} | {} | {}", e.entry_type, author, year, tags)
            };
            let line2 = Line::from(Span::styled(
                meta,
                Style::default().fg(Color::DarkGray),
            ));

            ListItem::new(Text::from(vec![line1, line2]))
        })
        .collect();

    let block = Block::default().borders(Borders::ALL);
    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");
    f.render_stateful_widget(list, chunks[1], &mut app.list_state);

    // ── Status bar ──
    let status = match &app.mode {
        Mode::Search => {
            format!("/ {} (Esc to clear)", app.search_query)
        }
        _ => {
            "/ search  s sort  e export  o open  d delete  c collect  t tag  n note  ? help  q quit".to_string()
        }
    };
    let status_widget = Paragraph::new(status).style(Style::default().fg(Color::DarkGray));
    f.render_widget(status_widget, chunks[2]);

    // ── Overlays ──
    match &app.mode {
        Mode::Detail => {
            if let Some(entry) = app.selected_entry() {
                draw_detail_popup(f, entry, size);
            }
        }
        Mode::Confirm(ConfirmAction::Delete(key)) => {
            let key = key.clone();
            draw_confirm_popup(f, &format!("Delete '{}'? (y/n)", key), size);
        }
        Mode::Message(msg) => {
            let msg = msg.clone();
            draw_message_popup(f, &msg, size);
        }
        Mode::Help => {
            draw_help_popup(f, size);
        }
        Mode::NoteView => {
            draw_note_popup(f, app, size);
        }
        Mode::SortMenu => {
            draw_sort_popup(f, app, size);
        }
        Mode::CollectionPicker | Mode::TagEditor => {
            if let Some(ref picker) = app.picker {
                draw_checklist_popup(f, picker, size);
            }
        }
        _ => {}
    }
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

fn draw_detail_popup(f: &mut Frame, entry: &Entry, area: Rect) {
    let popup_area = centered_rect(80, 20, area);
    f.render_widget(Clear, popup_area);

    let mut lines = vec![];
    lines.push(Line::from(vec![
        Span::styled("Key: ", Style::default().fg(Color::Cyan)),
        Span::raw(&entry.bibtex_key),
    ]));
    if let Some(t) = &entry.title {
        lines.push(Line::from(vec![
            Span::styled("Title: ", Style::default().fg(Color::Cyan)),
            Span::raw(t.as_str()),
        ]));
    }
    if !entry.author.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("Author: ", Style::default().fg(Color::Cyan)),
            Span::raw(entry.author.join("; ")),
        ]));
    }
    if let Some(y) = entry.year {
        lines.push(Line::from(vec![
            Span::styled("Year: ", Style::default().fg(Color::Cyan)),
            Span::raw(y.to_string()),
        ]));
    }
    lines.push(Line::from(vec![
        Span::styled("Type: ", Style::default().fg(Color::Cyan)),
        Span::raw(entry.entry_type.to_string()),
    ]));
    if let Some(j) = &entry.journal {
        lines.push(Line::from(vec![
            Span::styled("Journal: ", Style::default().fg(Color::Cyan)),
            Span::raw(j.as_str()),
        ]));
    }
    if let Some(p) = &entry.publisher {
        lines.push(Line::from(vec![
            Span::styled("Publisher: ", Style::default().fg(Color::Cyan)),
            Span::raw(p.as_str()),
        ]));
    }
    if let Some(bt) = &entry.booktitle {
        lines.push(Line::from(vec![
            Span::styled("Booktitle: ", Style::default().fg(Color::Cyan)),
            Span::raw(bt.as_str()),
        ]));
    }
    if let Some(doi) = &entry.doi {
        lines.push(Line::from(vec![
            Span::styled("DOI: ", Style::default().fg(Color::Cyan)),
            Span::raw(doi.as_str()),
        ]));
    }
    if !entry.tags.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("Tags: ", Style::default().fg(Color::Cyan)),
            Span::raw(entry.tags.join(", ")),
        ]));
    }
    if !entry.collections.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("Collections: ", Style::default().fg(Color::Cyan)),
            Span::raw(entry.collections.join(", ")),
        ]));
    }
    if let Some(fp) = &entry.file_path {
        lines.push(Line::from(vec![
            Span::styled("File: ", Style::default().fg(Color::Cyan)),
            Span::raw(fp.as_str()),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Press Esc or Enter to close",
        Style::default().fg(Color::DarkGray),
    )));

    let text = Text::from(lines);
    let popup = Paragraph::new(text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Entry Detail "),
        )
        .wrap(ratatui::widgets::Wrap { trim: true });
    f.render_widget(popup, popup_area);
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
    let popup_area = centered_rect(80, 20, area);
    f.render_widget(Clear, popup_area);

    let help_text = vec![
        Line::from(Span::styled(
            "bibox — Keyboard Shortcuts",
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("Navigation", Style::default().fg(Color::Cyan)),
        ]),
        Line::from("  j/↓  Move down          Enter  Show details"),
        Line::from("  k/↑  Move up            y      Copy citekey"),
        Line::from("  Tab   Next collection    e      Export .bib"),
        Line::from("  h/l   Prev/Next tab      o      Open PDF"),
        Line::from(""),
        Line::from(vec![
            Span::styled("Edit", Style::default().fg(Color::Cyan)),
        ]),
        Line::from("  c     Manage collections  d     Delete entry"),
        Line::from("  t     Edit tags           n     View note"),
        Line::from("  s     Sort menu           N     Edit note ($EDITOR)"),
        Line::from(""),
        Line::from(vec![
            Span::styled("Other", Style::default().fg(Color::Cyan)),
        ]),
        Line::from("  /     Search              q     Quit"),
        Line::from("  ?     This help screen"),
        Line::from(""),
        Line::from(Span::styled(
            "Press ? or Esc to close",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let popup = Paragraph::new(help_text)
        .block(Block::default().borders(Borders::ALL).title(" Help "))
        .wrap(ratatui::widgets::Wrap { trim: false });
    f.render_widget(popup, popup_area);
}

fn draw_note_popup(f: &mut Frame, app: &App, area: Rect) {
    let popup_area = centered_rect(80, 20, area);
    f.render_widget(Clear, popup_area);

    let lines: Vec<Line> = app
        .note_content
        .lines()
        .skip(app.note_scroll as usize)
        .map(|l| Line::from(l.to_string()))
        .collect();

    let mut text_lines = lines;
    text_lines.push(Line::from(""));
    text_lines.push(Line::from(Span::styled(
        "↑↓ scroll  N edit in $EDITOR  Esc close",
        Style::default().fg(Color::DarkGray),
    )));

    let title = format!(" Note: {} ", app.note_citekey);
    let popup = Paragraph::new(text_lines)
        .block(Block::default().borders(Borders::ALL).title(title))
        .wrap(ratatui::widgets::Wrap { trim: false });
    f.render_widget(popup, popup_area);
}

fn draw_sort_popup(f: &mut Frame, app: &App, area: Rect) {
    let popup_area = centered_rect(50, 10, area);
    f.render_widget(Clear, popup_area);

    let criteria = SortCriterion::all();
    let mut lines = vec![
        Line::from(Span::styled("Sort by:", Style::default().fg(Color::Yellow))),
        Line::from(""),
    ];
    for (i, c) in criteria.iter().enumerate() {
        let selected = *c == app.sort_by;
        let arrow = if i == app.sort_menu_index { "▶ " } else { "  " };
        let dir = if selected {
            if app.sort_ascending { "↑ asc" } else { "↓ desc" }
        } else {
            "     "
        };
        let style = if selected {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default()
        };
        lines.push(Line::from(vec![
            Span::styled(arrow, Style::default().fg(Color::Yellow)),
            Span::styled(format!("{:<12}", c.label()), style),
            Span::styled(dir, Style::default().fg(Color::DarkGray)),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "↑↓ select  Enter apply  Space toggle ↑↓  Esc cancel",
        Style::default().fg(Color::DarkGray),
    )));

    let popup = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(" Sort "));
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
        let check_style = if *checked {
            Style::default().fg(Color::Green)
        } else {
            Style::default().fg(Color::DarkGray)
        };
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
    let footer = if picker.in_input_mode() {
        "Enter confirm  Esc cancel"
    } else {
        "↑↓ navigate  Space toggle  Enter done  Esc cancel"
    };
    lines.push(Line::from(Span::styled(footer, Style::default().fg(Color::DarkGray))));

    let popup = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(popup, popup_area);
}

// ── Event loop ───────────────────────────────────────────────────────────────

fn handle_key(app: &mut App, key: crossterm::event::KeyEvent) -> Result<bool> {
    match &app.mode {
        Mode::Normal => handle_normal(app, key),
        Mode::Search => handle_search(app, key),
        Mode::Confirm(_) => handle_confirm(app, key),
        Mode::Detail => handle_detail(app, key),
        Mode::Message(_) => {
            app.mode = Mode::Normal;
            Ok(false)
        }
        Mode::Help => handle_help(app, key),
        Mode::NoteView => handle_note_view(app, key),
        Mode::SortMenu => handle_sort_menu(app, key),
        Mode::CollectionPicker => handle_picker(app, key, false),
        Mode::TagEditor => handle_picker(app, key, true),
    }
}

fn handle_normal(app: &mut App, key: crossterm::event::KeyEvent) -> Result<bool> {
    match key.code {
        // Quit
        KeyCode::Char('q') => return Ok(true),
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => return Ok(true),

        // Navigation
        KeyCode::Char('j') | KeyCode::Down => app.move_down(),
        KeyCode::Char('k') | KeyCode::Up => app.move_up(),

        // Tab switching
        KeyCode::Tab | KeyCode::Char('l') => app.next_tab(),
        KeyCode::BackTab | KeyCode::Char('h') => app.prev_tab(),

        // Search
        KeyCode::Char('/') => {
            app.mode = Mode::Search;
        }

        // Detail popup
        KeyCode::Enter => {
            if app.selected_entry().is_some() {
                app.mode = Mode::Detail;
            }
        }

        // Copy citekey to clipboard
        KeyCode::Char('y') => {
            if let Some(entry) = app.selected_entry() {
                let key = entry.bibtex_key.clone();
                if let Ok(mut ctx) = arboard::Clipboard::new() {
                    let _ = ctx.set_text(&key);
                }
                app.mode = Mode::Message(format!("Copied: {}", key));
            }
        }

        // Open PDF with 'o'
        KeyCode::Char('o') => {
            if let Some(entry) = app.selected_entry() {
                let entry = entry.clone();
                app.open_pdf(&entry);
            }
        }

        // Export .bib for current collection
        KeyCode::Char('e') => {
            let path = app.export_collection_bib();
            app.mode = Mode::Message(format!("Exported: {}", path));
        }

        // Delete with confirm
        KeyCode::Char('d') => {
            if let Some(entry) = app.selected_entry() {
                let key = entry.bibtex_key.clone();
                app.mode = Mode::Confirm(ConfirmAction::Delete(key));
            }
        }

        KeyCode::Char('?') => {
            app.mode = Mode::Help;
        }

        // View note
        KeyCode::Char('n') => {
            if app.selected_entry().is_some() {
                app.load_note();
                app.mode = Mode::NoteView;
            } else {
                app.mode = Mode::Message("No entry selected.".into());
            }
        }

        // Edit note in $EDITOR
        KeyCode::Char('N') => {
            if app.selected_entry().is_some() {
                return open_note_editor(app);
            } else {
                app.mode = Mode::Message("No entry selected.".into());
            }
        }

        KeyCode::Char('s') => {
            app.prev_sort_by = app.sort_by;
            app.prev_sort_ascending = app.sort_ascending;
            app.sort_menu_index = SortCriterion::all()
                .iter()
                .position(|c| *c == app.sort_by)
                .unwrap_or(0);
            app.mode = Mode::SortMenu;
        }

        KeyCode::Char('c') => {
            if app.selected_entry().is_some() {
                app.open_collection_picker();
            } else {
                app.mode = Mode::Message("No entry selected.".into());
            }
        }

        KeyCode::Char('t') => {
            if app.selected_entry().is_some() {
                app.open_tag_editor();
            } else {
                app.mode = Mode::Message("No entry selected.".into());
            }
        }

        _ => {}
    }
    Ok(false)
}

fn handle_search(app: &mut App, key: crossterm::event::KeyEvent) -> Result<bool> {
    match key.code {
        KeyCode::Esc => {
            app.search_query.clear();
            app.apply_filters();
            app.mode = Mode::Normal;
        }
        KeyCode::Backspace => {
            app.search_query.pop();
            app.apply_filters();
        }
        KeyCode::Char(c) => {
            app.search_query.push(c);
            app.apply_filters();
        }
        KeyCode::Enter => {
            // Stay in search but enter normal navigation
            app.mode = Mode::Normal;
        }
        _ => {}
    }
    Ok(false)
}

fn handle_confirm(app: &mut App, key: crossterm::event::KeyEvent) -> Result<bool> {
    match key.code {
        KeyCode::Char('y') => {
            let action = std::mem::replace(&mut app.mode, Mode::Normal);
            if let Mode::Confirm(ConfirmAction::Delete(_)) = action {
                app.delete_selected()?;
                app.mode = Mode::Message("Entry deleted.".to_string());
            }
        }
        KeyCode::Char('n') | KeyCode::Esc => {
            app.mode = Mode::Normal;
        }
        _ => {}
    }
    Ok(false)
}

fn handle_detail(app: &mut App, key: crossterm::event::KeyEvent) -> Result<bool> {
    match key.code {
        KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => {
            app.mode = Mode::Normal;
        }
        _ => {}
    }
    Ok(false)
}

fn handle_help(app: &mut App, key: crossterm::event::KeyEvent) -> Result<bool> {
    match key.code {
        KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') => {
            app.mode = Mode::Normal;
        }
        _ => {}
    }
    Ok(false)
}

fn handle_note_view(app: &mut App, key: crossterm::event::KeyEvent) -> Result<bool> {
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            app.mode = Mode::Normal;
        }
        KeyCode::Down | KeyCode::Char('j') => {
            let total_lines = app.note_content.lines().count() as u16;
            if app.note_scroll < total_lines.saturating_sub(1) {
                app.note_scroll += 1;
            }
        }
        KeyCode::Up | KeyCode::Char('k') => {
            app.note_scroll = app.note_scroll.saturating_sub(1);
        }
        KeyCode::Char('N') => {
            return open_note_editor(app);
        }
        _ => {}
    }
    Ok(false)
}

fn handle_sort_menu(app: &mut App, key: crossterm::event::KeyEvent) -> Result<bool> {
    let criteria = SortCriterion::all();
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => {
            if app.sort_menu_index > 0 {
                app.sort_menu_index -= 1;
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if app.sort_menu_index < criteria.len() - 1 {
                app.sort_menu_index += 1;
            }
        }
        KeyCode::Char(' ') => {
            let selected = criteria[app.sort_menu_index];
            if selected == app.sort_by {
                app.sort_ascending = !app.sort_ascending;
            } else {
                app.sort_by = selected;
            }
        }
        KeyCode::Enter => {
            let new_criterion = criteria[app.sort_menu_index];
            if new_criterion != app.sort_by {
                app.sort_ascending = new_criterion.default_ascending();
            }
            app.sort_by = new_criterion;
            app.apply_sort();
            app.mode = Mode::Normal;
        }
        KeyCode::Esc => {
            // Revert to pre-menu state
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
            KeyCode::Enter => {
                if let Some(ref mut picker) = app.picker { picker.confirm_input(); }
            }
            KeyCode::Esc => {
                if let Some(ref mut picker) = app.picker { picker.cancel_input(); }
            }
            KeyCode::Backspace => {
                if let Some(ref mut picker) = app.picker { picker.backspace(); }
            }
            KeyCode::Char(c) => {
                if let Some(ref mut picker) = app.picker { picker.apply_char(c); }
            }
            _ => {}
        }
    } else {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                if let Some(ref mut picker) = app.picker { picker.move_up(); }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if let Some(ref mut picker) = app.picker { picker.move_down(); }
            }
            KeyCode::Char(' ') => {
                if let Some(ref mut picker) = app.picker { picker.toggle(); }
            }
            KeyCode::Enter => {
                if is_tags {
                    app.apply_picker_tags()?;
                } else {
                    app.apply_picker_collections()?;
                }
                app.mode = Mode::Normal;
            }
            KeyCode::Esc => {
                app.picker = None;
                app.mode = Mode::Normal;
            }
            _ => {}
        }
    }
    Ok(false)
}

fn open_note_editor(app: &mut App) -> Result<bool> {
    let entry = match app.selected_entry() {
        Some(e) => e.clone(),
        None => return Ok(false),
    };

    let notes_dir = &app.config.notes_dir;
    std::fs::create_dir_all(notes_dir)?;
    let note_path = notes_dir.join(format!("{}.md", entry.bibtex_key));

    if !note_path.exists() {
        let header = format!(
            "# {}\ncitekey: {}\n\n",
            entry.title.as_deref().unwrap_or("Untitled"),
            entry.bibtex_key
        );
        std::fs::write(&note_path, &header)?;
    }

    app.pending_editor = Some(note_path);
    app.mode = Mode::Normal;
    Ok(false)
}

// ── Entry point ──────────────────────────────────────────────────────────────

pub fn run_tui(config: &Config) -> Result<()> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let config_clone = Config {
        bibox_dir: config.bibox_dir.clone(),
        pdf_viewer: config.pdf_viewer.clone(),
        default_collection: config.default_collection.clone(),
        search_case_sensitive: config.search_case_sensitive,
        default_page_size: config.default_page_size,
        language: config.language.clone(),
        git: config.git,
        notes_dir: config.notes_dir.clone(),
        templates_dir: config.templates_dir.clone(),
        msgs: crate::i18n::Msgs::new(&config.language),
    };

    let mut app = App::new(config_clone)?;
    app.apply_filters();

    let result = run_loop(&mut terminal, &mut app);

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    result
}

fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> Result<()> {
    loop {
        terminal.draw(|f| draw(f, app))?;

        if event::poll(std::time::Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                if handle_key(app, key)? {
                    break;
                }
            }
        }

        // Handle pending editor launch (suspend TUI, open editor, resume)
        if let Some(note_path) = app.pending_editor.take() {
            let editor = std::env::var("EDITOR").unwrap_or_else(|_| {
                if std::process::Command::new("which").arg("nano").output()
                    .map(|o| o.status.success()).unwrap_or(false)
                { "nano".to_string() } else { "vi".to_string() }
            });

            disable_raw_mode()?;
            execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture)?;

            let status = std::process::Command::new(&editor)
                .arg(&note_path)
                .status();

            enable_raw_mode()?;
            execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
            terminal.clear()?;

            if let Err(e) = status {
                app.mode = Mode::Message(format!("Editor failed: {}", e));
            }
        }
    }
    Ok(())
}
