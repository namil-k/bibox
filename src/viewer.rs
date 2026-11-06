//! PDF를 여는 방법. `pdf_viewer`가 없으면 OS 기본 앱(macOS `open`, 그 밖 `xdg-open`).
//! 있으면 첫 단어가 실행 파일인 명령(`zathura`, `open -a Skim`, `/usr/bin/evince`)이거나,
//! macOS에서는 앱 이름(`Skim`, `Adobe Acrobat Reader`)이다. 파일 경로는 맨 뒤에 붙는다.

use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};

#[derive(Debug, PartialEq)]
enum Launch {
    /// OS 기본 앱
    Default,
    /// 실행 파일과 인자
    Command { program: String, args: Vec<String> },
    /// macOS 앱 이름이나 .app 경로, `open -a`로 연다
    MacApp(String),
}

/// 셸 없이 명령을 단어로 나눈다. 작은따옴표와 큰따옴표로 공백을 묶을 수 있다.
fn split_words(s: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut in_word = false;
    let mut quote: Option<char> = None;
    for c in s.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => cur.push(c),
            None if c == '"' || c == '\'' => {
                quote = Some(c);
                in_word = true;
            }
            None if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut cur));
                    in_word = false;
                }
            }
            None => {
                cur.push(c);
                in_word = true;
            }
        }
    }
    if in_word {
        words.push(cur);
    }
    words
}

fn plan(viewer: Option<&str>, is_mac: bool, is_executable: impl Fn(&str) -> bool) -> std::result::Result<Launch, String> {
    let v = viewer.map(str::trim).unwrap_or("");
    let words = split_words(v);
    let Some(first) = words.first() else { return Ok(Launch::Default) };
    if is_executable(first) {
        return Ok(Launch::Command { program: first.clone(), args: words[1..].to_vec() });
    }
    if is_mac {
        // 앱 이름에는 공백이 흔하다("Adobe Acrobat Reader"). 따옴표만 벗기고 통째로 넘긴다.
        return Ok(Launch::MacApp(words.join(" ")));
    }
    Err(format!("PDF viewer \"{}\" is not a command on PATH (pdf_viewer in config.toml)", first))
}

fn is_executable(name: &str) -> bool {
    fn runnable(p: &Path) -> bool {
        let Ok(meta) = std::fs::metadata(p) else { return false };
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            meta.is_file() && meta.permissions().mode() & 0o111 != 0
        }
        #[cfg(not(unix))]
        {
            meta.is_file()
        }
    }
    if name.contains('/') {
        return runnable(&crate::config::expand_tilde(Path::new(name)));
    }
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| runnable(&dir.join(name))))
        .unwrap_or(false)
}

/// `pdf_viewer` 설정대로 PDF를 연다. 뷰어의 출력은 버린다(TUI 화면을 깨지 않게).
pub fn open_pdf(viewer: Option<&str>, path: &Path) -> Result<()> {
    let launch = plan(viewer, cfg!(target_os = "macos"), is_executable).map_err(anyhow::Error::msg)?;
    match launch {
        Launch::Default => {
            let program = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
            quiet(Command::new(program).arg(path)).spawn().with_context(|| format!("Failed to run '{}'", program))?;
        }
        Launch::Command { program, args } => {
            quiet(Command::new(&program).args(&args).arg(path))
                .spawn()
                .with_context(|| format!("Failed to launch PDF viewer '{}'", program))?;
        }
        Launch::MacApp(app) => {
            // `open -a`는 앱에 넘기고 바로 끝난다. 앱이 없으면 실패 코드로 끝나므로 기다려서 확인한다.
            let out = Command::new("open").arg("-a").arg(&app).arg(path).stdin(Stdio::null()).output().context("Failed to run 'open'")?;
            if !out.status.success() {
                bail!("macOS has no app named \"{}\" (pdf_viewer in config.toml)", app);
            }
        }
    }
    Ok(())
}

fn quiet(cmd: &mut Command) -> &mut Command {
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on_path(name: &str) -> bool {
        matches!(name, "open" | "zathura" | "/usr/bin/evince")
    }

    #[test]
    fn no_viewer_or_a_blank_one_means_the_system_default() {
        assert_eq!(plan(None, true, on_path), Ok(Launch::Default));
        assert_eq!(plan(Some("  "), false, on_path), Ok(Launch::Default));
    }

    #[test]
    fn a_command_on_path_runs_with_its_arguments() {
        assert_eq!(plan(Some("zathura"), false, on_path), Ok(Launch::Command { program: "zathura".into(), args: vec![] }));
        assert_eq!(
            plan(Some("open -a \"Adobe Acrobat Reader\""), true, on_path),
            Ok(Launch::Command { program: "open".into(), args: vec!["-a".into(), "Adobe Acrobat Reader".into()] })
        );
        assert_eq!(plan(Some("/usr/bin/evince"), false, on_path), Ok(Launch::Command { program: "/usr/bin/evince".into(), args: vec![] }));
    }

    #[test]
    fn on_macos_an_app_name_opens_with_open_a() {
        assert_eq!(plan(Some("Skim"), true, on_path), Ok(Launch::MacApp("Skim".into())));
        assert_eq!(plan(Some("Adobe Acrobat Reader"), true, on_path), Ok(Launch::MacApp("Adobe Acrobat Reader".into())));
        assert_eq!(plan(Some("/Applications/PDF Expert.app"), true, on_path), Ok(Launch::MacApp("/Applications/PDF Expert.app".into())));
    }

    #[test]
    fn elsewhere_an_unknown_viewer_is_an_error_that_names_it() {
        let err = plan(Some("okular --unique"), false, on_path).unwrap_err();
        assert!(err.contains("\"okular\""), "{}", err);
    }

    #[test]
    fn words_split_on_spaces_and_keep_quoted_groups() {
        assert_eq!(split_words("a  b"), vec!["a", "b"]);
        assert_eq!(split_words("open -a 'PDF Expert'"), vec!["open", "-a", "PDF Expert"]);
        assert_eq!(split_words("x \"\""), vec!["x", ""]);
    }
}
