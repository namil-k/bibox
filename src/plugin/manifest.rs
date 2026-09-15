use serde::Deserialize;
use std::path::{Path, PathBuf};

use crate::keymap::{parse_key, KeyPress, LayerId};

/// bibox가 이해하는 프로토콜 버전. 매니페스트의 `api`가 이 값이 아니면 로드하지 않는다.
pub const SUPPORTED_API: u32 = 1;

// ── 파일 모양 (serde) ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum KeyField {
    One(String),
    Many(Vec<String>),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct CommandFile {
    id: String,
    desc: String,
    #[serde(default)]
    key: Option<KeyField>,
    #[serde(default)]
    layers: Option<Vec<String>>,
    #[serde(default)]
    menu: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct HookFile {
    on: String,
    run: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct CliFile {
    run: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct SettingFile {
    key: String,
    #[serde(rename = "type")]
    kind: String,
    default: toml::Value,
    #[serde(default)]
    desc: Option<String>,
    #[serde(default)]
    choices: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestFile {
    api: u32,
    name: String,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    run: Option<String>,
    #[serde(default)]
    builtin: Option<String>,
    #[serde(default)]
    commands: Vec<CommandFile>,
    #[serde(default)]
    hooks: Vec<HookFile>,
    #[serde(default)]
    cli: Option<CliFile>,
    #[serde(default)]
    settings: Vec<SettingFile>,
    #[serde(default)]
    tabs: Vec<TabFile>,
}

/// `[[tabs]]`. 미리보기 패널의 탭 하나: 제목과 그 내용을 만드는 명령.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct TabFile {
    title: String,
    run: String,
}

// ── 검증된 모양 ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HookKind {
    BeforeAdd,
    AfterWrite,
    AfterNoteSave,
}

impl HookKind {
    pub fn parse(s: &str) -> Option<HookKind> {
        match s {
            "before_add" => Some(HookKind::BeforeAdd),
            "after_write" => Some(HookKind::AfterWrite),
            "after_note_save" => Some(HookKind::AfterNoteSave),
            _ => None,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            HookKind::BeforeAdd => "before_add",
            HookKind::AfterWrite => "after_write",
            HookKind::AfterNoteSave => "after_note_save",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Command {
    pub id: String,
    pub desc: String,
    pub key: Option<Vec<KeyPress>>,
    pub layers: Vec<LayerId>,
    pub menu: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SettingKind {
    Bool,
    Int,
    Str,
    Choice(Vec<String>),
}

impl SettingKind {
    pub fn name(&self) -> &'static str {
        match self {
            SettingKind::Bool => "bool",
            SettingKind::Int => "int",
            SettingKind::Str => "string",
            SettingKind::Choice(_) => "choice",
        }
    }

    /// 값이 이 종류에 맞는가. choice는 선택지 안에 있어야 한다.
    pub fn accepts(&self, v: &toml::Value) -> bool {
        match self {
            SettingKind::Bool => v.is_bool(),
            SettingKind::Int => v.is_integer(),
            SettingKind::Str => v.is_str(),
            SettingKind::Choice(cs) => v.as_str().map(|s| cs.iter().any(|c| c == s)).unwrap_or(false),
        }
    }
}

/// `[[settings]]` 하나. bibox가 화면에 그리고 doctor가 config.toml을 대조하는 근거.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingDecl {
    pub key: String,
    pub kind: SettingKind,
    pub default: toml::Value,
    pub desc: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Hook {
    pub on: HookKind,
    pub run: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Manifest {
    pub name: String,
    pub version: Option<String>,
    pub description: Option<String>,
    pub run: Vec<String>,
    pub commands: Vec<Command>,
    pub hooks: Vec<Hook>,
    pub cli: Option<Vec<String>>,
    pub settings: Vec<SettingDecl>,
    pub tabs: Vec<Tab>,
    pub builtin: Option<String>,
    pub dir: PathBuf,
}

/// 미리보기 탭. `run`은 이 플러그인의 command id. 호스트가 `trigger = "tab"`으로 부른다.
#[derive(Debug, Clone, PartialEq)]
pub struct Tab {
    pub title: String,
    pub run: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PluginProblem {
    /// 파싱 또는 검증 실패. 플러그인 전체를 로드하지 않는다.
    Manifest { plugin: String, detail: String },
    /// 기본 키 표기 오류. 키만 버린다.
    BadKey { plugin: String, command: String, token: String },
    /// `layers`의 모르는 값. 그 값만 버린다.
    UnknownLayer { plugin: String, command: String, layer: String },
    /// 모르는 이벤트 또는 없는 command id. 그 훅만 버린다.
    BadHook { plugin: String, detail: String },
    /// doctor 전용. `plugins/` 아래 디렉토리에 `plugin.toml`이 없다.
    NoManifest { dir: String },
    /// doctor 전용. `plugins/<name>`이 심링크인데 대상이 없다(`plugin install ./path` 뒤 그 디렉토리를 지운 경우).
    DanglingLink { name: String, target: String },
    /// doctor 전용. `[plugins.x]`가 있는데 x 플러그인이 없다.
    ConfigWithoutPlugin { name: String },
    /// doctor 전용. `run`/`[cli].run`의 첫 토큰이 PATH에 없다.
    ExecutableMissing { plugin: String, program: String },
    /// doctor 전용. 플러그인이 쓰는 외부 도구가 PATH에 없다. 플러그인은 뜨지만 그 기능이 안내 문구를 낸다.
    ToolMissing { plugin: String, program: String, hint: String },
    /// doctor 전용. 이름이 내장 서브커맨드와 같아 `bibox <name>`이 절대 폴스루되지 않는다.
    NameCollidesWithSubcommand { plugin: String },
    /// `config.toml`의 `git = true`. git-sync 플러그인이 대신한다.
    ObsoleteGitSetting,
    /// `[plugins.x] enabled`. 지우기/깔기만 남았다.
    ObsoleteEnabledFlag { name: String },
    /// doctor 전용. `[plugins.x] key`의 값이 `[[settings]]` 선언과 다른 타입이다.
    SettingTypeMismatch { plugin: String, key: String, expected: String, found: String },
    /// doctor 전용. `[plugins.x] key`가 선언에 없다. 선언이 하나라도 있는 플러그인만 검사한다.
    UndeclaredSetting { plugin: String, key: String, suggestion: Option<String> },
}

impl PluginProblem {
    #[cfg(test)]
    pub fn is_error(&self) -> bool {
        matches!(self, PluginProblem::Manifest { .. })
    }

    pub fn plugin(&self) -> &str {
        match self {
            PluginProblem::Manifest { plugin, .. }
            | PluginProblem::BadKey { plugin, .. }
            | PluginProblem::UnknownLayer { plugin, .. }
            | PluginProblem::BadHook { plugin, .. }
            | PluginProblem::ExecutableMissing { plugin, .. }
            | PluginProblem::ToolMissing { plugin, .. }
            | PluginProblem::NameCollidesWithSubcommand { plugin }
            | PluginProblem::SettingTypeMismatch { plugin, .. }
            | PluginProblem::UndeclaredSetting { plugin, .. } => plugin,
            PluginProblem::NoManifest { dir } => dir,
            PluginProblem::DanglingLink { name, .. } => name,
            PluginProblem::ConfigWithoutPlugin { name } => name,
            PluginProblem::ObsoleteGitSetting => "config",
            PluginProblem::ObsoleteEnabledFlag { name } => name,
        }
    }
}

fn valid_name(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() || c.is_ascii_digit() => {}
        _ => return false,
    }
    s.len() <= 64 && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn valid_command_id(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

fn valid_setting_key(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// 디렉토리 이름을 플러그인 이름의 근거로 쓴다. 매니페스트가 깨져 `name`을 못 읽어도
/// 문제를 어느 플러그인 것으로 돌릴지는 알아야 한다.
fn dir_name(dir: &Path) -> String {
    dir.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()
}

/// 파싱 이후 문제는 전부 `problems`에 모은다. 오류 등급 문제가 하나라도 있으면 `None`.
/// TOML 문법 오류는 serde가 첫 오류에서 멈추므로 하나만 나온다(키맵과 같은 한계).
pub fn parse_manifest(dir: &Path, text: &str, problems: &mut Vec<PluginProblem>) -> Option<Manifest> {
    parse_manifest_with(dir, text, problems, crate::plugin::builtin::BUILTINS)
}

/// `builtins`를 인자로 받는 것은 테스트가 가짜 내장 목록을 넣기 위해서다.
pub fn parse_manifest_with(
    dir: &Path,
    text: &str,
    problems: &mut Vec<PluginProblem>,
    builtins: &[crate::plugin::builtin::Builtin],
) -> Option<Manifest> {
    let plugin = dir_name(dir);
    let err = |detail: String| PluginProblem::Manifest { plugin: plugin.clone(), detail };

    let file: ManifestFile = match toml::from_str(text) {
        Ok(f) => f,
        Err(e) => {
            problems.push(err(e.to_string()));
            return None;
        }
    };

    match (&file.run, &file.builtin) {
        (Some(_), Some(_)) => {
            problems.push(err("run and builtin are mutually exclusive".to_string()));
            None
        }
        (None, None) => {
            problems.push(err("run or builtin is required".to_string()));
            None
        }
        (None, Some(b)) => expand_stub(dir, &file, b, problems, builtins),
        (Some(_), None) => build(dir, file, None, problems),
    }
}

/// 스텁을 바이너리 안의 매니페스트로 바꾼다. 스텁에는 api, name, builtin만 있어야 한다.
fn expand_stub(
    dir: &Path,
    file: &ManifestFile,
    builtin: &str,
    problems: &mut Vec<PluginProblem>,
    builtins: &[crate::plugin::builtin::Builtin],
) -> Option<Manifest> {
    let plugin = dir_name(dir);
    let err = |detail: String| PluginProblem::Manifest { plugin: plugin.clone(), detail };

    let extra = file.version.is_some() || file.description.is_some() || !file.commands.is_empty() || !file.hooks.is_empty() || file.cli.is_some() || !file.settings.is_empty() || !file.tabs.is_empty();
    if extra {
        problems.push(err("a built-in stub carries only api, name and builtin".to_string()));
        return None;
    }
    if file.api != SUPPORTED_API {
        problems.push(err(format!("bibox supports api {}, plugin declares {}", SUPPORTED_API, file.api)));
        return None;
    }
    if builtin != file.name {
        problems.push(err(format!("builtin \"{}\" must equal name \"{}\"", builtin, file.name)));
        return None;
    }
    if file.name != plugin {
        problems.push(err(format!("name \"{}\" must equal the directory name \"{}\"", file.name, plugin)));
        return None;
    }
    let Some(b) = builtins.iter().find(|b| b.name == builtin) else {
        problems.push(err(format!("unknown built-in plugin \"{}\" (downgraded bibox?)", builtin)));
        return None;
    };
    let exe = match std::env::current_exe() {
        Ok(p) => p.to_string_lossy().to_string(),
        Err(e) => {
            problems.push(err(format!("cannot locate the bibox executable: {}", e)));
            return None;
        }
    };
    let embedded: ManifestFile = match toml::from_str(b.manifest) {
        Ok(f) => f,
        Err(e) => {
            // bibox 자체의 버그다. every_real_builtin_manifest_expands_cleanly 테스트가 막는다.
            problems.push(err(format!("embedded manifest for {} is broken: {}", builtin, e)));
            return None;
        }
    };
    let run = vec![exe, "plugin".to_string(), "run".to_string(), builtin.to_string()];
    let mut m = build(dir, embedded, Some(run), problems)?;
    m.builtin = Some(builtin.to_string());
    m.version = Some(env!("CARGO_PKG_VERSION").to_string());
    Some(m)
}

/// 검증 본체. `run_override`가 있으면(내장) 파일의 `run`을 보지 않는다.
fn build(dir: &Path, file: ManifestFile, run_override: Option<Vec<String>>, problems: &mut Vec<PluginProblem>) -> Option<Manifest> {
    let plugin = dir_name(dir);
    let err = |detail: String| PluginProblem::Manifest { plugin: plugin.clone(), detail };

    let mut fatal = false;
    if file.api != SUPPORTED_API {
        problems.push(err(format!("bibox supports api {}, plugin declares {}", SUPPORTED_API, file.api)));
        fatal = true;
    }
    if file.name != plugin {
        problems.push(err(format!("name \"{}\" must equal the directory name \"{}\"", file.name, plugin)));
        fatal = true;
    } else if !valid_name(&file.name) {
        problems.push(err(format!("name \"{}\" must match [a-z0-9][a-z0-9-]{{0,63}}", file.name)));
        fatal = true;
    }
    let run: Vec<String> = match run_override {
        Some(r) => r,
        None => file.run.as_deref().unwrap_or("").split_whitespace().map(str::to_string).collect(),
    };
    if run.is_empty() {
        problems.push(err("run must name a program".to_string()));
        fatal = true;
    }
    let cli = match &file.cli {
        Some(c) => {
            let argv: Vec<String> = c.run.split_whitespace().map(str::to_string).collect();
            if argv.is_empty() {
                problems.push(err("[cli] run must name a program".to_string()));
                fatal = true;
            }
            Some(argv)
        }
        None => None,
    };

    let mut commands: Vec<Command> = Vec::new();
    for cf in &file.commands {
        if !valid_command_id(&cf.id) {
            problems.push(err(format!("command id \"{}\" must match [a-z0-9_]+", cf.id)));
            fatal = true;
            continue;
        }
        if commands.iter().any(|c| c.id == cf.id) {
            problems.push(err(format!("duplicate command id \"{}\"", cf.id)));
            fatal = true;
            continue;
        }
        let key = match &cf.key {
            None => None,
            Some(kf) => {
                let tokens: Vec<&str> = match kf {
                    KeyField::One(s) => vec![s.as_str()],
                    KeyField::Many(v) => v.iter().map(String::as_str).collect(),
                };
                let mut keys = Vec::new();
                let mut ok = true;
                for t in tokens {
                    match parse_key(t) {
                        Ok(k) => keys.push(k),
                        Err(e) => {
                            problems.push(PluginProblem::BadKey {
                                plugin: plugin.clone(),
                                command: cf.id.clone(),
                                token: e.token,
                            });
                            ok = false;
                            break;
                        }
                    }
                }
                if ok && !keys.is_empty() { Some(keys) } else { None }
            }
        };
        let layers = match &cf.layers {
            None => vec![LayerId::Collections, LayerId::Entries, LayerId::Preview],
            Some(names) => {
                let mut out = Vec::new();
                for n in names {
                    match n.as_str() {
                        "collections" => out.push(LayerId::Collections),
                        "entries" => out.push(LayerId::Entries),
                        "preview" => out.push(LayerId::Preview),
                        other => problems.push(PluginProblem::UnknownLayer {
                            plugin: plugin.clone(),
                            command: cf.id.clone(),
                            layer: other.to_string(),
                        }),
                    }
                }
                if out.is_empty() {
                    vec![LayerId::Collections, LayerId::Entries, LayerId::Preview]
                } else {
                    out
                }
            }
        };
        commands.push(Command { id: cf.id.clone(), desc: cf.desc.clone(), key, layers, menu: cf.menu });
    }

    let mut hooks = Vec::new();
    for hf in &file.hooks {
        let Some(on) = HookKind::parse(&hf.on) else {
            problems.push(PluginProblem::BadHook {
                plugin: plugin.clone(),
                detail: format!("unknown hook \"{}\"", hf.on),
            });
            continue;
        };
        if !commands.iter().any(|c| c.id == hf.run) {
            problems.push(PluginProblem::BadHook {
                plugin: plugin.clone(),
                detail: format!("hook {} refers to unknown command \"{}\"", hf.on, hf.run),
            });
            continue;
        }
        hooks.push(Hook { on, run: hf.run.clone() });
    }

    let mut settings: Vec<SettingDecl> = Vec::new();
    for sf in &file.settings {
        if !valid_setting_key(&sf.key) {
            problems.push(err(format!("setting key \"{}\" must match [A-Za-z0-9_-]+", sf.key)));
            fatal = true;
            continue;
        }
        if settings.iter().any(|s| s.key == sf.key) {
            problems.push(err(format!("duplicate setting key \"{}\"", sf.key)));
            fatal = true;
            continue;
        }
        let kind = match (sf.kind.as_str(), &sf.choices) {
            ("bool", None) => SettingKind::Bool,
            ("int", None) => SettingKind::Int,
            ("string", None) => SettingKind::Str,
            ("choice", Some(cs)) if !cs.is_empty() => SettingKind::Choice(cs.clone()),
            ("choice", _) => {
                problems.push(err(format!("setting \"{}\": type = \"choice\" needs a non-empty choices list", sf.key)));
                fatal = true;
                continue;
            }
            ("bool", Some(_)) | ("int", Some(_)) | ("string", Some(_)) => {
                problems.push(err(format!("setting \"{}\": choices is only for type = \"choice\"", sf.key)));
                fatal = true;
                continue;
            }
            (other, _) => {
                problems.push(err(format!("setting \"{}\": unknown type \"{}\" (bool, int, string, choice)", sf.key, other)));
                fatal = true;
                continue;
            }
        };
        if !kind.accepts(&sf.default) {
            problems.push(err(format!("setting \"{}\": default {} does not match type {}", sf.key, sf.default, kind.name())));
            fatal = true;
            continue;
        }
        settings.push(SettingDecl { key: sf.key.clone(), kind, default: sf.default.clone(), desc: sf.desc.clone() });
    }

    // 탭 제목은 탭 줄에 그대로 놓이므로 짧게. 명령은 위에서 검증된 것 중 하나여야 한다.
    let mut tabs: Vec<Tab> = Vec::new();
    for tf in &file.tabs {
        let n = tf.title.chars().count();
        if n == 0 || n > 12 {
            problems.push(err(format!("tab title \"{}\" must be 1 to 12 characters", tf.title)));
            fatal = true;
            continue;
        }
        if !commands.iter().any(|c| c.id == tf.run) {
            problems.push(err(format!("tab \"{}\" refers to unknown command \"{}\"", tf.title, tf.run)));
            fatal = true;
            continue;
        }
        tabs.push(Tab { title: tf.title.clone(), run: tf.run.clone() });
    }

    if fatal {
        return None;
    }
    Some(Manifest {
        name: file.name,
        version: file.version,
        description: file.description,
        run,
        commands,
        hooks,
        cli,
        settings,
        tabs,
        builtin: None,
        dir: dir.to_path_buf(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers};
    use std::path::PathBuf;

    const OK: &str = r#"
api = 1
name = "entry-tidy"
version = "0.1.0"
description = "Normalize entries"
run = "python3 main.py"

[[commands]]
id = "tidy"
desc = "Normalize the current entry"
key = "="
layers = ["entries", "preview"]
menu = true

[[hooks]]
on = "before_add"
run = "tidy"

[cli]
run = "python3 cli.py"
"#;

    fn dir(name: &str) -> PathBuf {
        PathBuf::from("/tmp/plugins").join(name)
    }

    #[test]
    fn a_valid_manifest_parses_into_every_field() {
        let mut problems = vec![];
        let m = parse_manifest(&dir("entry-tidy"), OK, &mut problems).expect("manifest");
        assert!(problems.is_empty(), "{:?}", problems);
        assert_eq!(m.name, "entry-tidy");
        assert_eq!(m.version.as_deref(), Some("0.1.0"));
        assert_eq!(m.run, vec!["python3", "main.py"]);
        assert_eq!(m.commands.len(), 1);
        let c = &m.commands[0];
        assert_eq!(c.id, "tidy");
        assert_eq!(c.key, Some(vec![KeyPress::new(KeyCode::Char('='), KeyModifiers::NONE)]));
        assert_eq!(c.layers, vec![LayerId::Entries, LayerId::Preview]);
        assert!(c.menu);
        assert_eq!(m.hooks, vec![Hook { on: HookKind::BeforeAdd, run: "tidy".into() }]);
        assert_eq!(m.cli, Some(vec!["python3".to_string(), "cli.py".to_string()]));
        assert_eq!(m.dir, dir("entry-tidy"));
    }

    #[test]
    fn layers_default_to_all_three_and_menu_defaults_to_false() {
        let text = "api = 1\nname = \"x\"\nrun = \"sh run.sh\"\n[[commands]]\nid = \"a\"\ndesc = \"A\"\n";
        let mut problems = vec![];
        let m = parse_manifest(&dir("x"), text, &mut problems).unwrap();
        assert_eq!(m.commands[0].layers, vec![LayerId::Collections, LayerId::Entries, LayerId::Preview]);
        assert!(!m.commands[0].menu);
        assert_eq!(m.commands[0].key, None);
    }

    #[test]
    fn a_key_sequence_is_accepted_as_a_list() {
        let text = "api = 1\nname = \"x\"\nrun = \"sh run.sh\"\n[[commands]]\nid = \"a\"\ndesc = \"A\"\nkey = [\"g\", \"t\"]\n";
        let mut problems = vec![];
        let m = parse_manifest(&dir("x"), text, &mut problems).unwrap();
        assert_eq!(m.commands[0].key.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn a_missing_required_field_is_a_manifest_error_and_yields_nothing() {
        let text = "api = 1\nname = \"x\"\n"; // run 누락
        let mut problems = vec![];
        assert!(parse_manifest(&dir("x"), text, &mut problems).is_none());
        assert_eq!(problems.len(), 1);
        assert!(matches!(&problems[0], PluginProblem::Manifest { plugin, detail } if plugin == "x" && detail.contains("run")));
        assert!(problems[0].is_error());
    }

    #[test]
    fn an_unknown_field_is_a_manifest_error() {
        let text = "api = 1\nname = \"x\"\nrun = \"sh\"\ncolour = \"red\"\n";
        let mut problems = vec![];
        assert!(parse_manifest(&dir("x"), text, &mut problems).is_none());
        assert!(matches!(&problems[0], PluginProblem::Manifest { detail, .. } if detail.contains("colour")));
    }

    #[test]
    fn an_unsupported_api_version_is_rejected() {
        let text = "api = 2\nname = \"x\"\nrun = \"sh\"\n";
        let mut problems = vec![];
        assert!(parse_manifest(&dir("x"), text, &mut problems).is_none());
        assert!(matches!(&problems[0], PluginProblem::Manifest { detail, .. } if detail.contains("api 1") && detail.contains("2")));
    }

    #[test]
    fn name_must_match_the_directory_and_the_charset() {
        let mut problems = vec![];
        assert!(parse_manifest(&dir("other"), "api = 1\nname = \"x\"\nrun = \"sh\"\n", &mut problems).is_none());
        assert!(matches!(&problems[0], PluginProblem::Manifest { detail, .. } if detail.contains("directory")));

        let mut problems = vec![];
        assert!(parse_manifest(&dir("Bad_Name"), "api = 1\nname = \"Bad_Name\"\nrun = \"sh\"\n", &mut problems).is_none());
        assert!(matches!(&problems[0], PluginProblem::Manifest { detail, .. } if detail.contains("a-z")));
    }

    #[test]
    fn an_empty_run_is_an_error() {
        let mut problems = vec![];
        assert!(parse_manifest(&dir("x"), "api = 1\nname = \"x\"\nrun = \"   \"\n", &mut problems).is_none());
        assert!(matches!(&problems[0], PluginProblem::Manifest { detail, .. } if detail.contains("run")));
    }

    #[test]
    fn duplicate_or_malformed_command_ids_are_errors() {
        let dup = "api = 1\nname = \"x\"\nrun = \"sh\"\n[[commands]]\nid = \"a\"\ndesc = \"A\"\n[[commands]]\nid = \"a\"\ndesc = \"B\"\n";
        let mut problems = vec![];
        assert!(parse_manifest(&dir("x"), dup, &mut problems).is_none());
        assert!(matches!(&problems[0], PluginProblem::Manifest { detail, .. } if detail.contains("duplicate")));

        let bad = "api = 1\nname = \"x\"\nrun = \"sh\"\n[[commands]]\nid = \"Tidy-Up\"\ndesc = \"A\"\n";
        let mut problems = vec![];
        assert!(parse_manifest(&dir("x"), bad, &mut problems).is_none());
        assert!(matches!(&problems[0], PluginProblem::Manifest { detail, .. } if detail.contains("a-z0-9_")));
    }

    #[test]
    fn a_bad_key_is_a_warning_and_only_drops_the_key() {
        let text = "api = 1\nname = \"x\"\nrun = \"sh\"\n[[commands]]\nid = \"a\"\ndesc = \"A\"\nkey = \"<Bogus>\"\n";
        let mut problems = vec![];
        let m = parse_manifest(&dir("x"), text, &mut problems).unwrap();
        assert_eq!(m.commands[0].key, None);
        assert_eq!(problems, vec![PluginProblem::BadKey { plugin: "x".into(), command: "a".into(), token: "<Bogus>".into() }]);
        assert!(!problems[0].is_error());
    }

    #[test]
    fn an_unknown_layer_is_a_warning_and_the_rest_survive() {
        let text = "api = 1\nname = \"x\"\nrun = \"sh\"\n[[commands]]\nid = \"a\"\ndesc = \"A\"\nlayers = [\"entries\", \"sidebar\"]\n";
        let mut problems = vec![];
        let m = parse_manifest(&dir("x"), text, &mut problems).unwrap();
        assert_eq!(m.commands[0].layers, vec![LayerId::Entries]);
        assert!(matches!(&problems[0], PluginProblem::UnknownLayer { layer, .. } if layer == "sidebar"));
    }

    #[test]
    fn a_hook_with_an_unknown_event_or_command_is_a_warning() {
        let text = "api = 1\nname = \"x\"\nrun = \"sh\"\n[[commands]]\nid = \"a\"\ndesc = \"A\"\n[[hooks]]\non = \"on_boot\"\nrun = \"a\"\n[[hooks]]\non = \"after_write\"\nrun = \"zzz\"\n";
        let mut problems = vec![];
        let m = parse_manifest(&dir("x"), text, &mut problems).unwrap();
        assert!(m.hooks.is_empty());
        assert_eq!(problems.len(), 2);
        assert!(problems.iter().all(|p| matches!(p, PluginProblem::BadHook { .. })));
    }

    #[test]
    fn hook_kind_round_trips_its_three_names() {
        for name in ["before_add", "after_write", "after_note_save"] {
            assert_eq!(HookKind::parse(name).unwrap().name(), name);
        }
        assert_eq!(HookKind::parse("after_add"), None);
    }

    #[test]
    fn discover_reads_sorted_directories_and_skips_files_and_manifestless_dirs() {
        let root = std::env::temp_dir().join(format!("bibox-discover-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("zeta")).unwrap();
        std::fs::create_dir_all(root.join("alpha")).unwrap();
        std::fs::create_dir_all(root.join("empty")).unwrap();
        std::fs::write(root.join("stray.txt"), "x").unwrap();
        std::fs::write(root.join("zeta/plugin.toml"), "api = 1\nname = \"zeta\"\nrun = \"sh\"\n").unwrap();
        std::fs::write(root.join("alpha/plugin.toml"), "api = 1\nname = \"alpha\"\nrun = \"sh\"\n").unwrap();

        let (manifests, problems) = crate::plugin::discover(&root);
        assert_eq!(manifests.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(), vec!["alpha", "zeta"]);
        assert!(problems.is_empty(), "{:?}", problems); // 매니페스트 없는 디렉토리는 doctor 전용
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn discover_reports_a_broken_manifest_and_keeps_the_others() {
        let root = std::env::temp_dir().join(format!("bibox-discover-broken-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("good")).unwrap();
        std::fs::create_dir_all(root.join("bad")).unwrap();
        std::fs::write(root.join("good/plugin.toml"), "api = 1\nname = \"good\"\nrun = \"sh\"\n").unwrap();
        std::fs::write(root.join("bad/plugin.toml"), "api = 1\nname = \"bad\"\n").unwrap();

        let (manifests, problems) = crate::plugin::discover(&root);
        assert_eq!(manifests.len(), 1);
        assert_eq!(manifests[0].name, "good");
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].plugin(), "bad");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn discover_on_a_missing_directory_is_empty_not_an_error() {
        let (manifests, problems) = crate::plugin::discover(std::path::Path::new("/nonexistent/bibox/plugins"));
        assert!(manifests.is_empty());
        assert!(problems.is_empty());
    }
    // ── 내장 스텁 ──

    fn noop() {}

    const TEST_BUILTINS: &[crate::plugin::builtin::Builtin] = &[crate::plugin::builtin::Builtin {
        name: "demo",
        manifest: "api = 1\nname = \"demo\"\ndescription = \"Demo plugin\"\n[[commands]]\nid = \"hello\"\ndesc = \"Say hello\"\nkey = \"<C-g>\"\n[[hooks]]\non = \"after_write\"\nrun = \"hello\"\n",
        run: noop,
        seeded: true,
    }];

    const STUB: &str = "api = 1\nname = \"demo\"\nbuiltin = \"demo\"\n";

    #[test]
    fn a_stub_expands_to_the_embedded_manifest_run_by_this_executable() {
        let mut problems = vec![];
        let m = parse_manifest_with(&dir("demo"), STUB, &mut problems, TEST_BUILTINS).expect("manifest");
        assert!(problems.is_empty(), "{:?}", problems);
        assert_eq!(m.builtin.as_deref(), Some("demo"));
        assert_eq!(m.description.as_deref(), Some("Demo plugin"));
        assert_eq!(m.version.as_deref(), Some(env!("CARGO_PKG_VERSION")));
        assert_eq!(m.commands.len(), 1);
        assert_eq!(m.commands[0].id, "hello");
        assert_eq!(m.hooks.len(), 1);
        let exe = std::env::current_exe().unwrap().to_string_lossy().to_string();
        assert_eq!(m.run, vec![exe, "plugin".to_string(), "run".to_string(), "demo".to_string()]);
        assert_eq!(m.dir, dir("demo"));
    }

    #[test]
    fn an_unknown_builtin_name_is_a_manifest_error() {
        let mut problems = vec![];
        assert!(parse_manifest_with(&dir("gone"), "api = 1\nname = \"gone\"\nbuiltin = \"gone\"\n", &mut problems, TEST_BUILTINS).is_none());
        assert!(matches!(&problems[0], PluginProblem::Manifest { detail, .. } if detail.contains("unknown built-in")));
    }

    #[test]
    fn run_and_builtin_are_mutually_exclusive_and_one_is_required() {
        let mut problems = vec![];
        assert!(parse_manifest_with(&dir("demo"), "api = 1\nname = \"demo\"\nrun = \"sh\"\nbuiltin = \"demo\"\n", &mut problems, TEST_BUILTINS).is_none());
        assert!(matches!(&problems[0], PluginProblem::Manifest { detail, .. } if detail.contains("mutually exclusive")));
        let mut problems = vec![];
        assert!(parse_manifest_with(&dir("demo"), "api = 1\nname = \"demo\"\n", &mut problems, TEST_BUILTINS).is_none());
        assert!(matches!(&problems[0], PluginProblem::Manifest { detail, .. } if detail.contains("run or builtin")));
    }

    #[test]
    fn a_stub_with_extra_fields_is_rejected() {
        let mut problems = vec![];
        let text = "api = 1\nname = \"demo\"\nbuiltin = \"demo\"\ndescription = \"x\"\n";
        assert!(parse_manifest_with(&dir("demo"), text, &mut problems, TEST_BUILTINS).is_none());
        assert!(matches!(&problems[0], PluginProblem::Manifest { detail, .. } if detail.contains("only api, name and builtin")));
    }

    #[test]
    fn a_stub_whose_builtin_differs_from_its_name_is_rejected() {
        let mut problems = vec![];
        let text = "api = 1\nname = \"other\"\nbuiltin = \"demo\"\n";
        assert!(parse_manifest_with(&dir("other"), text, &mut problems, TEST_BUILTINS).is_none());
        assert!(matches!(&problems[0], PluginProblem::Manifest { detail, .. } if detail.contains("must equal name")));
    }

    #[test]
    fn every_real_builtin_manifest_expands_cleanly() {
        for b in crate::plugin::builtin::BUILTINS {
            let stub = format!("api = 1\nname = \"{0}\"\nbuiltin = \"{0}\"\n", b.name);
            let mut problems = vec![];
            let m = parse_manifest_with(&dir(b.name), &stub, &mut problems, crate::plugin::builtin::BUILTINS);
            assert!(m.is_some() && problems.is_empty(), "{}: {:?}", b.name, problems);
        }
    }

    #[test]
    fn a_normal_manifest_has_no_builtin() {
        let mut problems = vec![];
        let m = parse_manifest(&dir("entry-tidy"), OK, &mut problems).unwrap();
        assert_eq!(m.builtin, None);
    }
    const WITH_SETTINGS: &str = r#"
api = 1
name = "x"
run = "sh run.sh"

[[settings]]
key = "push_on_write"
type = "bool"
default = false
desc = "git push after every hook commit"

[[settings]]
key = "max_tokens"
type = "int"
default = 4000

[[settings]]
key = "prefix"
type = "string"
default = ""

[[settings]]
key = "model"
type = "choice"
choices = ["claude-opus-5", "claude-sonnet-5"]
default = "claude-opus-5"
"#;

    #[test]
    fn settings_parse_into_typed_declarations_in_order() {
        let mut problems = vec![];
        let m = parse_manifest(&dir("x"), WITH_SETTINGS, &mut problems).expect("manifest");
        assert!(problems.is_empty(), "{:?}", problems);
        let keys: Vec<&str> = m.settings.iter().map(|s| s.key.as_str()).collect();
        assert_eq!(keys, vec!["push_on_write", "max_tokens", "prefix", "model"]);
        assert_eq!(m.settings[0].kind, SettingKind::Bool);
        assert_eq!(m.settings[0].default, toml::Value::Boolean(false));
        assert_eq!(m.settings[0].desc.as_deref(), Some("git push after every hook commit"));
        assert_eq!(m.settings[1].kind, SettingKind::Int);
        assert_eq!(m.settings[2].kind, SettingKind::Str);
        assert_eq!(m.settings[2].desc, None);
        assert_eq!(m.settings[3].kind, SettingKind::Choice(vec!["claude-opus-5".into(), "claude-sonnet-5".into()]));
    }

    fn settings_error(body: &str) -> String {
        let text = format!("api = 1\nname = \"x\"\nrun = \"sh\"\n\n[[settings]]\n{}", body);
        let mut problems = vec![];
        assert!(parse_manifest(&dir("x"), &text, &mut problems).is_none(), "should not load: {}", body);
        match &problems[0] {
            PluginProblem::Manifest { detail, .. } => detail.clone(),
            other => panic!("{:?}", other),
        }
    }

    #[test]
    fn an_unknown_setting_type_is_a_manifest_error() {
        assert!(settings_error("key = \"a\"\ntype = \"float\"\ndefault = 1.5\n").contains("unknown type \"float\""));
    }

    #[test]
    fn a_default_must_match_its_type() {
        assert!(settings_error("key = \"a\"\ntype = \"int\"\ndefault = \"800\"\n").contains("does not match type int"));
        assert!(settings_error("key = \"a\"\ntype = \"bool\"\ndefault = \"true\"\n").contains("does not match type bool"));
    }

    #[test]
    fn a_duplicate_setting_key_is_an_error() {
        let d = settings_error("key = \"a\"\ntype = \"bool\"\ndefault = true\n\n[[settings]]\nkey = \"a\"\ntype = \"bool\"\ndefault = false\n");
        assert!(d.contains("duplicate setting key \"a\""), "{}", d);
    }

    #[test]
    fn a_choice_needs_choices_and_its_default_among_them() {
        assert!(settings_error("key = \"m\"\ntype = \"choice\"\ndefault = \"x\"\n").contains("choices"));
        assert!(settings_error("key = \"m\"\ntype = \"choice\"\nchoices = []\ndefault = \"x\"\n").contains("choices"));
        assert!(settings_error("key = \"m\"\ntype = \"choice\"\nchoices = [\"a\", \"b\"]\ndefault = \"x\"\n").contains("does not match"));
    }

    #[test]
    fn choices_on_a_non_choice_type_is_an_error() {
        assert!(settings_error("key = \"m\"\ntype = \"bool\"\nchoices = [\"a\"]\ndefault = true\n").contains("choices is only for"));
    }

    #[test]
    fn a_setting_without_a_default_does_not_parse() {
        let d = settings_error("key = \"a\"\ntype = \"bool\"\n");
        assert!(d.contains("default"), "{}", d);
    }

    #[test]
    fn a_bad_setting_key_is_an_error() {
        assert!(settings_error("key = \"has space\"\ntype = \"bool\"\ndefault = true\n").contains("setting key"));
    }

    #[test]
    fn a_stub_with_settings_is_rejected() {
        let mut problems = vec![];
        let text = "api = 1\nname = \"demo\"\nbuiltin = \"demo\"\n[[settings]]\nkey = \"a\"\ntype = \"bool\"\ndefault = true\n";
        assert!(parse_manifest_with(&dir("demo"), text, &mut problems, TEST_BUILTINS).is_none());
        assert!(matches!(&problems[0], PluginProblem::Manifest { detail, .. } if detail.contains("only api, name and builtin")));
    }

    #[test]
    fn git_sync_declares_include_pdfs_and_push_on_write() {
        let stub = "api = 1\nname = \"git-sync\"\nbuiltin = \"git-sync\"\n";
        let mut problems = vec![];
        let m = parse_manifest(&dir("git-sync"), stub, &mut problems).expect("git-sync");
        let keys: Vec<&str> = m.settings.iter().map(|s| s.key.as_str()).collect();
        assert_eq!(keys, vec!["include_pdfs", "push_on_write"]);
        assert!(m.settings.iter().all(|s| s.kind == SettingKind::Bool && s.default == toml::Value::Boolean(false)));
    }

    #[test]
    fn setting_kind_accepts_only_its_own_values() {
        use toml::Value::*;
        assert!(SettingKind::Bool.accepts(&Boolean(true)) && !SettingKind::Bool.accepts(&String("true".into())));
        assert!(SettingKind::Int.accepts(&Integer(3)) && !SettingKind::Int.accepts(&Float(3.0)));
        assert!(SettingKind::Str.accepts(&String("s".into())) && !SettingKind::Str.accepts(&Integer(1)));
        let c = SettingKind::Choice(vec!["a".into()]);
        assert!(c.accepts(&String("a".into())) && !c.accepts(&String("b".into())));
    }

    const WITH_TAB: &str = r#"
api = 1
name = "x"
run = "sh run.sh"

[[tabs]]
title = "PDF"
run = "render"

[[commands]]
id = "render"
desc = "Render a page"
"#;

    #[test]
    fn a_tab_names_its_command() {
        let mut problems = vec![];
        let m = parse_manifest(&dir("x"), WITH_TAB, &mut problems).expect("manifest");
        assert!(problems.is_empty(), "{:?}", problems);
        assert_eq!(m.tabs, vec![Tab { title: "PDF".into(), run: "render".into() }]);
    }

    #[test]
    fn a_tab_whose_run_is_not_a_command_is_a_manifest_error() {
        let text = "api = 1\nname = \"x\"\nrun = \"sh\"\n[[tabs]]\ntitle = \"PDF\"\nrun = \"nope\"\n";
        let mut problems = vec![];
        assert!(parse_manifest(&dir("x"), text, &mut problems).is_none());
        assert!(matches!(&problems[0], PluginProblem::Manifest { detail, .. } if detail.contains("tab \"PDF\" refers to unknown command \"nope\"")));
    }

    #[test]
    fn a_tab_title_must_be_one_to_twelve_chars() {
        for title in ["", "ThirteenChars"] {
            let text = format!("api = 1\nname = \"x\"\nrun = \"sh\"\n[[tabs]]\ntitle = \"{}\"\nrun = \"r\"\n[[commands]]\nid = \"r\"\ndesc = \"R\"\n", title);
            let mut problems = vec![];
            assert!(parse_manifest(&dir("x"), &text, &mut problems).is_none(), "{:?}", title);
            assert!(matches!(&problems[0], PluginProblem::Manifest { detail, .. } if detail.contains("tab title")));
        }
    }

    #[test]
    fn a_stub_with_tabs_is_rejected() {
        let mut problems = vec![];
        let text = "api = 1\nname = \"demo\"\nbuiltin = \"demo\"\n[[tabs]]\ntitle = \"T\"\nrun = \"r\"\n";
        assert!(parse_manifest_with(&dir("demo"), text, &mut problems, TEST_BUILTINS).is_none());
        assert!(matches!(&problems[0], PluginProblem::Manifest { detail, .. } if detail.contains("only api, name and builtin")));
    }
}
