//! 바이너리 안에 있는 플러그인들. 사용자 디렉토리에는 `builtin = "<name>"` 스텁만 있고,
//! 매니페스트와 코드는 여기 있다. bibox가 `bibox plugin run <name>`으로 자기 자신을 띄운다.

pub mod git_sync;
pub mod pdf_view;

#[derive(Clone, Copy)]
pub struct Builtin {
    pub name: &'static str,
    /// `run`/`builtin` 없는 매니페스트 TOML. 파싱 뒤 `run`과 `version`은 bibox가 채운다.
    pub manifest: &'static str,
    /// `serve()`를 부르는 진입점. 돌아오면 프로세스가 끝난다.
    pub run: fn(),
    /// 첫 실행 때 스텁을 만들어 켤 것인가. 외부 도구가 필요한 것(pdf-view의 poppler)은 옵트인.
    pub seeded: bool,
}

pub const BUILTINS: &[Builtin] = &[git_sync::BUILTIN, pdf_view::BUILTIN];

pub fn find(name: &str) -> Option<&'static Builtin> {
    BUILTINS.iter().find(|b| b.name == name)
}

/// `bibox plugin run <name>`. 모르는 이름이면 종료 코드 2.
pub fn run(name: &str) -> anyhow::Result<()> {
    match find(name) {
        Some(b) => {
            (b.run)();
            Ok(())
        }
        None => {
            eprintln!("bibox: unknown built-in plugin \"{}\"", name);
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_knows_every_registered_builtin_and_nothing_else() {
        for b in BUILTINS {
            assert_eq!(find(b.name).map(|x| x.name), Some(b.name));
        }
        assert!(find("definitely-not-a-builtin").is_none());
    }

    #[test]
    fn builtin_names_are_valid_plugin_names() {
        for b in BUILTINS {
            let first = b.name.chars().next().unwrap();
            assert!(first.is_ascii_lowercase() || first.is_ascii_digit(), "{}", b.name);
            assert!(b.name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'), "{}", b.name);
        }
    }
}
