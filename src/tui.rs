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
}

enum ConfirmAction {
    Delete(String), // bibtex_key
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
            "j/k navigate  Tab/h/l tabs  / search  Enter detail  y copy  o export  p open  d delete  q quit".to_string()
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

        // Open PDF with 'p'
        KeyCode::Char('p') => {
            if let Some(entry) = app.selected_entry() {
                let entry = entry.clone();
                app.open_pdf(&entry);
            }
        }

        // Export .bib for current collection
        KeyCode::Char('o') => {
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

fn run_loop<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
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
    }
    Ok(())
}
