use crossterm::event::{KeyCode, KeyModifiers};

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
}
