pub mod builtin;
pub mod cli;
pub mod fields;
pub mod guide;
pub mod host;
pub mod manifest;
pub mod protocol;
pub mod rpc;
pub mod serve;

use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub use host::{CliSink, PluginCmdId, PluginCommands, PluginEnv, PluginError, PluginHost, UiSink};
pub use cli::{commit_staged, discard_staged, install_local, stage_from_git, Staged};
pub use manifest::{Manifest, PluginProblem, SettingDecl, SettingKind};

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
    format!("api = 2\nname = \"{0}\"\nbuiltin = \"{0}\"\n", name)
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

/// `settings/set`: 플러그인이 자기 설정 하나를 바꾼다. 매니페스트에 선언된 키와 종류만 받아
/// `[plugins.<name>]`에 넣는다. 저장과 `config/changed` 알림은 부른 쪽(TUI의 `save_settings`)이 한다.
pub fn set_setting(config: &mut crate::config::Config, manifests: &[Manifest], plugin: &str, key: &str, value: &serde_json::Value) -> Result<(), String> {
    let m = manifests.iter().find(|m| m.name == plugin).ok_or_else(|| format!("no plugin named {}", plugin))?;
    let decl = m.settings.iter().find(|s| s.key == key).ok_or_else(|| format!("setting {} is not declared in {}'s plugin.toml", key, plugin))?;
    let v = match value {
        serde_json::Value::String(s) => toml::Value::String(s.clone()),
        serde_json::Value::Bool(b) => toml::Value::Boolean(*b),
        serde_json::Value::Number(n) if n.is_i64() => toml::Value::Integer(n.as_i64().unwrap_or_default()),
        other => return Err(format!("setting {} wants {}, got {}", key, decl.kind.name(), json_type_name(other))),
    };
    if !decl.kind.accepts(&v) {
        let expected = match &decl.kind {
            manifest::SettingKind::Choice(cs) => format!("one of {}", cs.join(", ")),
            k => k.name().to_string(),
        };
        return Err(format!("setting {} wants {}, got {}", key, expected, json_type_name(value)));
    }
    config.plugins.entry(plugin.to_string()).or_default().insert(key.to_string(), v);
    Ok(())
}

fn json_type_name(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => format!("{:?}", s),
        serde_json::Value::Bool(_) => "bool".into(),
        serde_json::Value::Number(_) => "number".into(),
        serde_json::Value::Array(_) => "array".into(),
        serde_json::Value::Object(_) => "object".into(),
        serde_json::Value::Null => "null".into(),
    }
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
        assert_eq!(stub_text("git-sync"), "api = 2\nname = \"git-sync\"\nbuiltin = \"git-sync\"\n");
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

    fn demo_manifest() -> Manifest {
        let text = "api = 2\nname = \"demo\"\nrun = \"x\"\n[[settings]]\nkey = \"links\"\ntype = \"string\"\ndefault = \"\"\n[[settings]]\nkey = \"limit\"\ntype = \"int\"\ndefault = 3\n[[settings]]\nkey = \"mode\"\ntype = \"choice\"\nchoices = [\"a\", \"b\"]\ndefault = \"a\"\n";
        let mut problems = vec![];
        manifest::parse_manifest(Path::new("/tmp/demo"), text, &mut problems).expect("manifest")
    }

    /// `settings/set`: 선언된 키에 맞는 종류의 값만 `[plugins.<name>]`에 들어간다.
    #[test]
    fn set_setting_writes_a_declared_value_into_the_plugin_table() {
        let mut config = crate::config::Config::default();
        let ms = vec![demo_manifest()];
        set_setting(&mut config, &ms, "demo", "links", &serde_json::json!("https://a\nhttps://b")).unwrap();
        set_setting(&mut config, &ms, "demo", "limit", &serde_json::json!(7)).unwrap();
        set_setting(&mut config, &ms, "demo", "mode", &serde_json::json!("b")).unwrap();
        let t = &config.plugins["demo"];
        assert_eq!(t["links"].as_str(), Some("https://a\nhttps://b"));
        assert_eq!(t["limit"].as_integer(), Some(7));
        assert_eq!(t["mode"].as_str(), Some("b"));
    }

    #[test]
    fn set_setting_refuses_an_unknown_plugin_key_or_kind_and_changes_nothing() {
        let mut config = crate::config::Config::default();
        let ms = vec![demo_manifest()];
        let e = set_setting(&mut config, &ms, "nope", "links", &serde_json::json!("x")).unwrap_err();
        assert!(e.contains("nope"), "{}", e);
        let e = set_setting(&mut config, &ms, "demo", "colour", &serde_json::json!("x")).unwrap_err();
        assert!(e.contains("colour") && e.contains("not declared"), "{}", e);
        let e = set_setting(&mut config, &ms, "demo", "limit", &serde_json::json!("seven")).unwrap_err();
        assert!(e.contains("int"), "{}", e);
        let e = set_setting(&mut config, &ms, "demo", "mode", &serde_json::json!("z")).unwrap_err();
        assert!(e.contains("one of a, b"), "{}", e);
        let e = set_setting(&mut config, &ms, "demo", "links", &serde_json::json!(["a"])).unwrap_err();
        assert!(e.contains("string"), "{}", e);
        assert!(config.plugins.is_empty(), "nothing written on refusal");
    }
}
