//! 화면 색의 역할 이름과 테마. 기본 `terminal`은 ANSI 이름이라 터미널 팔레트를 따르고,
//! 테마 파일은 VS Code 색 테마 JSON의 `colors`에서 아는 키만 읽는다. 그리기 코드는 `theme()`로 읽는다.

use std::sync::RwLock;

use ratatui::style::Color;

/// 역할별 색. `bg`가 Some이면 화면 전체와 팝업 안을 그 색으로 칠한다(VS Code 테마의 글자색은 그 배경 전제).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Theme {
    /// 도움말의 액션 이름, 검색 입력 글자. 배경을 칠할 때 프레임 전체의 기본 글자색
    pub fg: Color,
    pub bg: Option<Color>,
    /// 포커스 패널 테두리, 활성 탭, 커서 행 글자, Info 라벨
    pub accent: Color,
    /// 팝업 제목, 프롬프트, citekey, 절 머리
    pub heading: Color,
    /// 설명, 안내 줄, 구분선, 비활성 탭, 비포커스 테두리
    pub muted: Color,
    /// 포커스 패널의 커서 행(컬렉션), 활성 탭
    pub selection_bg: Color,
    pub selection_fg: Color,
    /// 약한 강조: 항목 목록의 커서 행, 비포커스 패널의 커서 행, 노트의 코드
    pub inactive_bg: Color,
    pub inactive_fg: Color,
    pub success: Color,
    pub warning: Color,
    pub error: Color,
}

impl Theme {
    /// 지금까지의 색 그대로. ANSI 이름이라 터미널 팔레트를 따른다.
    pub const fn terminal() -> Theme {
        Theme {
            fg: Color::White,
            bg: None,
            accent: Color::Cyan,
            heading: Color::Yellow,
            muted: Color::DarkGray,
            selection_bg: Color::Cyan,
            selection_fg: Color::Black,
            inactive_bg: Color::DarkGray,
            inactive_fg: Color::White,
            success: Color::Green,
            warning: Color::Yellow,
            error: Color::Red,
        }
    }

    /// VS Code 색 테마 JSON. `colors` 객체에서 아는 키만 읽고 나머지는 무시한다. 없는 키는 `terminal` 값,
    /// 선택 글자색 둘은 VS Code처럼 `editor.foreground`를 먼저 물려받는다.
    pub fn from_vscode_json(text: &str) -> Result<Theme, String> {
        let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let Some(colors) = v.get("colors").and_then(|c| c.as_object()) else {
            return Err("no \"colors\" object".to_string());
        };
        let color = |key: &str| colors.get(key).and_then(|v| v.as_str()).and_then(parse_hex);
        let base = Theme::terminal();
        let fg = color("editor.foreground");
        let text_on_selection = |key: &str, fallback: Color| color(key).or(fg).unwrap_or(fallback);
        Ok(Theme {
            fg: fg.unwrap_or(base.fg),
            bg: color("editor.background"),
            accent: color("focusBorder").unwrap_or(base.accent),
            heading: color("pickerGroup.foreground").unwrap_or(base.heading),
            muted: color("descriptionForeground").unwrap_or(base.muted),
            selection_bg: color("list.activeSelectionBackground").unwrap_or(base.selection_bg),
            selection_fg: text_on_selection("list.activeSelectionForeground", base.selection_fg),
            inactive_bg: color("list.inactiveSelectionBackground").unwrap_or(base.inactive_bg),
            inactive_fg: text_on_selection("list.inactiveSelectionForeground", base.inactive_fg),
            success: color("terminal.ansiGreen").unwrap_or(base.success),
            warning: color("editorWarning.foreground").unwrap_or(base.warning),
            error: color("editorError.foreground").unwrap_or(base.error),
        })
    }
}

/// `#rgb`, `#rrggbb`, `#rrggbbaa`(알파는 버린다). 그 밖은 None.
pub fn parse_hex(s: &str) -> Option<Color> {
    let hex = s.strip_prefix('#')?;
    let digits: Vec<u8> = hex.chars().map(|c| c.to_digit(16).map(|d| d as u8)).collect::<Option<_>>()?;
    let (r, g, b) = match digits.len() {
        3 => (digits[0] * 17, digits[1] * 17, digits[2] * 17),
        6 | 8 => (digits[0] * 16 + digits[1], digits[2] * 16 + digits[3], digits[4] * 16 + digits[5]),
        _ => return None,
    };
    Some(Color::Rgb(r, g, b))
}

static THEME: RwLock<Theme> = RwLock::new(Theme::terminal());

/// 지금 테마. `Theme`은 Copy라 값으로 준다. 그리기는 메인 스레드뿐이다.
pub fn theme() -> Theme {
    *THEME.read().unwrap_or_else(|p| p.into_inner())
}

pub fn set_theme(t: Theme) {
    *THEME.write().unwrap_or_else(|p| p.into_inner()) = t;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_accepts_three_six_and_eight_digits_and_nothing_else() {
        assert_eq!(parse_hex("#abc"), Some(Color::Rgb(0xaa, 0xbb, 0xcc)));
        assert_eq!(parse_hex("#1F1F1F"), Some(Color::Rgb(0x1f, 0x1f, 0x1f)));
        assert_eq!(parse_hex("#0078d4ff"), Some(Color::Rgb(0x00, 0x78, 0xd4)), "alpha is dropped");
        for bad in ["", "#", "#12", "#12345", "#1234567", "1F1F1F", "#GGGGGG", "blue"] {
            assert_eq!(parse_hex(bad), None, "{:?}", bad);
        }
    }

    #[test]
    fn a_full_theme_maps_every_role() {
        let t = Theme::from_vscode_json(r##"{"name":"x","type":"dark","colors":{
            "editor.background":"#101010","editor.foreground":"#f0f0f0","focusBorder":"#0078d4",
            "pickerGroup.foreground":"#3794ff","descriptionForeground":"#9d9d9d",
            "list.activeSelectionBackground":"#04395e","list.activeSelectionForeground":"#ffffff",
            "list.inactiveSelectionBackground":"#37373d","list.inactiveSelectionForeground":"#cccccc",
            "terminal.ansiGreen":"#0dbc79","editorWarning.foreground":"#cca700","editorError.foreground":"#f14c4c",
            "editor.lineHighlightBackground":"#ignored"},"tokenColors":[{"scope":"comment","settings":{"foreground":"#5c6370"}}]}"##).unwrap();
        assert_eq!(t.bg, Some(Color::Rgb(0x10, 0x10, 0x10)));
        assert_eq!(t.fg, Color::Rgb(0xf0, 0xf0, 0xf0));
        assert_eq!(t.accent, Color::Rgb(0x00, 0x78, 0xd4));
        assert_eq!(t.heading, Color::Rgb(0x37, 0x94, 0xff));
        assert_eq!(t.muted, Color::Rgb(0x9d, 0x9d, 0x9d));
        assert_eq!((t.selection_bg, t.selection_fg), (Color::Rgb(0x04, 0x39, 0x5e), Color::Rgb(0xff, 0xff, 0xff)));
        assert_eq!((t.inactive_bg, t.inactive_fg), (Color::Rgb(0x37, 0x37, 0x3d), Color::Rgb(0xcc, 0xcc, 0xcc)));
        assert_eq!(t.success, Color::Rgb(0x0d, 0xbc, 0x79));
        assert_eq!(t.warning, Color::Rgb(0xcc, 0xa7, 0x00));
        assert_eq!(t.error, Color::Rgb(0xf1, 0x4c, 0x4c));
    }

    #[test]
    fn missing_keys_fall_back_to_terminal_and_selection_text_to_the_editor_foreground() {
        let t = Theme::from_vscode_json(r##"{"colors":{"editor.foreground":"#eeeeee","focusBorder":"#123456"}}"##).unwrap();
        let base = Theme::terminal();
        assert_eq!(t.accent, Color::Rgb(0x12, 0x34, 0x56));
        assert_eq!(t.bg, None, "no background key, nothing painted");
        assert_eq!(t.muted, base.muted);
        assert_eq!(t.heading, base.heading);
        assert_eq!(t.selection_bg, base.selection_bg);
        assert_eq!(t.selection_fg, Color::Rgb(0xee, 0xee, 0xee), "VS Code inherits the editor foreground");
        assert_eq!(t.inactive_fg, Color::Rgb(0xee, 0xee, 0xee));
        let t = Theme::from_vscode_json(r##"{"colors":{"focusBorder":"#123456"}}"##).unwrap();
        assert_eq!(t.selection_fg, base.selection_fg, "no editor foreground either: terminal");
    }

    #[test]
    fn a_bad_value_only_loses_its_own_key() {
        let t = Theme::from_vscode_json(r##"{"colors":{"focusBorder":"blue","editorError.foreground":"#ff0000"}}"##).unwrap();
        assert_eq!(t.accent, Theme::terminal().accent);
        assert_eq!(t.error, Color::Rgb(0xff, 0, 0));
    }

    #[test]
    fn a_file_without_a_colors_object_is_an_error() {
        assert!(Theme::from_vscode_json(r#"{"name":"x"}"#).unwrap_err().contains("colors"));
        assert!(Theme::from_vscode_json(r#"{"colors":[]}"#).unwrap_err().contains("colors"));
        assert!(Theme::from_vscode_json("{not json").is_err());
    }

    #[test]
    fn the_global_starts_as_terminal_and_can_be_replaced() {
        assert_eq!(theme(), Theme::terminal());
        let mut t = Theme::terminal();
        t.accent = Color::Rgb(1, 2, 3);
        set_theme(t);
        assert_eq!(theme().accent, Color::Rgb(1, 2, 3));
        set_theme(Theme::terminal());
    }
}
