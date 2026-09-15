pub mod builtin;
pub mod cli;
pub mod guide;
pub mod host;
pub mod manifest;
pub mod protocol;
pub mod rpc;
pub mod serve;

use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub use host::{CliSink, NoUiSink, PluginCmdId, PluginCommands, PluginEnv, PluginError, PluginHost, UiSink};
pub use cli::{commit_staged, discard_staged, install_local, stage_from_git, Staged};
pub use manifest::{Activation, Manifest, PluginProblem, SettingDecl, SettingKind};

/// `config.toml`, `keymap.toml`과 같은 디렉토리 아래 `plugins/`.
pub fn plugins_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("bibox")
        .join("plugins")
}

/// 디렉토리 이름 알파벳순으로 매니페스트를 읽는다. 파일은 무시하고, `plugin.toml`이
/// 없는 디렉토리는 조용히 건너뛴다(doctor가 따로 보고한다). 디렉토리 자체가 없으면 빈 결과.
pub fn discover(dir: &Path) -> (Vec<Manifest>, Vec<PluginProblem>) {
    let mut manifests = Vec::new();
    let mut problems = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return (manifests, problems);
    };
    let mut dirs: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    for d in dirs {
        let manifest_path = d.join("plugin.toml");
        let Ok(text) = std::fs::read_to_string(&manifest_path) else { continue };
        if let Some(m) = manifest::parse_manifest(&d, &text, &mut problems) {
            manifests.push(m);
        }
    }
    (manifests, problems)
}

const SEEDED_HEADER: &str = "# plugins bibox has installed once; delete a line to have it re-created\n";

/// 사용자 디렉토리에 놓이는 스텁. 나머지는 바이너리 안의 매니페스트에서 온다.
pub fn stub_text(name: &str) -> String {
    format!("api = 1\nname = \"{0}\"\nbuiltin = \"{0}\"\n", name)
}

pub fn write_stub(plugins_dir: &Path, name: &str) -> std::io::Result<PathBuf> {
    let dir = plugins_dir.join(name);
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("plugin.toml"), stub_text(name))?;
    Ok(dir)
}

fn seeded_names(marker: &Path) -> HashSet<String> {
    std::fs::read_to_string(marker)
        .map(|s| {
            s.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// `.seeded`에 없는 내장 이름마다 스텁을 만들고 이름을 적는다. 사용자가 지운 스텁은
/// 이름이 남아 있어 다시 만들지 않는다. 쓰기에 실패하면 조용히 넘어간다.
pub fn seed_builtins(plugins_dir: &Path) -> Vec<String> {
    let names: Vec<&str> = builtin::BUILTINS.iter().filter(|b| b.seeded).map(|b| b.name).collect();
    seed_builtins_from(plugins_dir, &names)
}

pub fn seed_builtins_from(plugins_dir: &Path, names: &[&str]) -> Vec<String> {
    let marker = plugins_dir.join(".seeded");
    let seeded = seeded_names(&marker);
    let mut created = Vec::new();
    for name in names {
        if seeded.contains(*name) {
            continue;
        }
        if std::fs::create_dir_all(plugins_dir).is_err() {
            return created;
        }
        if !plugins_dir.join(name).exists() && write_stub(plugins_dir, name).is_err() {
            continue;
        }
        let mut text = if marker.exists() {
            std::fs::read_to_string(&marker).unwrap_or_default()
        } else {
            SEEDED_HEADER.to_string()
        };
        if !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(name);
        text.push('\n');
        if std::fs::write(&marker, text).is_err() {
            continue;
        }
        created.push(name.to_string());
    }
    created
}

/// 더 이상 쓰이지 않는 설정. TUI 시작 화면과 doctor에 나온다.
pub fn obsolete_config_problems(config: &crate::config::Config) -> Vec<PluginProblem> {
    let mut out = Vec::new();
    if config.git {
        out.push(PluginProblem::ObsoleteGitSetting);
    }
    for (name, table) in &config.plugins {
        if table.contains_key("enabled") {
            out.push(PluginProblem::ObsoleteEnabledFlag { name: name.clone() });
        }
    }
    out
}

impl host::PluginHost {
    /// 설정에서 호스트를 만든다. 매니페스트 문제는 돌려주되 호스트 구성은 계속한다.
    /// CLI 명령은 문제를 무시하고, TUI 시작 화면과 doctor가 보여준다.
    pub fn discover(config: &crate::config::Config) -> (host::PluginHost, Vec<PluginProblem>) {
        Self::discover_with(config, false)
    }

    /// `builtins_only`는 훅 안에서 불린 bibox용이다. 외부 플러그인은 `$BIBOX_BIN`을 되불러
    /// 무한 루프를 만들 수 있지만 내장은 그러지 않으므로, 내장만 두면 git-sync가 훅 안의
    /// 쓰기도 커밋한다.
    pub fn discover_with(config: &crate::config::Config, builtins_only: bool) -> (host::PluginHost, Vec<PluginProblem>) {
        seed_builtins(&plugins_dir());
        let (mut manifests, mut problems) = discover(&plugins_dir());
        if builtins_only {
            manifests.retain(|m| m.builtin.is_some());
        } else {
            problems.extend(obsolete_config_problems(config));
        }
        let host = host::PluginHost::new(manifests, crate::config::plugin_tables(config), host::PluginEnv::from_config(config));
        (host, problems)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discover_on_a_config_uses_the_plugins_dir_and_never_panics() {
        let config = crate::config::Config::default();
        let (host, problems) = host::PluginHost::discover(&config);
        // 실제 사용자 디렉토리를 읽으므로 내용은 단정하지 않는다. 죽지 않고 일관된 테이블을 만드는지만 본다.
        let _ = problems;
        for (id, c) in host.commands().iter() {
            assert_eq!(host.commands().find(&c.full_name()), Some(id));
        }
    }
    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("bibox-seed-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn seeding_creates_stubs_once_and_records_them() {
        let dir = temp("once");
        let created = seed_builtins_from(&dir, &["demo"]);
        assert_eq!(created, vec!["demo"]);
        assert_eq!(std::fs::read_to_string(dir.join("demo/plugin.toml")).unwrap(), stub_text("demo"));
        let marker = std::fs::read_to_string(dir.join(".seeded")).unwrap();
        assert!(marker.starts_with('#'), "first line is a comment: {}", marker);
        assert!(marker.lines().any(|l| l == "demo"));
        assert!(seed_builtins_from(&dir, &["demo"]).is_empty(), "second call is a no-op");
    }

    #[test]
    fn a_removed_stub_is_not_recreated_but_a_new_builtin_is_seeded() {
        let dir = temp("removed");
        seed_builtins_from(&dir, &["demo"]);
        std::fs::remove_dir_all(dir.join("demo")).unwrap();
        assert!(seed_builtins_from(&dir, &["demo"]).is_empty());
        assert!(!dir.join("demo").exists());
        assert_eq!(seed_builtins_from(&dir, &["demo", "other"]), vec!["other"]);
        assert!(dir.join("other/plugin.toml").exists());
        assert!(!dir.join("demo").exists());
    }

    /// pdf-view는 poppler가 필요해 기본으로 깔지 않는다. 원하는 사람이 Settings나 `plugin install pdf-view`로 켠다.
    #[test]
    fn only_builtins_marked_seeded_are_installed_by_default() {
        let dir = temp("default-set");
        let created = seed_builtins(&dir);
        assert_eq!(created, vec!["git-sync"]);
        assert!(!dir.join("pdf-view").exists());
        assert!(builtin::find("pdf-view").is_some(), "still a built-in, just opt-in");
    }

    #[test]
    fn seeding_a_missing_plugins_dir_creates_it() {
        let dir = temp("missing").join("nested").join("plugins");
        assert_eq!(seed_builtins_from(&dir, &["demo"]), vec!["demo"]);
        assert!(dir.join("demo/plugin.toml").exists());
    }

    #[test]
    fn an_existing_directory_is_left_alone_but_recorded() {
        let dir = temp("existing");
        std::fs::create_dir_all(dir.join("demo")).unwrap();
        std::fs::write(dir.join("demo/plugin.toml"), "user content\n").unwrap();
        assert_eq!(seed_builtins_from(&dir, &["demo"]), vec!["demo"]);
        assert_eq!(std::fs::read_to_string(dir.join("demo/plugin.toml")).unwrap(), "user content\n");
    }

    #[test]
    fn stub_text_is_three_lines() {
        assert_eq!(stub_text("git-sync"), "api = 1\nname = \"git-sync\"\nbuiltin = \"git-sync\"\n");
    }
    #[test]
    fn obsolete_settings_are_reported_once_each() {
        let text = "bibox_dir = \"/tmp/b\"\nsearch_case_sensitive = false\ndefault_page_size = 20\ngit = true\n[plugins.a]\nenabled = false\n[plugins.b]\nenabled = true\n[plugins.c]\nmodel = \"x\"\n";
        let config: crate::config::Config = toml::from_str(text).unwrap();
        let p = obsolete_config_problems(&config);
        assert_eq!(p.len(), 3, "{:?}", p);
        assert!(p.contains(&PluginProblem::ObsoleteGitSetting));
        assert!(p.contains(&PluginProblem::ObsoleteEnabledFlag { name: "a".into() }));
        assert!(p.contains(&PluginProblem::ObsoleteEnabledFlag { name: "b".into() }));
    }

    #[test]
    fn a_clean_config_has_no_obsolete_problems() {
        let config = crate::config::Config::default();
        assert!(obsolete_config_problems(&config).is_empty());
    }
}
