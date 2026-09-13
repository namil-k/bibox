//! 화면 색의 역할 이름과 테마. 기본 `terminal`은 ANSI 이름이라 터미널 팔레트를 따르고,
//! 테마 파일은 VS Code 색 테마 JSON의 `colors`에서 아는 키만 읽는다. 그리기 코드는 `theme()`로 읽는다.

use std::path::Path;
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

pub const TERMINAL: &str = "terminal";
const DARK: &str = include_str!("../assets/themes/dark.json");
const LIGHT: &str = include_str!("../assets/themes/light.json");
/// 바이너리 안의 프리셋. 파일 없이도 Theme 행이 뜻이 있고, 스키마의 예제다.
pub const BUILTIN: [(&str, &str); 2] = [("dark", DARK), ("light", LIGHT)];

/// 테마 이름. 파일 이름이 되므로 경로 문자를 막는다.
pub fn valid_name(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// `terminal`, 프리셋, 아니면 `themes/<name>.json`. 오류 문구는 사용자에게 그대로 보인다.
pub fn load(name: &str, themes_dir: &Path) -> Result<Theme, String> {
    if name == TERMINAL {
        return Ok(Theme::terminal());
    }
    if let Some((_, text)) = BUILTIN.iter().find(|(n, _)| *n == name) {
        return Theme::from_vscode_json(text);
    }
    if !valid_name(name) {
        return Err(format!("theme name \"{}\" may only use letters, digits, - and _", name));
    }
    let path = themes_dir.join(format!("{}.json", name));
    let shown = format!("themes/{}.json", name);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(format!("{} not found", shown)),
        Err(e) => return Err(format!("{}: {}", shown, e)),
    };
    Theme::from_vscode_json(&text).map_err(|e| format!("{}: {}", shown, e))
}

/// Settings 행의 선택지. 파일 이름이 프리셋과 같으면 프리셋이 이긴다(한 번만 나온다).
pub fn list(themes_dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = vec![TERMINAL.to_string()];
    names.extend(BUILTIN.iter().map(|(n, _)| n.to_string()));
    let mut files: Vec<String> = std::fs::read_dir(themes_dir)
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| {
                    let p = e.path();
                    let stem = p.file_stem()?.to_str()?.to_string();
                    (p.extension()?.to_str()? == "json" && valid_name(&stem) && !names.contains(&stem)).then_some(stem)
                })
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    names.extend(files);
    names
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
    #[test]
    fn both_presets_parse_and_paint_a_background() {
        for (name, text) in BUILTIN {
            let t = Theme::from_vscode_json(text).unwrap_or_else(|e| panic!("{}: {}", name, e));
            assert!(t.bg.is_some(), "{}", name);
            assert_ne!(t.accent, Theme::terminal().accent, "{}", name);
        }
        assert_eq!(BUILTIN.map(|(n, _)| n), ["dark", "light"]);
    }

    #[test]
    fn names_are_letters_digits_dash_and_underscore() {
        for ok in ["terminal", "one-dark", "Solarized_Light", "x1"] { assert!(valid_name(ok), "{}", ok); }
        for bad in ["", "../x", "a b", "a.json", "한글", "a/b"] { assert!(!valid_name(bad), "{}", bad); }
    }

    fn scratch(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("bibox-themes-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn list_puts_terminal_and_presets_first_then_files_sorted() {
        let d = scratch("list");
        std::fs::write(d.join("b.json"), "{}").unwrap();
        std::fs::write(d.join("a.json"), "{}").unwrap();
        std::fs::write(d.join("notes.txt"), "").unwrap();
        std::fs::write(d.join("bad name.json"), "").unwrap();
        std::fs::write(d.join("dark.json"), "{}").unwrap();
        assert_eq!(list(&d), vec!["terminal", "dark", "light", "a", "b"], "a file named like a preset does not appear twice");
        assert_eq!(list(&d.join("missing")), vec!["terminal", "dark", "light"]);
    }

    #[test]
    fn load_knows_terminal_presets_files_and_says_what_is_wrong() {
        let d = scratch("load");
        assert_eq!(load("terminal", &d).unwrap(), Theme::terminal());
        assert!(load("dark", &d).unwrap().bg.is_some());
        std::fs::write(d.join("mine.json"), r##"{"colors":{"focusBorder":"#010203"}}"##).unwrap();
        assert_eq!(load("mine", &d).unwrap().accent, Color::Rgb(1, 2, 3));
        let e = load("gone", &d).unwrap_err();
        assert!(e.contains("themes/gone.json") && e.contains("not found"), "{}", e);
        std::fs::write(d.join("broken.json"), "{oops").unwrap();
        let e = load("broken", &d).unwrap_err();
        assert!(e.starts_with("themes/broken.json: "), "{}", e);
        assert!(load("../etc", &d).unwrap_err().contains("name"));
    }
}
