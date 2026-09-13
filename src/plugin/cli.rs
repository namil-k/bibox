use anyhow::{bail, Context as _, Result};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::i18n::Msgs;
use crate::plugin::host::apply_env;
use crate::plugin::manifest::{parse_manifest, PluginProblem};
use crate::plugin::{discover, plugins_dir, Manifest, PluginEnv, PluginHost};

/// 헬퍼의 단일 출처는 레포의 `plugins/lib/bibox_plugin.py`다. `plugin new`가 이 사본을 넣는다.
pub const HELPER_PY: &str = include_str!("../../plugins/lib/bibox_plugin.py");

/// `bibox <x>`가 절대 플러그인으로 폴스루되지 않는 이름들. clap의 `Commands`와 맞춰 둔다.
pub const BUILTIN_SUBCOMMANDS: &[&str] = &[
    "add", "list", "search", "show", "edit", "delete", "collect", "uncollect", "import", "export", "open", "sync",
    "init", "note", "modify", "review", "config", "agent-guide", "update", "doctor", "template", "plugin", "help",
];

#[derive(Debug, PartialEq)]
pub enum Source {
    Builtin(String),
    Local(PathBuf),
    GitHub { owner: String, repo: String, subdir: Option<PathBuf> },
    Url(String),
}

pub fn parse_source(s: &str) -> Source {
    if !s.contains('/') && !s.contains("://") && crate::plugin::builtin::find(s).is_some() {
        return Source::Builtin(s.to_string());
    }
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

pub struct ListRow {
    pub name: String,
    pub version: String,
    pub source: String,
    pub description: String,
    pub builtin: bool,
}

/// `built-in`(스텁), `local`(심링크), `git`(.git 있음), `dir`.
pub fn source_of(dir: &Path, m: &Manifest) -> &'static str {
    if m.builtin.is_some() {
        return "built-in";
    }
    let path = dir.join(&m.name);
    if std::fs::symlink_metadata(&path).map(|md| md.file_type().is_symlink()).unwrap_or(false) {
        return "local";
    }
    if path.join(".git").exists() {
        return "git";
    }
    "dir"
}

pub fn list_rows(dir: &Path) -> Vec<ListRow> {
    let (manifests, problems) = discover(dir);
    let mut rows: Vec<ListRow> = manifests
        .iter()
        .map(|m| ListRow {
            name: m.name.clone(),
            version: m.version.clone().unwrap_or_default(),
            source: source_of(dir, m).to_string(),
            description: m.description.clone().unwrap_or_default(),
            builtin: m.builtin.is_some(),
        })
        .collect();
    for p in &problems {
        if let PluginProblem::Manifest { plugin, detail } = p {
            rows.push(ListRow { name: plugin.clone(), version: String::new(), source: "error".to_string(), description: detail.clone(), builtin: false });
        }
    }
    rows.sort_by(|a, b| a.name.cmp(&b.name));
    rows
}

pub fn cmd_plugin_list(json: bool, _config: &Config) -> Result<()> {
    let dir = plugins_dir();
    crate::plugin::seed_builtins(&dir);
    let rows = list_rows(&dir);
    if json {
        let v: Vec<serde_json::Value> = rows
            .iter()
            .map(|r| serde_json::json!({ "name": r.name, "version": r.version, "source": r.source, "builtin": r.builtin, "description": r.description, "dir": dir.join(&r.name) }))
            .collect();
        println!("{}", serde_json::to_string_pretty(&v)?);
        return Ok(());
    }
    if rows.is_empty() {
        println!("(no plugins in {})", dir.display());
        return Ok(());
    }
    println!("{:<20} {:<10} {:<10} DESCRIPTION", "NAME", "VERSION", "SOURCE");
    for r in rows {
        println!("{:<20} {:<10} {:<10} {}", r.name, r.version, r.source, r.description);
    }
    Ok(())
}

// ── install / remove ────────────────────────────────────────────────────────

/// clone은 됐고 아직 `plugins/<name>/`으로 옮기지 않은 상태. CLI는 사이에 터미널 프롬프트,
/// TUI는 확인 팝업을 끼운다.
#[derive(Debug)]
pub struct Staged {
    pub name: String,
    pub run: String,
    /// 사용자가 친 그대로(`owner/repo`, URL)
    pub source: String,
    tmp: PathBuf,
    src: PathBuf,
    dest: PathBuf,
    copy: bool,
}

pub fn stage_from_git(dir: &Path, url: &str, subdir: Option<&Path>, shown_source: &str) -> Result<Staged> {
    std::fs::create_dir_all(dir)?;
    // 같은 파일시스템 안의 임시 디렉토리라야 rename이 된다.
    let tmp = dir.join(format!(".install-{}", uuid::Uuid::new_v4()));
    // 터미널 프롬프트를 막는다. TUI는 raw mode라 git의 "Username:" 질문이 보이지 않은 채
    // 키를 가로채고, 스레드는 영원히 기다린다. 자격 증명 helper(osxkeychain 등)는 그대로 쓰인다.
    let output = std::process::Command::new("git")
        .args(["clone", "--depth", "1", "--quiet", url])
        .arg(&tmp)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output()
        .context("git is required to install from a repository")?;
    if !output.status.success() {
        let _ = std::fs::remove_dir_all(&tmp);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let reason = stderr.lines().find(|l| l.starts_with("fatal:") || l.starts_with("error:")).or_else(|| stderr.lines().last()).unwrap_or("").trim();
        if reason.is_empty() {
            bail!("git clone failed for {}", url);
        }
        bail!("git clone failed for {}: {}", url, reason);
    }
    let staged = (|| -> Result<Staged> {
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
        Ok(Staged { name, run, source: shown_source.to_string(), tmp: tmp.clone(), src, dest, copy: subdir.is_some() })
    })();
    if staged.is_err() {
        let _ = std::fs::remove_dir_all(&tmp);
    }
    staged
}

/// 최종 디렉토리로 옮기고 검증한다. 검증에 실패하면 옮긴 것도 지운다.
pub fn commit_staged(staged: Staged, msgs: &Msgs) -> Result<(String, PathBuf)> {
    let result = (|| -> Result<()> {
        if staged.copy {
            copy_dir(&staged.src, &staged.dest)?;
        } else {
            std::fs::rename(&staged.src, &staged.dest)?;
        }
        let final_text = std::fs::read_to_string(staged.dest.join("plugin.toml"))?;
        let mut problems = vec![];
        if parse_manifest(&staged.dest, &final_text, &mut problems).is_none() {
            let _ = std::fs::remove_dir_all(&staged.dest);
            bail!("{}", problems.iter().map(|p| msgs.plugin_problem(p).trim().to_string()).collect::<Vec<_>>().join("\n"));
        }
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&staged.tmp);
    result.map(|_| (staged.name, staged.dest))
}

pub fn discard_staged(staged: Staged) {
    let _ = std::fs::remove_dir_all(&staged.tmp);
}

/// 로컬 디렉토리를 심링크로 깐다. 매니페스트가 깨졌으면 그 문구가 오류다.
pub fn install_local(dir: &Path, path: &Path, msgs: &Msgs) -> Result<(String, PathBuf)> {
    std::fs::create_dir_all(dir)?;
    let path = path.canonicalize()?;
    let text = std::fs::read_to_string(path.join("plugin.toml"))
        .with_context(|| format!("no plugin.toml in {}", path.display()))?;
    let mut problems = vec![];
    let Some(m) = parse_manifest(&path, &text, &mut problems) else {
        bail!("{}", problems.iter().map(|p| msgs.plugin_problem(p).trim().to_string()).collect::<Vec<_>>().join("\n"));
    };
    let dest = dir.join(&m.name);
    if dest.exists() {
        bail!("{} already exists", dest.display());
    }
    std::os::unix::fs::symlink(&path, &dest)?;
    Ok((m.name, dest))
}

pub fn cmd_plugin_install(source: &str, yes: bool, config: &Config) -> Result<()> {
    let dir = plugins_dir();
    match parse_source(source) {
        Source::Builtin(name) => {
            std::fs::create_dir_all(&dir)?;
            let dest = dir.join(&name);
            if dest.exists() {
                bail!("{} already exists", dest.display());
            }
            crate::plugin::write_stub(&dir, &name)?;
            println!("{}", config.msgs.plugin_installed(&name, &dest.display().to_string()));
            Ok(())
        }
        Source::Local(path) => {
            let (name, dest) = install_local(&dir, &path, &config.msgs)?;
            println!("{}", config.msgs.plugin_installed(&name, &dest.display().to_string()));
            Ok(())
        }
        Source::GitHub { owner, repo, subdir } => {
            install_from_git(&dir, &format!("https://github.com/{}/{}.git", owner, repo), subdir.as_deref(), source, yes, config)
        }
        Source::Url(url) => install_from_git(&dir, &url, None, source, yes, config),
    }
}

fn install_from_git(dir: &Path, url: &str, subdir: Option<&Path>, shown_source: &str, yes: bool, config: &Config) -> Result<()> {
    let staged = stage_from_git(dir, url, subdir, shown_source)?;
    println!("{}", config.msgs.plugin_install_header(&staged.name, shown_source));
    println!("{}", config.msgs.plugin_not_reviewed());
    println!("{}", config.msgs.plugin_runs(&staged.run));
    println!("{}", config.msgs.plugin_runs_as_you());
    if !yes {
        use std::io::IsTerminal;
        if !std::io::stdin().is_terminal() {
            discard_staged(staged);
            bail!("not a terminal; pass --yes to install without confirmation");
        }
        if !crate::interactive::prompt_yes_no(config.msgs.plugin_install_question()) {
            discard_staged(staged);
            bail!("cancelled");
        }
    }
    let (name, dest) = commit_staged(staged, &config.msgs)?;
    println!("{}", config.msgs.plugin_installed(&name, &dest.display().to_string()));
    Ok(())
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

/// 심링크면 링크만, 디렉토리면 통째로. TUI와 CLI가 같이 쓴다.
pub fn remove_plugin_files(dir: &Path, name: &str) -> Result<()> {
    let dest = dir.join(name);
    let meta = std::fs::symlink_metadata(&dest).with_context(|| format!("no plugin directory {}", dest.display()))?;
    if meta.file_type().is_symlink() {
        std::fs::remove_file(&dest)?;
    } else {
        std::fs::remove_dir_all(&dest)?;
    }
    Ok(())
}

pub fn cmd_plugin_remove(name: &str, yes: bool, config: &Config) -> Result<()> {
    let dir = plugins_dir();
    let dest = dir.join(name);
    if std::fs::symlink_metadata(&dest).is_err() {
        bail!("{}", config.msgs.plugin_not_found(name));
    }
    if !yes && !crate::interactive::prompt_yes_no(&config.msgs.plugin_remove_question(name)) {
        bail!("cancelled");
    }
    remove_plugin_files(&dir, name)?;
    println!("Removed {}", dest.display());
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

/// Levenshtein. 오타 제안(거리 2 이하)에만 쓴다.
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            cur.push((prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}

fn toml_type_name(v: &toml::Value) -> &'static str {
    match v {
        toml::Value::String(_) => "string",
        toml::Value::Integer(_) => "int",
        toml::Value::Float(_) => "float",
        toml::Value::Boolean(_) => "bool",
        toml::Value::Datetime(_) => "datetime",
        toml::Value::Array(_) => "array",
        toml::Value::Table(_) => "table",
    }
}

/// `[[settings]]`를 선언한 플러그인에 한해 `[plugins.<name>]`을 대조한다.
pub fn setting_problems(manifests: &[Manifest], config: &Config) -> Vec<PluginProblem> {
    use crate::plugin::manifest::SettingKind;
    let mut out = Vec::new();
    for m in manifests {
        if m.settings.is_empty() {
            continue;
        }
        let Some(table) = config.plugins.get(&m.name) else { continue };
        for (key, value) in table {
            match m.settings.iter().find(|s| &s.key == key) {
                Some(decl) => {
                    if !decl.kind.accepts(value) {
                        let (expected, found) = match &decl.kind {
                            SettingKind::Choice(cs) => (format!("one of {}", cs.join(", ")), value.to_string()),
                            k => (k.name().to_string(), toml_type_name(value).to_string()),
                        };
                        out.push(PluginProblem::SettingTypeMismatch { plugin: m.name.clone(), key: key.clone(), expected, found });
                    }
                }
                None => {
                    let suggestion = m
                        .settings
                        .iter()
                        .map(|s| (edit_distance(key, &s.key), s.key.clone()))
                        .filter(|(d, _)| *d <= 2)
                        .min_by_key(|(d, _)| *d)
                        .map(|(_, k)| k);
                    out.push(PluginProblem::UndeclaredSetting { plugin: m.name.clone(), key: key.clone(), suggestion });
                }
            }
        }
    }
    out
}

/// 로드 시점에는 안 보는 검사들. `plugin.toml` 없는 디렉토리, PATH, 설정 고아, 이름 충돌, 설정 대조.
pub fn doctor_checks(host: &PluginHost, config: &Config) -> Vec<PluginProblem> {
    let mut out = dir_problems(&plugins_dir());
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
    out.extend(tool_problems(host, which));
    out.extend(setting_problems(host.manifests(), config));
    for name in config.plugins.keys() {
        if !host.manifests().iter().any(|m| &m.name == name) {
            out.push(PluginProblem::ConfigWithoutPlugin { name: name.clone() });
        }
    }
    out
}

/// `plugins/` 아래에서 매니페스트 없는 디렉토리와 대상이 사라진 심링크. 로더는 둘 다 조용히 건너뛰므로
/// doctor만이 말해 준다(`plugin install ./path`로 깐 뒤 그 디렉토리를 지운 경우).
fn dir_problems(dir: &Path) -> Vec<PluginProblem> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else { return out };
    let mut paths: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    paths.sort();
    for p in paths {
        let name = p.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        if name.starts_with('.') {
            continue;
        }
        let is_link = std::fs::symlink_metadata(&p).map(|m| m.file_type().is_symlink()).unwrap_or(false);
        if is_link && !p.exists() {
            let target = std::fs::read_link(&p).map(|t| t.display().to_string()).unwrap_or_default();
            out.push(PluginProblem::DanglingLink { name, target });
        } else if p.is_dir() && !p.join("plugin.toml").exists() {
            out.push(PluginProblem::NoManifest { dir: name });
        }
    }
    out
}

/// pdf-view가 깔려 있으면 poppler 세 도구를 PATH에서 찾는다. `found`는 테스트가 바꿔 끼운다.
fn tool_problems(host: &PluginHost, found: impl Fn(&str) -> bool) -> Vec<PluginProblem> {
    use crate::plugin::builtin::pdf_view;
    if !host.manifests().iter().any(|m| m.name == pdf_view::BUILTIN.name) {
        return vec![];
    }
    pdf_view::TOOLS
        .iter()
        .filter(|t| !found(t))
        .map(|t| PluginProblem::ToolMissing { plugin: pdf_view::BUILTIN.name.to_string(), program: t.to_string(), hint: "brew install poppler".to_string() })
        .collect()
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
    #[test]
    fn a_bare_builtin_name_is_a_builtin_source() {
        assert!(matches!(parse_source("git-sync"), Source::Builtin(n) if n == "git-sync"));
        assert!(matches!(parse_source("someone/git-sync"), Source::GitHub { .. }));
        assert!(matches!(parse_source("not-a-builtin-xyz"), Source::Url(_)));
    }

    #[test]
    fn list_rows_tell_built_in_local_git_and_plain_apart() {
        let dir = std::env::temp_dir().join(format!("bibox-list-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // built-in stub
        crate::plugin::write_stub(&dir, "git-sync").unwrap();
        // plain dir
        std::fs::create_dir_all(dir.join("plain")).unwrap();
        std::fs::write(dir.join("plain/plugin.toml"), "api = 1\nname = \"plain\"\nrun = \"sh\"\nversion = \"0.2.0\"\n").unwrap();
        // git clone
        std::fs::create_dir_all(dir.join("cloned/.git")).unwrap();
        std::fs::write(dir.join("cloned/plugin.toml"), "api = 1\nname = \"cloned\"\nrun = \"sh\"\n").unwrap();
        // symlink
        let src = dir.join("src-of-local");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("plugin.toml"), "api = 1\nname = \"local\"\nrun = \"sh\"\n").unwrap();
        std::os::unix::fs::symlink(&src, dir.join("local")).unwrap();
        // broken
        std::fs::create_dir_all(dir.join("broken")).unwrap();
        std::fs::write(dir.join("broken/plugin.toml"), "api = 1\nname = \"broken\"\n").unwrap();

        let rows = list_rows(&dir);
        let get = |n: &str| rows.iter().find(|r| r.name == n).unwrap_or_else(|| panic!("row {}", n));
        assert_eq!(get("git-sync").source, "built-in");
        assert!(get("git-sync").builtin);
        assert_eq!(get("git-sync").version, env!("CARGO_PKG_VERSION"));
        assert_eq!(get("plain").source, "dir");
        assert_eq!(get("plain").version, "0.2.0");
        assert_eq!(get("cloned").source, "git");
        assert_eq!(get("local").source, "local");
        assert_eq!(get("broken").source, "error");
        assert!(get("broken").description.contains("run or builtin"));
        assert!(rows.windows(2).all(|w| w[0].name <= w[1].name), "sorted");
        let _ = std::fs::remove_dir_all(&dir);
    }
    #[test]
    fn remove_plugin_files_unlinks_a_symlink_and_deletes_a_directory() {
        let root = std::env::temp_dir().join(format!("bibox-rm-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let src = root.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("plugin.toml"), "x").unwrap();
        std::os::unix::fs::symlink(&src, root.join("linked")).unwrap();
        std::fs::create_dir_all(root.join("plain")).unwrap();
        remove_plugin_files(&root, "linked").unwrap();
        assert!(!root.join("linked").exists() && src.join("plugin.toml").exists(), "source untouched");
        remove_plugin_files(&root, "plain").unwrap();
        assert!(!root.join("plain").exists());
        assert!(remove_plugin_files(&root, "nope").is_err());
        let _ = std::fs::remove_dir_all(&root);
    }
    /// 커밋 하나 있는 로컬 저장소. `file://` URL로 clone된다.
    fn git_fixture(tag: &str, manifest: &str) -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("bibox-stage-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let repo = root.join("repo");
        let plugins = root.join("plugins");
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::create_dir_all(&plugins).unwrap();
        std::fs::write(repo.join("plugin.toml"), manifest).unwrap();
        let git = |args: &[&str]| {
            let ok = std::process::Command::new("git")
                .args(["-c", "user.email=t@t", "-c", "user.name=t", "-c", "commit.gpgsign=false"])
                .args(args)
                .current_dir(&repo)
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
            assert!(ok, "git {:?}", args);
        };
        git(&["init", "-q"]);
        git(&["add", "."]);
        git(&["commit", "-q", "-m", "init"]);
        (repo, plugins)
    }

    #[test]
    fn staging_clones_and_reads_the_name_then_discard_removes_the_temp_dir() {
        let (repo, plugins) = git_fixture("discard", "api = 1\nname = \"remote-demo\"\nrun = \"sh\"\n");
        let url = format!("file://{}", repo.display());
        let staged = stage_from_git(&plugins, &url, None, "someone/remote-demo").unwrap();
        assert_eq!(staged.name, "remote-demo");
        assert_eq!(staged.run, "sh");
        assert_eq!(staged.source, "someone/remote-demo");
        let leftovers = || std::fs::read_dir(&plugins).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().starts_with(".install-")).count();
        assert_eq!(leftovers(), 1, "one temp clone while staged");
        discard_staged(staged);
        assert_eq!(leftovers(), 0);
        assert!(!plugins.join("remote-demo").exists());
        let _ = std::fs::remove_dir_all(plugins.parent().unwrap());
    }

    #[test]
    fn commit_moves_the_clone_into_place_and_validates_it() {
        let (repo, plugins) = git_fixture("commit", "api = 1\nname = \"remote-demo\"\nrun = \"sh\"\n");
        let url = format!("file://{}", repo.display());
        let staged = stage_from_git(&plugins, &url, None, "x").unwrap();
        let (name, dest) = commit_staged(staged, &crate::i18n::Msgs::default()).unwrap();
        assert_eq!(name, "remote-demo");
        assert_eq!(dest, plugins.join("remote-demo"));
        assert!(dest.join("plugin.toml").exists());
        assert!(dest.join(".git").exists(), "a git install keeps .git so list shows `git`");
        let leftovers = std::fs::read_dir(&plugins).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().starts_with(".install-")).count();
        assert_eq!(leftovers, 0);
        // 같은 이름을 다시 깔면 clone 뒤 거절되고 임시 디렉토리는 남지 않는다
        let again = stage_from_git(&plugins, &url, None, "x");
        assert!(again.is_err());
        assert!(again.unwrap_err().to_string().contains("already exists"));
        let _ = std::fs::remove_dir_all(plugins.parent().unwrap());
    }

    #[test]
    fn commit_rejects_a_clone_whose_manifest_does_not_validate() {
        // 이름이 디렉토리 이름과 달라질 수 없으므로 run이 없는 매니페스트로 검증 실패를 만든다
        let (repo, plugins) = git_fixture("bad", "api = 1\nname = \"remote-bad\"\n");
        let url = format!("file://{}", repo.display());
        let staged = stage_from_git(&plugins, &url, None, "x").unwrap();
        let err = commit_staged(staged, &crate::i18n::Msgs::default()).unwrap_err().to_string();
        assert!(err.contains("run or builtin"), "{}", err);
        assert!(!plugins.join("remote-bad").exists(), "a rejected install leaves nothing behind");
        let _ = std::fs::remove_dir_all(plugins.parent().unwrap());
    }

    #[test]
    fn install_local_symlinks_and_returns_the_manifest_name() {
        let root = std::env::temp_dir().join(format!("bibox-local-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let src = root.join("src");
        let plugins = root.join("plugins");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&plugins).unwrap();
        std::fs::write(src.join("plugin.toml"), "api = 1\nname = \"src\"\nrun = \"sh\"\n").unwrap();
        let (name, dest) = install_local(&plugins, &src, &crate::i18n::Msgs::default()).unwrap();
        assert_eq!(name, "src");
        assert!(std::fs::symlink_metadata(&dest).unwrap().file_type().is_symlink());
        assert!(install_local(&plugins, &src, &crate::i18n::Msgs::default()).is_err(), "twice is an error");
        let _ = std::fs::remove_dir_all(&root);
    }
    fn declared() -> Vec<Manifest> {
        let text = "api = 1\nname = \"demo\"\nrun = \"sh\"\n\n[[settings]]\nkey = \"push_on_write\"\ntype = \"bool\"\ndefault = false\n\n[[settings]]\nkey = \"model\"\ntype = \"choice\"\nchoices = [\"a\", \"b\"]\ndefault = \"a\"\n";
        let mut problems = vec![];
        vec![parse_manifest(Path::new("/tmp/plugins/demo"), text, &mut problems).unwrap()]
    }

    fn config_with(plugins_toml: &str) -> Config {
        toml::from_str(&format!("bibox_dir = \"/tmp/b\"\nsearch_case_sensitive = false\ndefault_page_size = 20\n{}", plugins_toml)).unwrap()
    }

    #[test]
    fn a_wrongly_typed_value_is_a_type_mismatch() {
        let p = setting_problems(&declared(), &config_with("[plugins.demo]\npush_on_write = \"yes\"\n"));
        assert_eq!(p, vec![PluginProblem::SettingTypeMismatch { plugin: "demo".into(), key: "push_on_write".into(), expected: "bool".into(), found: "string".into() }]);
    }

    #[test]
    fn a_choice_outside_the_list_names_the_choices() {
        let p = setting_problems(&declared(), &config_with("[plugins.demo]\nmodel = \"zzz\"\n"));
        assert_eq!(p, vec![PluginProblem::SettingTypeMismatch { plugin: "demo".into(), key: "model".into(), expected: "one of a, b".into(), found: "\"zzz\"".into() }]);
    }

    #[test]
    fn an_undeclared_key_is_reported_with_a_close_suggestion() {
        let p = setting_problems(&declared(), &config_with("[plugins.demo]\npush_on_wirte = true\nfoo = 1\n"));
        assert_eq!(p, vec![
            PluginProblem::UndeclaredSetting { plugin: "demo".into(), key: "foo".into(), suggestion: None },
            PluginProblem::UndeclaredSetting { plugin: "demo".into(), key: "push_on_wirte".into(), suggestion: Some("push_on_write".into()) },
        ]);
    }

    #[test]
    fn a_plugin_without_declarations_is_not_checked() {
        let text = "api = 1\nname = \"free\"\nrun = \"sh\"\n";
        let mut problems = vec![];
        let m = parse_manifest(Path::new("/tmp/plugins/free"), text, &mut problems).unwrap();
        let p = setting_problems(&[m], &config_with("[plugins.free]\nanything = 1\n"));
        assert!(p.is_empty());
    }

    #[test]
    fn edit_distance_counts_edits() {
        assert_eq!(edit_distance("push_on_write", "push_on_wirte"), 2);
        assert_eq!(edit_distance("abc", "abc"), 0);
        assert_eq!(edit_distance("", "abc"), 3);
        assert_eq!(edit_distance("kitten", "sitting"), 3);
    }

    #[test]
    fn doctor_names_missing_poppler_tools_only_when_pdf_view_is_installed() {
        let m = crate::plugin::manifest::parse_manifest(&std::path::PathBuf::from("/x/pdf-view"), "api = 1\nname = \"pdf-view\"\nbuiltin = \"pdf-view\"\n", &mut vec![]).unwrap();
        let env = crate::plugin::PluginEnv { bin: "/bin/true".into(), config_dir: "/tmp".into(), db: "/tmp/db.json".into(), notes: "/tmp/n".into(), pdfs: "/tmp/p".into(), home: None };
        let host = PluginHost::new(vec![m], Default::default(), env);
        let missing = tool_problems(&host, |t| t == "pdftoppm");
        assert_eq!(missing.len(), 2, "{:?}", missing);
        assert!(missing.iter().all(|p| matches!(p, PluginProblem::ToolMissing { plugin, hint, .. } if plugin == "pdf-view" && hint.contains("brew install poppler"))));
        assert!(missing.iter().any(|p| matches!(p, PluginProblem::ToolMissing { program, .. } if program == "pdfinfo")));
        let env = crate::plugin::PluginEnv { bin: "/bin/true".into(), config_dir: "/tmp".into(), db: "/tmp/db.json".into(), notes: "/tmp/n".into(), pdfs: "/tmp/p".into(), home: None };
        let host = PluginHost::new(vec![], Default::default(), env);
        assert!(tool_problems(&host, |_| false).is_empty(), "no pdf-view, no complaint");
    }

    #[test]
    fn doctor_reports_a_symlink_whose_target_is_gone() {
        let dir = std::env::temp_dir().join(format!("bibox-dangling-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("fine")).unwrap();
        std::fs::write(dir.join("fine").join("plugin.toml"), "").unwrap();
        std::fs::create_dir_all(dir.join("empty")).unwrap();
        std::os::unix::fs::symlink(dir.join("nowhere"), dir.join("gone")).unwrap();
        let problems = dir_problems(&dir);
        assert!(matches!(&problems[..], [PluginProblem::NoManifest { dir }, PluginProblem::DanglingLink { name, target }]
            if dir == "empty" && name == "gone" && target.ends_with("nowhere")), "{:?}", problems);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
