use anyhow::{bail, Context as _, Result};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::plugin::host::{apply_env, PluginEnv};
use crate::plugin::manifest::{parse_manifest, PluginProblem};
use crate::plugin::{discover, plugins_dir, PluginHost};

/// 헬퍼의 단일 출처는 레포의 `plugins/lib/bibox_plugin.py`다. `plugin new`가 이 사본을 넣는다.
pub const HELPER_PY: &str = include_str!("../../plugins/lib/bibox_plugin.py");

/// `bibox <x>`가 절대 플러그인으로 폴스루되지 않는 이름들. clap의 `Commands`와 맞춰 둔다.
pub const BUILTIN_SUBCOMMANDS: &[&str] = &[
    "add", "list", "search", "show", "edit", "delete", "collect", "uncollect", "import", "export", "open", "sync",
    "init", "note", "modify", "review", "config", "agent-guide", "update", "doctor", "template", "plugin", "help",
];

#[derive(Debug, PartialEq)]
pub enum Source {
    Local(PathBuf),
    GitHub { owner: String, repo: String, subdir: Option<PathBuf> },
    Url(String),
}

pub fn parse_source(s: &str) -> Source {
    let p = Path::new(s);
    if p.is_dir() {
        return Source::Local(p.to_path_buf());
    }
    if s.contains("://") || s.starts_with("git@") {
        return Source::Url(s.to_string());
    }
    let parts: Vec<&str> = s.split('/').filter(|x| !x.is_empty()).collect();
    if parts.len() >= 2 {
        let subdir = if parts.len() > 2 { Some(parts[2..].iter().collect::<PathBuf>()) } else { None };
        return Source::GitHub { owner: parts[0].to_string(), repo: parts[1].to_string(), subdir };
    }
    Source::Url(s.to_string())
}

/// PATH에서 실행 파일을 찾는다. `/`가 있으면 그 경로를 직접 본다.
pub fn which(program: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let executable = |p: &Path| p.is_file() && p.metadata().map(|m| m.permissions().mode() & 0o111 != 0).unwrap_or(false);
    if program.contains('/') {
        return executable(Path::new(program));
    }
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|d| executable(&d.join(program))))
        .unwrap_or(false)
}

// ── list ────────────────────────────────────────────────────────────────────

pub fn cmd_plugin_list(json: bool, config: &Config) -> Result<()> {
    let dir = plugins_dir();
    let (manifests, problems) = discover(&dir);
    let disabled = crate::config::disabled_plugins(config);
    let mut rows: Vec<(String, String, String, String)> = Vec::new(); // name, version, status, description
    for m in &manifests {
        let status = if disabled.contains(&m.name) { "disabled".to_string() } else { "ok".to_string() };
        rows.push((m.name.clone(), m.version.clone().unwrap_or_default(), status, m.description.clone().unwrap_or_default()));
    }
    for p in &problems {
        if let PluginProblem::Manifest { plugin, detail } = p {
            rows.push((plugin.clone(), String::new(), format!("error: {}", detail), String::new()));
        }
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0));

    if json {
        let v: Vec<serde_json::Value> = rows
            .iter()
            .map(|(n, v, s, d)| serde_json::json!({ "name": n, "version": v, "status": s, "description": d, "dir": dir.join(n) }))
            .collect();
        println!("{}", serde_json::to_string_pretty(&v)?);
        return Ok(());
    }
    if rows.is_empty() {
        println!("(no plugins in {})", dir.display());
        return Ok(());
    }
    println!("{:<20} {:<10} {:<28} {}", "NAME", "VERSION", "STATUS", "DESCRIPTION");
    for (n, v, s, d) in rows {
        println!("{:<20} {:<10} {:<28} {}", n, v, s, d);
    }
    Ok(())
}

// ── install / remove / enable / disable ─────────────────────────────────────

pub fn cmd_plugin_install(source: &str, yes: bool, config: &Config) -> Result<()> {
    let dir = plugins_dir();
    std::fs::create_dir_all(&dir)?;
    match parse_source(source) {
        Source::Local(path) => {
            let path = path.canonicalize()?;
            let text = std::fs::read_to_string(path.join("plugin.toml"))
                .with_context(|| format!("no plugin.toml in {}", path.display()))?;
            let mut problems = vec![];
            let Some(m) = parse_manifest(&path, &text, &mut problems) else {
                bail!("{}", problems.iter().map(|p| config.msgs.plugin_problem(p)).collect::<Vec<_>>().join("\n"));
            };
            let dest = dir.join(&m.name);
            if dest.exists() {
                bail!("{} already exists", dest.display());
            }
            std::os::unix::fs::symlink(&path, &dest)?;
            println!("{}", config.msgs.plugin_installed(&m.name, &dest.display().to_string()));
            Ok(())
        }
        Source::GitHub { owner, repo, subdir } => {
            install_from_git(&format!("https://github.com/{}/{}.git", owner, repo), subdir.as_deref(), source, yes, config)
        }
        Source::Url(url) => install_from_git(&url, None, source, yes, config),
    }
}

fn install_from_git(url: &str, subdir: Option<&Path>, shown_source: &str, yes: bool, config: &Config) -> Result<()> {
    let dir = plugins_dir();
    // 같은 파일시스템 안의 임시 디렉토리라야 rename이 된다.
    let tmp = dir.join(format!(".install-{}", uuid::Uuid::new_v4()));
    let status = std::process::Command::new("git")
        .args(["clone", "--depth", "1", "--quiet", url])
        .arg(&tmp)
        .status()
        .context("git is required to install from a repository")?;
    if !status.success() {
        let _ = std::fs::remove_dir_all(&tmp);
        bail!("git clone failed for {}", url);
    }
    let result = (|| -> Result<()> {
        let src = match subdir {
            Some(s) => tmp.join(s),
            None => tmp.clone(),
        };
        let text = std::fs::read_to_string(src.join("plugin.toml"))
            .with_context(|| format!("no plugin.toml at {}", src.display()))?;
        // 이름은 매니페스트에서 읽어야 하는데 디렉토리 이름은 아직 임시다. 전체 검증은
        // 최종 디렉토리로 옮긴 뒤 한다.
        let file: toml::Value = toml::from_str(&text)?;
        let name = file.get("name").and_then(|v| v.as_str()).context("plugin.toml has no name")?.to_string();
        let run = file.get("run").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let dest = dir.join(&name);
        if dest.exists() {
            bail!("{} already exists", dest.display());
        }

        println!("{}", config.msgs.plugin_install_header(&name, shown_source));
        println!("{}", config.msgs.plugin_not_reviewed());
        println!("{}", config.msgs.plugin_runs(&run));
        println!("{}", config.msgs.plugin_runs_as_you());
        if !yes {
            use std::io::IsTerminal;
            if !std::io::stdin().is_terminal() {
                bail!("not a terminal; pass --yes to install without confirmation");
            }
            if !crate::interactive::prompt_yes_no(config.msgs.plugin_install_question()) {
                bail!("cancelled");
            }
        }

        if subdir.is_some() {
            copy_dir(&src, &dest)?;
        } else {
            std::fs::rename(&src, &dest)?;
        }
        let final_text = std::fs::read_to_string(dest.join("plugin.toml"))?;
        let mut problems = vec![];
        if parse_manifest(&dest, &final_text, &mut problems).is_none() {
            let _ = std::fs::remove_dir_all(&dest);
            bail!("{}", problems.iter().map(|p| config.msgs.plugin_problem(p)).collect::<Vec<_>>().join("\n"));
        }
        println!("{}", config.msgs.plugin_installed(&name, &dest.display().to_string()));
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&tmp);
    result
}

fn copy_dir(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let target = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

pub fn cmd_plugin_remove(name: &str, yes: bool, config: &Config) -> Result<()> {
    let dest = plugins_dir().join(name);
    let meta = std::fs::symlink_metadata(&dest).with_context(|| config.msgs.plugin_not_found(name))?;
    if !yes && !crate::interactive::prompt_yes_no(&config.msgs.plugin_remove_question(name)) {
        bail!("cancelled");
    }
    if meta.file_type().is_symlink() {
        std::fs::remove_file(&dest)?;
    } else {
        std::fs::remove_dir_all(&dest)?;
    }
    println!("Removed {}", dest.display());
    Ok(())
}

/// `config.toml`을 다시 쓴다. 주석이 사라지는 것은 Settings 화면과 같은 기존 동작이다.
pub fn cmd_plugin_set_enabled(name: &str, enabled: bool, config: &Config) -> Result<()> {
    if !plugins_dir().join(name).exists() {
        bail!("{}", config.msgs.plugin_not_found(name));
    }
    let mut cfg = crate::config::load_config()?;
    let table = cfg.plugins.entry(name.to_string()).or_default();
    if enabled {
        table.remove("enabled");
        if table.is_empty() {
            cfg.plugins.remove(name);
        }
    } else {
        table.insert("enabled".to_string(), toml::Value::Boolean(false));
    }
    crate::config::save_config(&cfg)?;
    println!("{} {}", if enabled { "Enabled" } else { "Disabled" }, name);
    Ok(())
}

// ── new ─────────────────────────────────────────────────────────────────────

pub fn cmd_plugin_new(name: &str, config: &Config) -> Result<()> {
    let dest = plugins_dir().join(name);
    if dest.exists() {
        bail!("{} already exists", dest.display());
    }
    scaffold(&dest, name)?;
    println!("{}", config.msgs.plugin_installed(name, &dest.display().to_string()));
    println!("Edit {}/main.py, then run its hello command from the TUI (bind a key in keymap.toml or set key = in plugin.toml).", dest.display());
    Ok(())
}

pub fn scaffold(dest: &Path, name: &str) -> Result<()> {
    std::fs::create_dir_all(dest)?;
    std::fs::write(
        dest.join("plugin.toml"),
        format!(
            "api = 1\nname = \"{name}\"\nversion = \"0.1.0\"\ndescription = \"Describe what {name} does\"\nrun = \"python3 main.py\"\n\n[[commands]]\nid = \"hello\"\ndesc = \"Say hello from {name}\"\n# key = \"<C-h>\"          # default key; users can rebind in keymap.toml\n# menu = true            # show in the right-click menu\n\n# [[hooks]]\n# on = \"after_write\"     # before_add | after_write | after_note_save\n# run = \"hello\"\n",
            name = name
        ),
    )?;
    std::fs::write(
        dest.join("main.py"),
        "from bibox_plugin import serve, pick, prompt, confirm, progress, bibox, copy_to_clipboard  # noqa: F401\n\n\ndef hello(ctx):\n    entry = ctx.get(\"entry\")\n    title = entry[\"title\"] if entry else \"(no entry)\"\n    return {\"message\": f\"Hello from the plugin. Current entry: {title}\"}\n\n\nserve({\"hello\": hello})\n",
    )?;
    std::fs::write(dest.join("bibox_plugin.py"), HELPER_PY)?;
    std::fs::write(dest.join(".gitignore"), "stderr.log\n")?;
    Ok(())
}

// ── [cli] 폴스루 ─────────────────────────────────────────────────────────────

/// `bibox <name> args...`. stdio를 물려주고 종료 코드를 돌려준다. 프로토콜은 쓰지 않는다.
pub fn run_external(args: Vec<OsString>, config: &Config) -> Result<i32> {
    let name = args.first().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let (manifests, _) = discover(&plugins_dir());
    let Some(m) = manifests.iter().find(|m| m.name == name) else {
        bail!("{}", config.msgs.plugin_not_found(&name));
    };
    let Some(argv) = &m.cli else {
        bail!("{}", config.msgs.plugin_no_cli(&name));
    };
    let mut cmd = std::process::Command::new(&argv[0]);
    cmd.args(&argv[1..]).args(&args[1..]).current_dir(&m.dir);
    apply_env(&mut cmd, &PluginEnv::from_config(config), &m.dir);
    let status = cmd.status().with_context(|| format!("failed to run {}", argv[0]))?;
    Ok(status.code().unwrap_or(1))
}

// ── doctor ──────────────────────────────────────────────────────────────────

/// 로드 시점에는 안 보는 검사들. `plugin.toml` 없는 디렉토리, PATH, 설정 고아, 이름 충돌.
pub fn doctor_checks(host: &PluginHost, config: &Config) -> Vec<PluginProblem> {
    let mut out = Vec::new();
    let dir = plugins_dir();
    if let Ok(rd) = std::fs::read_dir(&dir) {
        let mut dirs: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
        dirs.sort();
        for d in dirs {
            let name = d.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            if name.starts_with('.') {
                continue;
            }
            if !d.join("plugin.toml").exists() {
                out.push(PluginProblem::NoManifest { dir: name });
            }
        }
    }
    for m in host.manifests() {
        if !which(&m.run[0]) {
            out.push(PluginProblem::ExecutableMissing { plugin: m.name.clone(), program: m.run[0].clone() });
        }
        if let Some(cli) = &m.cli {
            if !which(&cli[0]) {
                out.push(PluginProblem::ExecutableMissing { plugin: m.name.clone(), program: cli[0].clone() });
            }
        }
        if BUILTIN_SUBCOMMANDS.contains(&m.name.as_str()) {
            out.push(PluginProblem::NameCollidesWithSubcommand { plugin: m.name.clone() });
        }
    }
    for name in config.plugins.keys() {
        if !host.manifests().iter().any(|m| &m.name == name) {
            out.push(PluginProblem::ConfigWithoutPlugin { name: name.clone() });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_local_directory_is_a_local_source() {
        let d = std::env::temp_dir();
        assert!(matches!(parse_source(d.to_str().unwrap()), Source::Local(_)));
    }

    #[test]
    fn owner_repo_with_optional_subdir_is_github() {
        match parse_source("namil-k/bibox-plugins") {
            Source::GitHub { owner, repo, subdir } => {
                assert_eq!(owner, "namil-k");
                assert_eq!(repo, "bibox-plugins");
                assert_eq!(subdir, None);
            }
            other => panic!("{:?}", other),
        }
        match parse_source("namil-k/bibox/plugins/summarize") {
            Source::GitHub { repo, subdir, .. } => {
                assert_eq!(repo, "bibox");
                assert_eq!(subdir, Some(PathBuf::from("plugins/summarize")));
            }
            other => panic!("{:?}", other),
        }
    }

    #[test]
    fn urls_are_passed_to_git_as_is() {
        assert!(matches!(parse_source("https://github.com/x/y.git"), Source::Url(_)));
        assert!(matches!(parse_source("git@github.com:x/y.git"), Source::Url(_)));
    }

    #[test]
    fn builtin_subcommands_cover_every_clap_variant() {
        for name in ["add", "list", "search", "show", "edit", "delete", "collect", "uncollect", "import", "export",
                     "open", "sync", "init", "note", "modify", "review", "config", "agent-guide", "update", "doctor",
                     "template", "plugin", "help"] {
            assert!(BUILTIN_SUBCOMMANDS.contains(&name), "{}", name);
        }
    }

    #[test]
    fn which_finds_sh_and_not_nonsense() {
        assert!(which("sh"));
        assert!(!which("definitely-not-a-program-xyz"));
        assert!(which("/bin/sh"));
    }

    #[test]
    fn new_scaffold_writes_a_loadable_manifest_and_a_gitignore() {
        let root = std::env::temp_dir().join(format!("bibox-new-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        scaffold(&root.join("my-plugin"), "my-plugin").unwrap();
        let text = std::fs::read_to_string(root.join("my-plugin/plugin.toml")).unwrap();
        let mut problems = vec![];
        let m = crate::plugin::manifest::parse_manifest(&root.join("my-plugin"), &text, &mut problems).unwrap();
        assert!(problems.is_empty(), "{:?}", problems);
        assert_eq!(m.commands[0].id, "hello");
        assert!(root.join("my-plugin/main.py").exists());
        assert!(root.join("my-plugin/bibox_plugin.py").exists());
        assert_eq!(std::fs::read_to_string(root.join("my-plugin/.gitignore")).unwrap().trim(), "stderr.log");
        let _ = std::fs::remove_dir_all(&root);
    }
}
