use anyhow::Result;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{self, ClearType},
};
use std::io::{self, Write};

pub struct SelectItem {
    pub key: String,
    pub display: String,
}

/// Display an interactive list. Returns the selected bibtex key, or None if cancelled.
pub fn interactive_select(items: &[SelectItem]) -> Result<Option<String>> {
    if items.is_empty() {
        return Ok(None);
    }

    let mut selected = 0usize;
    let mut stdout = io::stdout();

    terminal::enable_raw_mode()?;

    // Hide cursor
    execute!(stdout, cursor::Hide)?;

    let result = run_select(items, &mut selected, &mut stdout);

    // Always restore terminal
    execute!(stdout, cursor::Show)?;
    terminal::disable_raw_mode()?;
    println!(); // newline after selection

    result
}

fn run_select(
    items: &[SelectItem],
    selected: &mut usize,
    stdout: &mut io::Stdout,
) -> Result<Option<String>> {
    loop {
        // Render
        render(items, *selected, stdout)?;

        // Handle input
        if let Event::Key(key_event) = event::read()? {
            match (key_event.code, key_event.modifiers) {
                (KeyCode::Up, _) | (KeyCode::Char('k'), KeyModifiers::NONE) => {
                    if *selected > 0 {
                        *selected -= 1;
                    }
                }
                (KeyCode::Down, _) | (KeyCode::Char('j'), KeyModifiers::NONE) => {
                    if *selected < items.len() - 1 {
                        *selected += 1;
                    }
                }
                (KeyCode::Enter, _) => {
                    // Clear the rendered list before returning
                    clear_lines(items.len() + 2, stdout)?;
                    return Ok(Some(items[*selected].key.clone()));
                }
                (KeyCode::Esc, _) | (KeyCode::Char('q'), KeyModifiers::NONE) => {
                    clear_lines(items.len() + 2, stdout)?;
                    return Ok(None);
                }
                (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                    clear_lines(items.len() + 2, stdout)?;
                    return Ok(None);
                }
                _ => {}
            }
        }
    }
}

fn render(items: &[SelectItem], selected: usize, stdout: &mut io::Stdout) -> Result<()> {
    // Move cursor up to overwrite previous render (except first time)
    execute!(stdout, terminal::Clear(ClearType::FromCursorDown))?;

    for (i, item) in items.iter().enumerate() {
        if i == selected {
            writeln!(stdout, "\r\x1b[1m▶ {}\x1b[0m", item.display)?;
        } else {
            writeln!(stdout, "\r  {}", item.display)?;
        }
    }
    writeln!(stdout, "\r")?;
    write!(stdout, "\r  \x1b[2m↑↓ navigate   Enter: copy key   Esc: quit\x1b[0m")?;

    // Move cursor back to top of the list
    let lines = items.len() + 2;
    execute!(stdout, cursor::MoveUp(lines as u16))?;

    stdout.flush()?;
    Ok(())
}

fn clear_lines(n: usize, stdout: &mut io::Stdout) -> Result<()> {
    for _ in 0..n {
        execute!(
            stdout,
            terminal::Clear(ClearType::CurrentLine),
            cursor::MoveDown(1)
        )?;
    }
    execute!(stdout, cursor::MoveUp(n as u16))?;
    Ok(())
}
