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
struct ManifestFile {
    api: u32,
    name: String,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    description: Option<String>,
    run: String,
    #[serde(default)]
    commands: Vec<CommandFile>,
    #[serde(default)]
    hooks: Vec<HookFile>,
    #[serde(default)]
    cli: Option<CliFile>,
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
    pub dir: PathBuf,
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
    /// doctor 전용. `[plugins.x]`가 있는데 x 플러그인이 없다.
    ConfigWithoutPlugin { name: String },
    /// doctor 전용. `run`/`[cli].run`의 첫 토큰이 PATH에 없다.
    ExecutableMissing { plugin: String, program: String },
    /// doctor 전용. 이름이 내장 서브커맨드와 같아 `bibox <name>`이 절대 폴스루되지 않는다.
    NameCollidesWithSubcommand { plugin: String },
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
            | PluginProblem::NameCollidesWithSubcommand { plugin } => plugin,
            PluginProblem::NoManifest { dir } => dir,
            PluginProblem::ConfigWithoutPlugin { name } => name,
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

/// 디렉토리 이름을 플러그인 이름의 근거로 쓴다. 매니페스트가 깨져 `name`을 못 읽어도
/// 문제를 어느 플러그인 것으로 돌릴지는 알아야 한다.
fn dir_name(dir: &Path) -> String {
    dir.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()
}

/// 파싱 이후 문제는 전부 `problems`에 모은다. 오류 등급 문제가 하나라도 있으면 `None`.
/// TOML 문법 오류는 serde가 첫 오류에서 멈추므로 하나만 나온다(키맵과 같은 한계).
pub fn parse_manifest(dir: &Path, text: &str, problems: &mut Vec<PluginProblem>) -> Option<Manifest> {
    let plugin = dir_name(dir);
    let err = |detail: String| PluginProblem::Manifest { plugin: plugin.clone(), detail };

    let file: ManifestFile = match toml::from_str(text) {
        Ok(f) => f,
        Err(e) => {
            problems.push(err(e.to_string()));
            return None;
        }
    };

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
    let run: Vec<String> = file.run.split_whitespace().map(str::to_string).collect();
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
}
