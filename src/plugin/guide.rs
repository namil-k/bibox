//! `bibox agent-guide`의 "Installed plugins" 절. 설치된 플러그인마다 매니페스트에서 명령·훅·탭·설정을 뽑고
//! 작성자가 쓴 가이드(`guide = "AGENT.md"`, 내장은 코드)를 그 뒤에 그대로 싣는다. 에이전트는 이걸로
//! `bibox <name> ...`을 어떻게 쓰는지 안다.

use super::manifest::{Manifest, SettingKind};

/// 렌더에 필요한 만큼만 뽑은 것. 매니페스트 타입에서 떼어 두어 테스트가 손으로 만들 수 있다.
#[derive(Debug, Clone, PartialEq)]
pub struct Installed {
    pub name: String,
    pub version: String,
    pub source: String,
    pub description: String,
    pub cli: bool,
    /// (id, 기본 키, 설명)
    pub commands: Vec<(String, Option<String>, String)>,
    pub hooks: Vec<String>,
    pub tabs: Vec<String>,
    /// (key, 타입, 기본값, 설명)
    pub settings: Vec<(String, String, String, String)>,
    pub guide: Option<String>,
}

pub fn from_manifest(m: &Manifest, source: &str) -> Installed {
    Installed {
        name: m.name.clone(),
        version: m.version.clone().unwrap_or_default(),
        source: source.to_string(),
        description: m.description.clone().unwrap_or_default(),
        cli: m.cli.is_some(),
        commands: m
            .commands
            .iter()
            .map(|c| {
                let key = c.key.as_ref().map(|ks| ks.iter().map(|k| crate::keymap::render_key(*k)).collect::<Vec<_>>().join(""));
                (c.id.clone(), key, c.desc.clone())
            })
            .collect(),
        hooks: m.hooks.iter().map(|h| format!("{} -> {}", h.on.name(), h.run)).collect(),
        tabs: m.tabs.iter().map(|t| t.title.clone()).collect(),
        settings: m
            .settings
            .iter()
            .map(|s| {
                let kind = match &s.kind {
                    SettingKind::Bool => "bool".to_string(),
                    SettingKind::Int => "int".to_string(),
                    SettingKind::Str => "string".to_string(),
                    SettingKind::Choice(cs) => format!("choice of {}", cs.join(", ")),
                };
                let default = match &s.default {
                    toml::Value::String(v) => v.clone(),
                    other => other.to_string(),
                };
                (s.key.clone(), kind, default, s.desc.clone().unwrap_or_default())
            })
            .collect(),
        guide: m.guide.clone(),
    }
}

/// Markdown 절. 텍스트 가이드의 맨 끝에 붙는다.
pub fn section(list: &[Installed]) -> String {
    let mut s = String::from("## Installed plugins\n\n");
    if list.is_empty() {
        s.push_str("None. `bibox plugin list` shows what is installed; `bibox plugin install pdf-view` or `git-sync` turns a built-in on.\n");
        return s;
    }
    for p in list {
        s.push_str(&format!("### {}{}{}\n", p.name, if p.version.is_empty() { String::new() } else { format!(" {}", p.version) }, if p.source.is_empty() { String::new() } else { format!(" ({})", p.source) }));
        if !p.description.is_empty() {
            s.push_str(&p.description);
            s.push('\n');
        }
        if p.cli {
            s.push_str(&format!("CLI: `bibox {} <args>` runs the plugin's own command line.\n", p.name));
        }
        if !p.commands.is_empty() {
            let cmds: Vec<String> = p
                .commands
                .iter()
                .map(|(id, key, desc)| match key {
                    Some(k) => format!("{} ({}): {}", id, k, desc),
                    None => format!("{}: {}", id, desc),
                })
                .collect();
            s.push_str(&format!("Commands: {}\n", cmds.join("; ")));
        }
        if !p.hooks.is_empty() {
            s.push_str(&format!("Hooks: {}\n", p.hooks.join(", ")));
        }
        if !p.tabs.is_empty() {
            s.push_str(&format!("Preview tabs: {}\n", p.tabs.join(", ")));
        }
        if !p.settings.is_empty() {
            let rows: Vec<String> = p
                .settings
                .iter()
                .map(|(k, kind, d, desc)| if desc.is_empty() { format!("{} ({}, default {})", k, kind, d) } else { format!("{} ({}, default {}): {}", k, kind, d, desc) })
                .collect();
            s.push_str(&format!("Settings under [plugins.{}] in config.toml: {}\n", p.name, rows.join("; ")));
        }
        s.push('\n');
        match &p.guide {
            Some(g) => {
                s.push_str(g.trim_end());
                s.push('\n');
            }
            None => s.push_str("(This plugin ships no usage guide. Read its plugin.toml and README in the plugin directory.)\n"),
        }
        s.push('\n');
    }
    s
}

/// `agent-guide --json`의 `installed`.
pub fn json(list: &[Installed]) -> serde_json::Value {
    serde_json::Value::Array(
        list.iter()
            .map(|p| {
                serde_json::json!({
                    "name": p.name,
                    "version": p.version,
                    "source": p.source,
                    "description": p.description,
                    "cli": if p.cli { serde_json::Value::String(format!("bibox {} <args>", p.name)) } else { serde_json::Value::Null },
                    "commands": p.commands.iter().map(|(id, key, desc)| serde_json::json!({ "id": id, "key": key, "desc": desc })).collect::<Vec<_>>(),
                    "hooks": p.hooks,
                    "tabs": p.tabs,
                    "settings": p.settings.iter().map(|(k, kind, d, desc)| serde_json::json!({ "key": k, "type": kind, "default": d, "desc": desc })).collect::<Vec<_>>(),
                    "guide": p.guide,
                })
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summarize() -> Installed {
        Installed {
            name: "summarize".into(),
            version: "0.1.0".into(),
            source: "dir".into(),
            description: "Summarize the attached PDF into the note with Claude".into(),
            cli: true,
            commands: vec![("summarize".into(), Some("S".into()), "Summarize the PDF into the note".into())],
            hooks: vec!["after_write -> summarize".into()],
            tabs: vec![],
            settings: vec![("model".into(), "choice of claude-opus-5, claude-sonnet-5".into(), "claude-opus-5".into(), "Claude model".into())],
            guide: Some("Run `bibox summarize <citekey>` to write the Summary section.\n".into()),
        }
    }

    /// 에이전트가 읽을 순서: 이름·출처, 한 줄 설명, CLI 규칙, 명령(키), 훅, 설정, 그리고 작성자의 가이드 전문.
    #[test]
    fn the_section_lists_each_plugin_with_its_cli_rule_commands_settings_and_guide() {
        let s = section(&[summarize()]);
        assert!(s.starts_with("## Installed plugins\n\n### summarize 0.1.0 (dir)\nSummarize the attached PDF into the note with Claude\n"), "{}", s);
        assert!(s.contains("CLI: `bibox summarize <args>`"));
        assert!(s.contains("Commands: summarize (S): Summarize the PDF into the note"));
        assert!(s.contains("Hooks: after_write -> summarize"));
        assert!(s.contains("Settings under [plugins.summarize] in config.toml: model (choice of claude-opus-5, claude-sonnet-5, default claude-opus-5): Claude model"));
        assert!(s.ends_with("Run `bibox summarize <citekey>` to write the Summary section.\n\n"), "{:?}", &s[s.len() - 80..]);
    }

    #[test]
    fn a_plugin_without_a_guide_or_cli_says_so_and_an_empty_list_points_at_plugin_list() {
        let mut p = summarize();
        p.cli = false;
        p.guide = None;
        p.settings.clear();
        let s = section(&[p]);
        assert!(!s.contains("CLI:"));
        assert!(!s.contains("Settings under"));
        assert!(s.contains("ships no usage guide"));
        assert!(section(&[]).contains("bibox plugin list"));
    }

    #[test]
    fn json_mirrors_the_section() {
        let v = json(&[summarize()]);
        let p = &v[0];
        assert_eq!(p["name"], "summarize");
        assert_eq!(p["cli"], "bibox summarize <args>");
        assert_eq!(p["commands"][0]["key"], "S");
        assert_eq!(p["settings"][0]["default"], "claude-opus-5");
        assert!(p["guide"].as_str().unwrap().contains("bibox summarize <citekey>"));
        let mut q = summarize();
        q.cli = false;
        assert!(json(&[q])[0]["cli"].is_null());
    }

    /// 매니페스트에서 뽑을 때 키는 keymap 표기(`<C-h>`), 훅은 `on -> run`, choice는 선택지를 나열한다.
    #[test]
    fn from_manifest_renders_keys_hooks_and_choices() {
        let text = "api = 1\nname = \"m\"\nrun = \"sh x\"\n[cli]\nrun = \"sh cli\"\n[[commands]]\nid = \"go\"\ndesc = \"Go\"\nkey = \"<C-h>\"\n[[hooks]]\non = \"after_write\"\nrun = \"go\"\n[[tabs]]\ntitle = \"T\"\nrun = \"go\"\n[[settings]]\nkey = \"model\"\ntype = \"choice\"\nchoices = [\"a\", \"b\"]\ndefault = \"a\"\ndesc = \"Which\"\n[[settings]]\nkey = \"n\"\ntype = \"int\"\ndefault = 4\n";
        let mut problems = vec![];
        let m = super::super::manifest::parse_manifest(std::path::Path::new("/tmp/plugins/m"), text, &mut problems).unwrap();
        let i = from_manifest(&m, "dir");
        assert!(i.cli);
        assert_eq!(i.commands, vec![("go".to_string(), Some("<C-h>".to_string()), "Go".to_string())]);
        assert_eq!(i.hooks, vec!["after_write -> go"]);
        assert_eq!(i.tabs, vec!["T"]);
        assert_eq!(i.settings[0], ("model".to_string(), "choice of a, b".to_string(), "a".to_string(), "Which".to_string()));
        assert_eq!(i.settings[1], ("n".to_string(), "int".to_string(), "4".to_string(), String::new()));
    }
}
