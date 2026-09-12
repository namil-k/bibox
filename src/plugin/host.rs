use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use crate::config::Config;
use crate::keymap::{KeyPress, LayerId};
use crate::models::Entry;
use crate::plugin::manifest::{HookKind, Manifest};
use crate::plugin::protocol::{head, parse_plugin_line, Context, Final, Paths, PluginMsg, Request, UiAnswer, UiRequest};

// ── 명령 테이블 ──────────────────────────────────────────────────────────────

/// 로드 시 만들어지는 명령 테이블의 인덱스. `Action::Plugin(PluginCmdId)`가 이걸 들고
/// 있으므로 `Action`이 `Copy`를 유지한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PluginCmdId(pub u16);

#[derive(Debug, Clone, PartialEq)]
pub struct PluginCommand {
    pub plugin: String,
    pub id: String,
    pub desc: String,
    pub key: Option<Vec<KeyPress>>,
    pub layers: Vec<LayerId>,
    pub menu: bool,
}

impl PluginCommand {
    /// 키맵과 도움말에서 쓰는 이름. `entry-tidy.tidy`.
    pub fn full_name(&self) -> String {
        format!("{}.{}", self.plugin, self.id)
    }
}

#[derive(Debug, Clone, Default)]
pub struct PluginCommands {
    list: Vec<PluginCommand>,
}

impl PluginCommands {
    pub fn find(&self, full: &str) -> Option<PluginCmdId> {
        self.list
            .iter()
            .position(|c| c.full_name() == full)
            .map(|i| PluginCmdId(i as u16))
    }

    pub fn get(&self, id: PluginCmdId) -> Option<&PluginCommand> {
        self.list.get(id.0 as usize)
    }

    pub fn iter(&self) -> impl Iterator<Item = (PluginCmdId, &PluginCommand)> {
        self.list.iter().enumerate().map(|(i, c)| (PluginCmdId(i as u16), c))
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.list.len()
    }
}

// ── 실행 환경 ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct PluginEnv {
    pub bin: PathBuf,
    pub config_dir: PathBuf,
    pub db: PathBuf,
    pub notes: PathBuf,
    pub pdfs: PathBuf,
    pub home: Option<PathBuf>,
}

impl PluginEnv {
    pub fn from_config(config: &Config) -> PluginEnv {
        PluginEnv {
            bin: std::env::current_exe().unwrap_or_else(|_| PathBuf::from("bibox")),
            config_dir: crate::config::config_path().parent().map(|p| p.to_path_buf()).unwrap_or_default(),
            db: crate::config::resolve_db_path(config),
            notes: config.notes_dir.clone(),
            pdfs: config.bibox_dir.clone(),
            home: config.home.as_ref().map(|h| crate::config::expand_tilde(h)),
        }
    }

    fn paths(&self) -> Paths {
        Paths {
            config_dir: self.config_dir.clone(),
            db: self.db.clone(),
            notes: self.notes.clone(),
            pdfs: self.pdfs.clone(),
            home: self.home.clone(),
        }
    }
}

// ── UI 싱크 ──────────────────────────────────────────────────────────────────

/// 플러그인의 UI 요청을 어디로 보낼지. TUI는 채널로 메인 루프에, CLI는 터미널에,
/// 백그라운드 훅은 아무 데도 안 보내고 취소값을 답한다.
pub trait UiSink {
    fn ask(&mut self, plugin: &str, req: UiRequest) -> UiAnswer;
}

pub struct NoUiSink;

impl UiSink for NoUiSink {
    fn ask(&mut self, _plugin: &str, req: UiRequest) -> UiAnswer {
        UiAnswer::cancel_for(&req)
    }
}

/// CLI 싱크. stdin이 터미널이면 묻고, 아니면(에이전트가 부를 때) 취소값을 즉시 답한다.
pub struct CliSink;

impl UiSink for CliSink {
    fn ask(&mut self, plugin: &str, req: UiRequest) -> UiAnswer {
        use std::io::IsTerminal;
        if !std::io::stdin().is_terminal() {
            return UiAnswer::cancel_for(&req);
        }
        match req {
            UiRequest::Pick { title, items } => {
                let items: Vec<crate::interactive::SelectItem> = items
                    .iter()
                    .enumerate()
                    .map(|(i, s)| crate::interactive::SelectItem { key: i.to_string(), display: s.clone() })
                    .collect();
                eprintln!("{}: {}", plugin, title.unwrap_or_default());
                let picked = crate::interactive::interactive_select(&items).ok().flatten();
                UiAnswer::Index { index: picked.and_then(|k| k.parse().ok()) }
            }
            UiRequest::Prompt { title, default } => UiAnswer::Text {
                text: crate::interactive::prompt_line(&title.unwrap_or_else(|| plugin.to_string()), &default.unwrap_or_default()),
            },
            UiRequest::Confirm { title } => UiAnswer::Yes {
                yes: crate::interactive::prompt_yes_no(&title.unwrap_or_else(|| plugin.to_string())),
            },
            UiRequest::Progress { text } => {
                eprintln!("{}: {}", plugin, text);
                UiAnswer::Ack {}
            }
        }
    }
}

// ── 오류 ────────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum PluginError {
    Spawn(String),
    Exited(Option<i32>),
    BadJson(String),
    Cancelled,
    Protocol(String),
}

impl std::fmt::Display for PluginError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PluginError::Spawn(e) => write!(f, "failed to start: {}", e),
            PluginError::Exited(Some(c)) => write!(f, "exited with code {} (see stderr.log in the plugin directory)", c),
            PluginError::Exited(None) => write!(f, "exited by signal (see stderr.log in the plugin directory)"),
            PluginError::BadJson(s) => write!(f, "invalid response: {}", s),
            PluginError::Cancelled => write!(f, "cancelled"),
            PluginError::Protocol(s) => write!(f, "protocol error: {}", s),
        }
    }
}

// ── 호스트 ──────────────────────────────────────────────────────────────────

struct Pipes {
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

/// 플러그인 하나의 프로세스 자리. `io`는 요청 중에 잠기고, `child`는 kill/wait 전용으로
/// 분리되어 있어 요청이 진행 중인 동안에도 다른 스레드가 kill할 수 있다.
struct PluginSlot {
    manifest: Manifest,
    io: Mutex<Option<Pipes>>,
    child: Mutex<Option<Child>>,
    killed: AtomicBool,
}

pub struct PluginHost {
    manifests: Vec<Manifest>,
    slots: BTreeMap<String, Arc<PluginSlot>>,
    commands: PluginCommands,
    hooks: HashMap<HookKind, Vec<PluginCmdId>>,
    config_tables: RwLock<BTreeMap<String, Value>>,
    env: PluginEnv,
}

impl PluginHost {
    pub fn new(manifests: Vec<Manifest>, config_tables: BTreeMap<String, Value>, env: PluginEnv) -> PluginHost {
        let mut slots = BTreeMap::new();
        let mut list = Vec::new();
        let mut hooks: HashMap<HookKind, Vec<PluginCmdId>> = HashMap::new();
        for m in &manifests {
            for c in &m.commands {
                list.push(PluginCommand {
                    plugin: m.name.clone(),
                    id: c.id.clone(),
                    desc: c.desc.clone(),
                    key: c.key.clone(),
                    layers: c.layers.clone(),
                    menu: c.menu,
                });
            }
            slots.insert(
                m.name.clone(),
                Arc::new(PluginSlot {
                    manifest: m.clone(),
                    io: Mutex::new(None),
                    child: Mutex::new(None),
                    killed: AtomicBool::new(false),
                }),
            );
        }
        let commands = PluginCommands { list };
        for m in &manifests {
            for h in &m.hooks {
                if let Some(id) = commands.find(&format!("{}.{}", m.name, h.run)) {
                    hooks.entry(h.on).or_default().push(id);
                }
            }
        }
        PluginHost { manifests, slots, commands, hooks, config_tables: RwLock::new(config_tables), env }
    }

    pub fn commands(&self) -> &PluginCommands {
        &self.commands
    }

    /// Settings 화면이 `[plugins.<name>]`을 바꾼 뒤 부른다. 다음 요청부터 새 값이 간다.
    pub fn update_config_tables(&self, tables: BTreeMap<String, Value>) {
        *self.config_tables.write().unwrap_or_else(|p| p.into_inner()) = tables;
    }

    pub fn hooks(&self, kind: HookKind) -> &[PluginCmdId] {
        self.hooks.get(&kind).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn manifests(&self) -> &[Manifest] {
        &self.manifests
    }

    /// 요청 컨텍스트. `config`는 `[plugins.<name>]`에서 `enabled`를 뺀 것, 없으면 `{}`.
    pub fn context(
        &self,
        plugin: &str,
        focus: Option<String>,
        collection: Option<String>,
        entry: Option<Entry>,
        entries: Vec<Entry>,
        hook: Option<Value>,
    ) -> Context {
        let mut config = self
            .config_tables
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .get(plugin)
            .cloned()
            .unwrap_or_else(|| Value::Object(Default::default()));
        if let Some(obj) = config.as_object_mut() {
            obj.remove("enabled");
        }
        Context { focus, collection, entry, entries, config, paths: self.env.paths(), hook }
    }

    /// 살아 있는 프로세스의 pid. 테스트용.
    #[cfg(test)]
    pub fn pid(&self, plugin: &str) -> Option<u32> {
        let slot = self.slots.get(plugin)?;
        let guard = slot.child.lock().ok()?;
        guard.as_ref().map(|c| c.id())
    }

    fn spawn(&self, slot: &PluginSlot) -> Result<(), PluginError> {
        let m = &slot.manifest;
        let log = std::fs::File::create(m.dir.join("stderr.log")).map_err(|e| PluginError::Spawn(e.to_string()))?;
        let mut cmd = Command::new(&m.run[0]);
        // 자기 프로세스 그룹에 띄운다. 플러그인이 자식(예: `sleep`, `pdftotext`)을 띄운 채 죽으면
        // 그 자식이 stdout 파이프를 쥐고 있어 `read_line`이 영원히 안 풀린다. kill은 그룹째 한다.
        cmd.args(&m.run[1..])
            .current_dir(&m.dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::from(log))
            .process_group(0);
        apply_env(&mut cmd, &self.env, &m.dir);
        let mut child = cmd.spawn().map_err(|e| PluginError::Spawn(format!("{}: {}", m.run[0], e)))?;
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        *slot.child.lock().unwrap_or_else(|p| p.into_inner()) = Some(child);
        *slot.io.lock().unwrap_or_else(|p| p.into_inner()) = Some(Pipes { stdin, stdout: BufReader::new(stdout) });
        slot.killed.store(false, Ordering::SeqCst);
        Ok(())
    }

    /// 프로세스가 없으면 띄운다. 요청 없이 미리 띄우는 용도(테스트. 워밍업이 필요해지면 cfg를 뗀다).
    #[cfg(test)]
    pub fn ensure_running(&self, plugin: &str) -> Result<(), PluginError> {
        let slot = self.slots.get(plugin).ok_or_else(|| PluginError::Protocol(format!("no such plugin {}", plugin)))?;
        let running = slot.io.lock().unwrap_or_else(|p| p.into_inner()).is_some();
        if !running {
            self.spawn(slot)?;
        }
        Ok(())
    }

    /// 프로세스를 잃었을 때 종료 코드를 회수하고 자리를 비운다. `io` 잠금은 호출자가 들고 있다.
    fn reap(&self, slot: &PluginSlot, io: &mut Option<Pipes>) -> PluginError {
        *io = None;
        let killed = slot.killed.swap(false, Ordering::SeqCst);
        let status = slot.child.lock().unwrap_or_else(|p| p.into_inner()).take().and_then(|mut c| c.wait().ok());
        if killed {
            return PluginError::Cancelled;
        }
        PluginError::Exited(status.and_then(|s| s.code()))
    }

    /// 요청 하나를 보내고 최종 응답을 받을 때까지 돈다. 그 사이 UI 요청은 `ui`로 넘긴다.
    /// 플러그인당 한 번에 하나만 도는 것은 `io` 뮤텍스가 보장한다.
    pub fn invoke(
        &self,
        cmd: PluginCmdId,
        trigger: &str,
        context: Context,
        ui: &mut dyn UiSink,
    ) -> Result<Final, PluginError> {
        let command = self.commands.get(cmd).ok_or_else(|| PluginError::Protocol(format!("unknown command {:?}", cmd)))?;
        let plugin = command.plugin.clone();
        let slot = self.slots.get(&plugin).ok_or_else(|| PluginError::Protocol(format!("no such plugin {}", plugin)))?;

        let mut io = slot.io.lock().unwrap_or_else(|p| p.into_inner());
        if io.is_none() {
            drop(io);
            self.spawn(slot)?;
            io = slot.io.lock().unwrap_or_else(|p| p.into_inner());
        }

        let request = Request { r#type: "command".to_string(), id: command.id.clone(), trigger: trigger.to_string(), context };
        let line = serde_json::to_string(&request).map_err(|e| PluginError::Protocol(e.to_string()))?;
        if write_line(&mut io, &line).is_err() {
            return Err(self.reap(slot, &mut io));
        }

        loop {
            let mut buf = String::new();
            let n = io.as_mut().expect("pipes present").stdout.read_line(&mut buf).unwrap_or(0);
            if n == 0 {
                return Err(self.reap(slot, &mut io));
            }
            match parse_plugin_line(buf.trim_end()) {
                Ok(PluginMsg::Final(f)) => return Ok(f),
                Ok(PluginMsg::Ui(req)) => {
                    let answer = ui.ask(&plugin, req);
                    let reply = serde_json::to_string(&answer).map_err(|e| PluginError::Protocol(e.to_string()))?;
                    if write_line(&mut io, &reply).is_err() {
                        return Err(self.reap(slot, &mut io));
                    }
                }
                Err(e) => {
                    // 프로토콜이 어긋난 프로세스와는 더 대화할 수 없다. 죽이고 다음 호출에서 다시 띄운다.
                    let short = if e.starts_with("not json") { head(buf.trim_end()) } else { e };
                    self.terminate(slot, &mut io);
                    return Err(PluginError::BadJson(short));
                }
            }
        }
    }

    fn terminate(&self, slot: &PluginSlot, io: &mut Option<Pipes>) {
        *io = None;
        if let Some(mut c) = slot.child.lock().unwrap_or_else(|p| p.into_inner()).take() {
            kill_group(&mut c);
            let _ = c.wait();
        }
        slot.killed.store(false, Ordering::SeqCst);
    }

    /// 다른 스레드에서 부른다. 진행 중인 `invoke`는 EOF를 만나 `Cancelled`로 끝난다.
    pub fn kill(&self, plugin: &str) {
        let Some(slot) = self.slots.get(plugin) else { return };
        slot.killed.store(true, Ordering::SeqCst);
        if let Some(c) = slot.child.lock().unwrap_or_else(|p| p.into_inner()).as_mut() {
            kill_group(c);
        }
    }

    /// stdin을 닫아 루프가 EOF로 끝나게 하고, 1초 안에 안 끝나면 죽인다.
    pub fn shutdown(&self) {
        for slot in self.slots.values() {
            if let Ok(mut io) = slot.io.lock() {
                *io = None; // stdin drop -> 플러그인 쪽 EOF
            }
            let mut guard = slot.child.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(mut c) = guard.take() {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
                loop {
                    match c.try_wait() {
                        Ok(Some(_)) => break,
                        Ok(None) if std::time::Instant::now() < deadline => {
                            std::thread::sleep(std::time::Duration::from_millis(25));
                        }
                        _ => {
                            kill_group(&mut c);
                            let _ = c.wait();
                            break;
                        }
                    }
                }
            }
        }
    }
}

/// 호스트를 갈아 끼울 때(설치·제거) 옛 호스트의 프로세스가 고아로 남지 않도록.
impl Drop for PluginHost {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// 프로세스 그룹 전체에 SIGKILL. 플러그인이 띄운 자식까지 같이 죽여야 파이프가 닫힌다.
/// pgid는 `process_group(0)`으로 띄웠으므로 pid와 같다.
fn kill_group(child: &mut Child) {
    let _ = Command::new("kill")
        .args(["-9", "--", &format!("-{}", child.id())])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = child.kill();
}

fn write_line(io: &mut Option<Pipes>, line: &str) -> std::io::Result<()> {
    let p = io.as_mut().expect("pipes present");
    p.stdin.write_all(line.as_bytes())?;
    p.stdin.write_all(b"\n")?;
    p.stdin.flush()
}

/// 프로토콜 프로세스와 `[cli]` 프로세스가 같은 변수를 받는다.
pub fn apply_env(cmd: &mut Command, env: &PluginEnv, plugin_dir: &std::path::Path) {
    cmd.env("BIBOX_BIN", &env.bin)
        .env("BIBOX_API", crate::plugin::manifest::SUPPORTED_API.to_string())
        .env("BIBOX_CONFIG_DIR", &env.config_dir)
        .env("BIBOX_DB_PATH", &env.db)
        .env("BIBOX_NOTES_DIR", &env.notes)
        .env("BIBOX_PDF_DIR", &env.pdfs)
        .env("BIBOX_PLUGIN_DIR", plugin_dir);
    match &env.home {
        Some(h) => {
            cmd.env("BIBOX_HOME", h);
        }
        None => {
            cmd.env_remove("BIBOX_HOME");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::manifest::parse_manifest;
    use std::path::Path;

    fn fixtures() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/plugins")
    }

    fn env() -> PluginEnv {
        PluginEnv {
            bin: PathBuf::from("/usr/bin/true"),
            config_dir: PathBuf::from("/tmp/bibox-test"),
            db: PathBuf::from("/tmp/bibox-test/db.json"),
            notes: PathBuf::from("/tmp/bibox-test/notes"),
            pdfs: PathBuf::from("/tmp/bibox-test/pdfs"),
            home: None,
        }
    }

    /// fixture 스크립트 하나를 플러그인 `name`으로 삼는 호스트. 디렉토리는 테스트마다 고유하게
    /// 만든다(같은 이름을 쓰는 테스트가 병렬로 돌면서 서로의 디렉토리를 지우지 않도록).
    fn host_for(name: &str, script: &str) -> (PluginHost, PathBuf) {
        static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("bibox-host-{}-{}-{}", name, std::process::id(), seq)).join(name);
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
        std::fs::create_dir_all(&dir).unwrap();
        let text = format!(
            "api = 1\nname = \"{}\"\nrun = \"sh {}\"\n[[commands]]\nid = \"go\"\ndesc = \"Go\"\n[[hooks]]\non = \"after_write\"\nrun = \"go\"\n",
            name,
            fixtures().join(script).display()
        );
        let mut problems = vec![];
        let m = parse_manifest(&dir, &text, &mut problems).unwrap();
        (PluginHost::new(vec![m], BTreeMap::new(), env()), dir)
    }

    fn ctx(host: &PluginHost, plugin: &str) -> Context {
        host.context(plugin, None, None, None, vec![], None)
    }

    struct Picks(Vec<usize>);
    impl UiSink for Picks {
        fn ask(&mut self, _plugin: &str, req: UiRequest) -> UiAnswer {
            match req {
                UiRequest::Pick { .. } => UiAnswer::Index { index: self.0.pop() },
                other => UiAnswer::cancel_for(&other),
            }
        }
    }

    #[test]
    fn commands_and_hooks_are_built_from_the_manifests() {
        let (host, _) = host_for("alpha", "echo.sh");
        assert_eq!(host.commands().len(), 1);
        let id = host.commands().find("alpha.go").unwrap();
        assert_eq!(host.commands().get(id).unwrap().full_name(), "alpha.go");
        assert_eq!(host.commands().find("alpha.nope"), None);
        assert_eq!(host.hooks(HookKind::AfterWrite), &[id]);
        assert!(host.hooks(HookKind::BeforeAdd).is_empty());
        assert_eq!(host.manifests().len(), 1);
    }

    #[test]
    fn updated_config_tables_reach_the_next_context() {
        let (host, _) = host_for("alpha", "echo.sh");
        assert_eq!(ctx(&host, "alpha").config, serde_json::json!({}));
        let mut tables = BTreeMap::new();
        tables.insert("alpha".to_string(), serde_json::json!({ "push": true }));
        host.update_config_tables(tables);
        assert_eq!(ctx(&host, "alpha").config, serde_json::json!({ "push": true }));
    }

    #[test]
    fn context_carries_the_plugin_config_table_without_enabled() {
        let (h, _) = host_for("alpha", "echo.sh");
        let m = h.manifests()[0].clone();
        let mut tables = BTreeMap::new();
        tables.insert("alpha".to_string(), serde_json::json!({"model": "x", "enabled": true}));
        let host = PluginHost::new(vec![m], tables, env());
        let c = ctx(&host, "alpha");
        assert_eq!(c.config, serde_json::json!({"model": "x"}));
        assert_eq!(c.paths.db, PathBuf::from("/tmp/bibox-test/db.json"));
    }

    #[test]
    fn a_request_round_trips_and_the_process_sees_cwd_and_env() {
        let (host, dir) = host_for("alpha", "echo.sh");
        let id = host.commands().find("alpha.go").unwrap();
        let f = host.invoke(id, "key", ctx(&host, "alpha"), &mut NoUiSink).unwrap();
        let msg = f.message.unwrap();
        assert!(msg.contains(&format!("cwd={}", dir.canonicalize().unwrap().display())), "{}", msg);
        assert!(msg.contains(&format!("dir={}", dir.display())), "{}", msg);
        assert!(msg.contains("api=1"), "{}", msg);
        host.shutdown();
    }

    #[test]
    fn a_second_request_reuses_the_same_process() {
        let (host, _) = host_for("beta", "echo.sh");
        let id = host.commands().find("beta.go").unwrap();
        host.invoke(id, "key", ctx(&host, "beta"), &mut NoUiSink).unwrap();
        let pid1 = host.pid("beta");
        host.invoke(id, "key", ctx(&host, "beta"), &mut NoUiSink).unwrap();
        assert_eq!(pid1, host.pid("beta"));
        assert!(pid1.is_some());
        host.shutdown();
    }

    #[test]
    fn a_ui_request_goes_to_the_sink_and_the_answer_reaches_the_plugin() {
        let (host, _) = host_for("gamma", "ui.sh");
        let id = host.commands().find("gamma.go").unwrap();
        let f = host.invoke(id, "key", ctx(&host, "gamma"), &mut Picks(vec![1])).unwrap();
        assert_eq!(f.message.as_deref(), Some("picked index:1"));
        let f = host.invoke(id, "key", ctx(&host, "gamma"), &mut NoUiSink).unwrap();
        assert_eq!(f.message.as_deref(), Some("picked index:null"));
        host.shutdown();
    }

    #[test]
    fn a_crash_is_reported_with_the_exit_code_and_the_log_and_the_next_call_restarts() {
        let (host, dir) = host_for("delta", "crash.sh");
        let id = host.commands().find("delta.go").unwrap();
        match host.invoke(id, "key", ctx(&host, "delta"), &mut NoUiSink) {
            Err(PluginError::Exited(Some(1))) => {}
            other => panic!("expected Exited(Some(1)), got {:?}", other),
        }
        let log = std::fs::read_to_string(dir.join("stderr.log")).unwrap();
        assert!(log.contains("boom"));
        // 다음 호출은 새 프로세스를 띄운다 (또 죽지만, 살아 있는 프로세스가 없었다는 뜻이다)
        assert!(matches!(host.invoke(id, "key", ctx(&host, "delta"), &mut NoUiSink), Err(PluginError::Exited(Some(1)))));
        host.shutdown();
    }

    #[test]
    fn garbage_output_is_a_bad_json_error_with_the_first_80_chars() {
        let (host, _) = host_for("epsilon", "garbage.sh");
        let id = host.commands().find("epsilon.go").unwrap();
        match host.invoke(id, "key", ctx(&host, "epsilon"), &mut NoUiSink) {
            Err(PluginError::BadJson(s)) => {
                assert!(s.starts_with("Traceback"), "{}", s);
                assert!(s.len() <= 100, "{}", s);
            }
            other => panic!("expected BadJson, got {:?}", other),
        }
        host.shutdown();
    }

    #[test]
    fn kill_from_another_thread_turns_into_cancelled() {
        let (host, _) = host_for("zeta", "hang.sh");
        let host = Arc::new(host);
        let id = host.commands().find("zeta.go").unwrap();
        let h2 = Arc::clone(&host);
        let killer = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(300));
            h2.kill("zeta");
        });
        let r = host.invoke(id, "key", ctx(&host, "zeta"), &mut NoUiSink);
        killer.join().unwrap();
        assert!(matches!(r, Err(PluginError::Cancelled)), "{:?}", r);
        host.shutdown();
    }

    #[test]
    fn shutdown_closes_stdin_and_reaps_within_a_second() {
        let (host, _) = host_for("eta", "hang.sh");
        host.ensure_running("eta").unwrap();
        assert!(host.pid("eta").is_some());
        let t = std::time::Instant::now();
        host.shutdown();
        assert!(t.elapsed() < std::time::Duration::from_secs(3));
        assert!(host.pid("eta").is_none());
    }

    #[test]
    fn a_missing_program_is_a_spawn_error() {
        // 이름은 디렉토리 이름과 같아야 하므로 디렉토리를 "m"으로 만든다
        let dir = std::env::temp_dir().join(format!("bibox-host-missing-{}", std::process::id())).join("m");
        std::fs::create_dir_all(&dir).unwrap();
        let mut problems = vec![];
        let m = parse_manifest(&dir, "api = 1\nname = \"m\"\nrun = \"definitely-not-a-program-xyz\"\n[[commands]]\nid = \"go\"\ndesc = \"Go\"\n", &mut problems).unwrap();
        let host = PluginHost::new(vec![m], BTreeMap::new(), env());
        let id = host.commands().find("m.go").unwrap();
        assert!(matches!(host.invoke(id, "key", ctx(&host, "m"), &mut NoUiSink), Err(PluginError::Spawn(_))));
    }

    #[test]
    fn cli_sink_answers_cancel_when_stdin_is_not_a_terminal() {
        // cargo test 아래서는 stdin이 TTY가 아니다. 그 경로만 검사한다.
        let mut s = CliSink;
        assert_eq!(s.ask("p", UiRequest::Pick { title: None, items: vec!["a".into()] }), UiAnswer::Index { index: None });
        assert_eq!(s.ask("p", UiRequest::Prompt { title: None, default: None }), UiAnswer::Text { text: None });
        assert_eq!(s.ask("p", UiRequest::Confirm { title: None }), UiAnswer::Yes { yes: false });
        assert_eq!(s.ask("p", UiRequest::Progress { text: "x".into() }), UiAnswer::Ack {});
    }

    #[test]
    fn plugin_error_displays_point_at_the_log() {
        assert_eq!(PluginError::Exited(Some(1)).to_string(), "exited with code 1 (see stderr.log in the plugin directory)");
        assert_eq!(PluginError::Exited(None).to_string(), "exited by signal (see stderr.log in the plugin directory)");
        assert_eq!(PluginError::Cancelled.to_string(), "cancelled");
        assert_eq!(PluginError::BadJson("x".into()).to_string(), "invalid response: x");
    }
}
