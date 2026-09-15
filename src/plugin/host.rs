use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use crate::config::Config;
use crate::keymap::{KeyPress, LayerId};
use crate::plugin::manifest::Manifest;
use crate::plugin::protocol::{Capabilities, InitializeParams, InitializeResult, Paths, UiAnswer, UiRequest, PROTOCOL};
use crate::plugin::rpc::{self, Id, Message, RpcError};

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
    pub menus: Vec<String>,
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
    /// 테스트와 doctor용 추가 환경 변수. 프로토콜 프로세스와 `[cli]` 프로세스 둘 다 받는다.
    pub extra: BTreeMap<String, String>,
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
            extra: BTreeMap::new(),
        }
    }

    pub fn paths(&self) -> Paths {
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

#[cfg(test)]
pub struct NoUiSink;

#[cfg(test)]
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

// ── 오류와 이벤트 ────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum PluginError {
    Spawn(String),
    Exited(Option<i32>),
    Timeout,
    Cancelled,
    Rpc(RpcError),
    Protocol(String),
}

impl std::fmt::Display for PluginError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PluginError::Spawn(e) => write!(f, "failed to start: {}", e),
            PluginError::Exited(Some(c)) => write!(f, "exited with code {} (see stderr.log in the plugin directory)", c),
            PluginError::Exited(None) => write!(f, "exited (see stderr.log in the plugin directory)"),
            PluginError::Timeout => write!(f, "no answer (the plugin stayed silent too long)"),
            PluginError::Cancelled => write!(f, "cancelled"),
            PluginError::Rpc(e) => write!(f, "{}", e.message),
            PluginError::Protocol(s) => write!(f, "protocol error: {}", s),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    Request { id: Id, method: String, params: Value },
    Notification { method: String, params: Value },
}

#[derive(Debug, Clone, PartialEq)]
pub enum HostEvent {
    Incoming { plugin: String, msg: Incoming },
    Exited { plugin: String, status: Option<i32> },
}

#[derive(Debug, Clone, PartialEq)]
pub struct PluginView {
    pub plugin: String,
    pub title: String,
    pub run: String,
}

/// 플러그인이 죽었을 때 대기 중인 호출에 보내는 사설 코드. 사용자에게 보이지 않는다.
const EXITED: i64 = -32001;

struct Slot {
    manifest: Manifest,
    stdin: Mutex<Option<ChildStdin>>,
    child: Mutex<Option<Child>>,
    pending: Mutex<HashMap<Id, Sender<Result<Value, RpcError>>>>,
    next_id: AtomicU64,
    killed: AtomicBool,
    ready: AtomicBool,
}

pub struct PluginHost {
    manifests: Vec<Manifest>,
    slots: BTreeMap<String, Arc<Slot>>,
    commands: PluginCommands,
    views: Vec<PluginView>,
    config_tables: RwLock<BTreeMap<String, Value>>,
    env: PluginEnv,
    capabilities: RwLock<Capabilities>,
    events_tx: Sender<HostEvent>,
    events_rx: Mutex<Receiver<HostEvent>>,
}

impl PluginHost {
    pub fn new(manifests: Vec<Manifest>, config_tables: BTreeMap<String, Value>, env: PluginEnv) -> PluginHost {
        let mut slots = BTreeMap::new();
        let mut list = Vec::new();
        let mut views = Vec::new();
        for m in &manifests {
            for c in &m.commands {
                list.push(PluginCommand { plugin: m.name.clone(), id: c.id.clone(), desc: c.desc.clone(), key: c.key.clone(), layers: c.layers.clone(), menus: c.menus.clone() });
            }
            for v in &m.views {
                views.push(PluginView { plugin: m.name.clone(), title: v.title.clone(), run: v.run.clone() });
            }
            slots.insert(
                m.name.clone(),
                Arc::new(Slot { manifest: m.clone(), stdin: Mutex::new(None), child: Mutex::new(None), pending: Mutex::new(HashMap::new()), next_id: AtomicU64::new(1), killed: AtomicBool::new(false), ready: AtomicBool::new(false) }),
            );
        }
        let (events_tx, events_rx) = channel();
        PluginHost {
            manifests,
            slots,
            commands: PluginCommands { list },
            views,
            config_tables: RwLock::new(config_tables),
            env,
            capabilities: RwLock::new(Capabilities { images: false, status_bar: true }),
            events_tx,
            events_rx: Mutex::new(events_rx),
        }
    }

    pub fn commands(&self) -> &PluginCommands { &self.commands }
    pub fn views(&self) -> &[PluginView] { &self.views }
    pub fn manifests(&self) -> &[Manifest] { &self.manifests }
    pub fn set_capabilities(&self, c: Capabilities) { *self.capabilities.write().unwrap_or_else(|p| p.into_inner()) = c; }

    /// Settings 화면이 `[plugins.<name>]`을 바꾼 뒤 부른다. 다음 initialize부터 새 값이 간다(떠 있는 것은 `config/changed`로).
    pub fn update_config_tables(&self, tables: BTreeMap<String, Value>) {
        *self.config_tables.write().unwrap_or_else(|p| p.into_inner()) = tables;
    }

    pub fn try_recv_event(&self) -> Option<HostEvent> {
        self.events_rx.lock().ok()?.try_recv().ok()
    }

    pub fn subscribers(&self, event: &str) -> Vec<String> {
        self.manifests.iter().filter(|m| m.events.iter().any(|e| e == event)).map(|m| m.name.clone()).collect()
    }

    pub fn startup_plugins(&self) -> Vec<String> {
        self.manifests.iter().filter(|m| m.activation == crate::plugin::manifest::Activation::Startup).map(|m| m.name.clone()).collect()
    }

    pub fn is_running(&self, plugin: &str) -> bool {
        self.slots.get(plugin).map(|s| s.ready.load(Ordering::SeqCst)).unwrap_or(false)
    }

    fn slot(&self, plugin: &str) -> Result<&Arc<Slot>, PluginError> {
        self.slots.get(plugin).ok_or_else(|| PluginError::Protocol(format!("no such plugin {}", plugin)))
    }

    /// 안 떠 있으면 띄우고 initialize까지. 두 스레드가 동시에 불러도 프로세스는 하나(stdin 락 안에서 검사).
    pub fn ensure_running(&self, plugin: &str) -> Result<(), PluginError> {
        let slot = self.slot(plugin)?;
        if slot.ready.load(Ordering::SeqCst) {
            return Ok(());
        }
        {
            let mut stdin = slot.stdin.lock().unwrap_or_else(|p| p.into_inner());
            if stdin.is_some() {
                return Ok(()); // 다른 스레드가 initialize 중
            }
            let m = &slot.manifest;
            let log = std::fs::File::create(m.dir.join("stderr.log")).map_err(|e| PluginError::Spawn(e.to_string()))?;
            let mut cmd = Command::new(&m.run[0]);
            cmd.args(&m.run[1..]).current_dir(&m.dir).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::from(log)).process_group(0);
            apply_env(&mut cmd, &self.env, &m.dir);
            let mut child = cmd.spawn().map_err(|e| PluginError::Spawn(format!("{}: {}", m.run[0], e)))?;
            let out = child.stdout.take().expect("piped stdout");
            *stdin = child.stdin.take();
            *slot.child.lock().unwrap_or_else(|p| p.into_inner()) = Some(child);
            slot.killed.store(false, Ordering::SeqCst);
            let (name, slot2, tx) = (plugin.to_string(), Arc::clone(slot), self.events_tx.clone());
            std::thread::spawn(move || reader(name, slot2, out, tx));
        }
        let config = self.config_tables.read().unwrap_or_else(|p| p.into_inner()).get(plugin).cloned().unwrap_or_else(|| Value::Object(Default::default()));
        let params = InitializeParams { protocol: PROTOCOL, bibox: env!("CARGO_PKG_VERSION").to_string(), paths: self.env.paths(), config, capabilities: self.capabilities.read().unwrap_or_else(|p| p.into_inner()).clone() };
        let v = self.call_raw(slot, "initialize", serde_json::to_value(params).unwrap_or(Value::Null), Some(Duration::from_secs(5)))?;
        let r: InitializeResult = serde_json::from_value(v).unwrap_or_default();
        if r.protocol != PROTOCOL {
            self.terminate(slot);
            return Err(PluginError::Protocol(format!("plugin speaks protocol {}, bibox needs {}", r.protocol, PROTOCOL)));
        }
        slot.ready.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn call_raw(&self, slot: &Arc<Slot>, method: &str, params: Value, timeout: Option<Duration>) -> Result<Value, PluginError> {
        let id = slot.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = channel();
        slot.pending.lock().unwrap_or_else(|p| p.into_inner()).insert(id, tx);
        if let Err(e) = self.write(slot, &Message::Request { id, method: method.to_string(), params }) {
            slot.pending.lock().unwrap_or_else(|p| p.into_inner()).remove(&id);
            return Err(e);
        }
        let got = match timeout {
            Some(t) => match rx.recv_timeout(t) {
                Ok(r) => r,
                Err(RecvTimeoutError::Timeout) => {
                    slot.pending.lock().unwrap_or_else(|p| p.into_inner()).remove(&id);
                    return Err(PluginError::Timeout);
                }
                Err(RecvTimeoutError::Disconnected) => return Err(PluginError::Exited(None)),
            },
            None => rx.recv().map_err(|_| PluginError::Exited(None))?,
        };
        got.map_err(|e| if e.code == EXITED { PluginError::Exited(e.data.as_ref().and_then(Value::as_i64).map(|c| c as i32)) } else { PluginError::Rpc(e) })
    }

    pub fn call(&self, plugin: &str, method: &str, params: Value, timeout: Option<Duration>) -> Result<Value, PluginError> {
        self.ensure_running(plugin)?;
        let slot = self.slot(plugin)?;
        if slot.killed.load(Ordering::SeqCst) {
            return Err(PluginError::Cancelled);
        }
        self.call_raw(slot, method, params, timeout)
    }

    /// 플러그인이 이만큼 아무 말도 없으면 `call_with_ui`가 끊는다. 팝업 요청과 진행 알림도 "말"이다.
    pub const IDLE_LIMIT: Duration = Duration::from_secs(30);

    /// CLI에서: 답을 기다리는 동안 이벤트를 비우며 `window/*` 요청에 sink로 답한다. 다른 요청은 method not found.
    /// `idle` 동안 이 플러그인에게서 아무것도 안 오면 Timeout. 시계는 플러그인이 보낸 것(요청이든 알림이든)이 올 때마다 다시 간다.
    /// 팝업 안에서 사용자가 오래 고민하는 시간은 안 센다(요청이 온 순간 리셋되고 답은 그 뒤에 나가므로).
    pub fn call_with_ui_idle(&self, plugin: &str, method: &str, params: Value, sink: &mut dyn UiSink, idle: Duration) -> Result<Value, PluginError> {
        self.ensure_running(plugin)?;
        let slot = self.slot(plugin)?;
        let id = slot.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = channel();
        slot.pending.lock().unwrap_or_else(|p| p.into_inner()).insert(id, tx);
        self.write(slot, &Message::Request { id, method: method.to_string(), params })?;
        let mut last = std::time::Instant::now();
        loop {
            match rx.recv_timeout(Duration::from_millis(30)) {
                Ok(r) => return r.map_err(|e| if e.code == EXITED { PluginError::Exited(None) } else { PluginError::Rpc(e) }),
                Err(RecvTimeoutError::Disconnected) => return Err(PluginError::Exited(None)),
                Err(RecvTimeoutError::Timeout) => {
                    while let Some(ev) = self.try_recv_event() {
                        let from_this = matches!(&ev, HostEvent::Incoming { plugin: p, .. } if p == plugin);
                        self.answer_from_sink(ev, sink);
                        // 답한 뒤에 리셋: 팝업 안에서 사용자가 쓴 시간은 플러그인의 침묵이 아니다
                        if from_this {
                            last = std::time::Instant::now();
                        }
                    }
                    if last.elapsed() > idle {
                        slot.pending.lock().unwrap_or_else(|p| p.into_inner()).remove(&id);
                        return Err(PluginError::Timeout);
                    }
                }
            }
        }
    }

    /// 이벤트 하나를 CLI 방식으로 처리한다. 팝업은 sink, 메시지는 stderr, 나머지 알림은 버린다.
    pub fn answer_from_sink(&self, ev: HostEvent, sink: &mut dyn UiSink) {
        match ev {
            HostEvent::Incoming { plugin, msg: Incoming::Request { id, method, params } } => {
                let result = match UiRequest::from_method(&method, &params) {
                    Some(req) => Ok(sink.ask(&plugin, req.clone()).to_result()),
                    None => Err(RpcError::method_not_found(&method)),
                };
                self.respond(&plugin, id, result);
            }
            HostEvent::Incoming { plugin, msg: Incoming::Notification { method, params } } => {
                if method == "window/message" {
                    eprintln!("{}: {}", plugin, params.get("text").and_then(Value::as_str).unwrap_or(""));
                }
            }
            HostEvent::Exited { .. } => {}
        }
    }

    pub fn notify(&self, plugin: &str, method: &str, params: Value) -> Result<(), PluginError> {
        self.ensure_running(plugin)?;
        let slot = self.slot(plugin)?;
        self.write(slot, &Message::Notification { method: method.to_string(), params })
    }

    pub fn respond(&self, plugin: &str, id: Id, result: Result<Value, RpcError>) {
        let Ok(slot) = self.slot(plugin) else { return };
        let msg = match result {
            Ok(v) => Message::Response { id, result: v },
            Err(e) => Message::Error { id: Some(id), error: e },
        };
        let _ = self.write(slot, &msg);
    }

    /// 구독자 전부에게 알림. 안 떠 있으면 띄운다. (플러그인 이름, 결과) 목록.
    pub fn emit(&self, event: &str, params: Value) -> Vec<(String, Result<(), PluginError>)> {
        self.subscribers(event).into_iter().map(|p| { let r = self.notify(&p, event, params.clone()); (p, r) }).collect()
    }

    fn write(&self, slot: &Slot, msg: &Message) -> Result<(), PluginError> {
        let mut guard = slot.stdin.lock().unwrap_or_else(|p| p.into_inner());
        let Some(stdin) = guard.as_mut() else { return Err(PluginError::Exited(None)) };
        let line = rpc::to_line(msg);
        if stdin.write_all(line.as_bytes()).is_err() || stdin.write_all(b"\n").is_err() || stdin.flush().is_err() {
            *guard = None;
            return Err(PluginError::Exited(None));
        }
        Ok(())
    }

    fn terminate(&self, slot: &Slot) {
        *slot.stdin.lock().unwrap_or_else(|p| p.into_inner()) = None;
        slot.ready.store(false, Ordering::SeqCst);
        if let Some(mut c) = slot.child.lock().unwrap_or_else(|p| p.into_inner()).take() {
            kill_group(&mut c);
            let _ = c.wait();
        }
    }

    pub fn kill(&self, plugin: &str) {
        let Some(slot) = self.slots.get(plugin) else { return };
        slot.killed.store(true, Ordering::SeqCst);
        self.terminate(slot);
    }

    /// shutdown 요청 뒤 2초 기다리고 남은 것은 kill. Drop에서도 부른다.
    pub fn shutdown(&self) {
        self.shutdown_with(Duration::from_secs(2));
    }

    /// `grace` 동안 shutdown 답과 종료를 기다린다. CLI는 플러그인이 알림을 다 처리하도록 길게 준다.
    pub fn shutdown_with(&self, grace: Duration) {
        for (name, slot) in &self.slots {
            if !slot.ready.load(Ordering::SeqCst) && slot.child.lock().unwrap_or_else(|p| p.into_inner()).is_none() {
                continue;
            }
            // 답 기다림과 종료 기다림이 같은 유예를 나눠 쓴다. 답을 안 하는 플러그인도 grace 뒤에는 죽는다
            let deadline = std::time::Instant::now() + grace;
            let _ = self.call_raw(slot, "shutdown", Value::Object(Default::default()), Some(grace));
            *slot.stdin.lock().unwrap_or_else(|p| p.into_inner()) = None;
            slot.ready.store(false, Ordering::SeqCst);
            let mut guard = slot.child.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(mut c) = guard.take() {
                loop {
                    match c.try_wait() {
                        Ok(Some(_)) => break,
                        Ok(None) if std::time::Instant::now() < deadline => std::thread::sleep(Duration::from_millis(25)),
                        _ => { kill_group(&mut c); let _ = c.wait(); break; }
                    }
                }
            }
            let _ = name;
        }
    }

    #[cfg(test)]
    pub fn pid(&self, plugin: &str) -> Option<u32> {
        self.slots.get(plugin)?.child.lock().ok()?.as_ref().map(|c| c.id())
    }
}

/// 플러그인 stdout을 끝까지 읽는다. 응답은 기다리는 호출에, 요청·알림은 이벤트 채널에. EOF면 정리하고 Exited를 보낸다.
fn reader(plugin: String, slot: Arc<Slot>, stdout: ChildStdout, tx: Sender<HostEvent>) {
    let mut r = BufReader::new(stdout);
    let mut line = String::new();
    loop {
        line.clear();
        match r.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let text = line.trim_end();
        if text.is_empty() { continue; }
        match rpc::parse_line(text) {
            Ok(Message::Response { id, result }) => {
                if let Some(p) = slot.pending.lock().unwrap_or_else(|p| p.into_inner()).remove(&id) { let _ = p.send(Ok(result)); }
            }
            Ok(Message::Error { id: Some(id), error }) => {
                if let Some(p) = slot.pending.lock().unwrap_or_else(|p| p.into_inner()).remove(&id) { let _ = p.send(Err(error)); }
            }
            Ok(Message::Error { id: None, .. }) => {}
            Ok(Message::Request { id, method, params }) => {
                let _ = tx.send(HostEvent::Incoming { plugin: plugin.clone(), msg: Incoming::Request { id, method, params } });
            }
            Ok(Message::Notification { method, params }) => {
                let _ = tx.send(HostEvent::Incoming { plugin: plugin.clone(), msg: Incoming::Notification { method, params } });
            }
            Err(e) => {
                // 깨진 줄은 알림으로 올려 상태 줄에 한 번 보인다
                let _ = tx.send(HostEvent::Incoming { plugin: plugin.clone(), msg: Incoming::Notification { method: "bibox/bad-line".into(), params: serde_json::json!({"error": e.message}) } });
            }
        }
    }
    let status = slot.child.lock().unwrap_or_else(|p| p.into_inner()).take().and_then(|mut c| c.wait().ok()).and_then(|s| s.code());
    *slot.stdin.lock().unwrap_or_else(|p| p.into_inner()) = None;
    slot.ready.store(false, Ordering::SeqCst);
    for (_, p) in slot.pending.lock().unwrap_or_else(|p| p.into_inner()).drain() {
        let _ = p.send(Err(RpcError { code: EXITED, message: "plugin exited".into(), data: status.map(Value::from) }));
    }
    if !slot.killed.swap(false, Ordering::SeqCst) {
        let _ = tx.send(HostEvent::Exited { plugin, status });
    }
}

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
    for (k, v) in &env.extra {
        cmd.env(k, v);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::manifest::parse_manifest;
    use serde_json::json;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    fn fixtures() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/plugins")
    }

    fn env() -> PluginEnv {
        PluginEnv { bin: "/bin/true".into(), config_dir: "/tmp".into(), db: "/tmp/db.json".into(), notes: "/tmp/n".into(), pdfs: "/tmp/p".into(), home: None, extra: BTreeMap::new() }
    }

    /// `name`으로 픽스처 `script`를 도는 플러그인 하나짜리 호스트. `events`는 [events] subscribe.
    fn host_for(name: &str, script: &str, events: &[&str]) -> PluginHost {
        static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("bibox-host-{}-{}-{}", name, std::process::id(), seq)).join(name);
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
        std::fs::create_dir_all(&dir).unwrap();
        let subs = events.iter().map(|e| format!("\"{}\"", e)).collect::<Vec<_>>().join(", ");
        let text = format!(
            "api = 2\nname = \"{}\"\nrun = \"python3 {}\"\n[[commands]]\nid = \"go\"\ndesc = \"Go\"\n[[fields]]\nid = \"count\"\nplace = \"row.1\"\n[events]\nsubscribe = [{}]\n",
            name, fixtures().join(script).display(), subs
        );
        let mut problems = vec![];
        let m = parse_manifest(&dir, &text, &mut problems).unwrap();
        assert!(problems.is_empty(), "{:?}", problems);
        PluginHost::new(vec![m], BTreeMap::new(), env())
    }

    fn wait_event(host: &PluginHost, pred: impl Fn(&HostEvent) -> bool) -> Option<HostEvent> {
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            if let Some(ev) = host.try_recv_event() {
                if pred(&ev) { return Some(ev); }
            } else {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        None
    }

    #[test]
    fn initialize_runs_once_and_calls_round_trip_on_the_same_process() {
        let host = host_for("echo", "rpc_echo.py", &[]);
        assert!(!host.is_running("echo"));
        let r = host.call("echo", "commands/run", json!({"command": "go", "trigger": "key", "entry": {"bibtex_key": "kim2025"}}), Some(Duration::from_secs(5))).unwrap();
        assert_eq!(r["message"], "ran go on kim2025");
        let pid = host.pid("echo").expect("running");
        let r = host.call("echo", "fields/get", json!({"keys": ["a", "b"], "entries": []}), Some(Duration::from_secs(5))).unwrap();
        assert_eq!(r["fields"]["b"]["count"]["text"], "1");
        assert_eq!(host.pid("echo"), Some(pid), "one process serves every call");
        assert!(matches!(host.call("echo", "nope/x", json!({}), Some(Duration::from_secs(5))), Err(PluginError::Rpc(e)) if e.code == crate::plugin::rpc::METHOD_NOT_FOUND));
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

    /// 30초 동안 아무 말도 없으면(답도, 팝업 요청도, 진행 알림도) 끊는다. 팝업에서 사용자가 오래 고민해도 안 끊긴다.
    #[test]
    fn a_silent_plugin_times_out_but_a_popup_resets_the_clock() {
        let host = host_for("ask", "rpc_ask.py", &[]);
        let t0 = Instant::now();
        let r = host.call_with_ui_idle("ask", "fields/get", json!({"keys": []}), &mut Picks(vec![]), Duration::from_millis(300));
        assert!(matches!(r, Err(PluginError::Timeout)), "{:?}", r);
        assert!(t0.elapsed() < Duration::from_secs(2));
        assert!(host.is_running("ask"), "a silent plugin is not killed");
        struct SlowPicks;
        impl UiSink for SlowPicks {
            fn ask(&mut self, _plugin: &str, req: UiRequest) -> UiAnswer {
                std::thread::sleep(Duration::from_millis(600));
                match req {
                    UiRequest::Pick { .. } => UiAnswer::Index { index: Some(0) },
                    other => UiAnswer::cancel_for(&other),
                }
            }
        }
        let r = host.call_with_ui_idle("ask", "commands/run", json!({"command": "go", "trigger": "key"}), &mut SlowPicks, Duration::from_millis(300)).unwrap();
        assert_eq!(r["message"], "picked 0", "the user took longer than the idle limit inside the popup");
    }

    /// 명령 중에 플러그인이 보낸 window/pick은 호출자의 UiSink로 답한다(CLI 경로).
    #[test]
    fn a_window_request_during_a_call_is_answered_by_the_sink() {
        let host = host_for("ask", "rpc_ask.py", &[]);
        let r = host.call_with_ui_idle("ask", "commands/run", json!({"command": "go", "trigger": "key"}), &mut Picks(vec![1]), PluginHost::IDLE_LIMIT).unwrap();
        assert_eq!(r["message"], "picked 1");
    }

    #[test]
    fn a_call_that_is_never_answered_times_out_and_the_plugin_survives() {
        let host = host_for("ask", "rpc_ask.py", &[]);
        let t0 = Instant::now();
        assert!(matches!(host.call("ask", "fields/get", json!({"keys": []}), Some(Duration::from_millis(200))), Err(PluginError::Timeout)));
        assert!(t0.elapsed() < Duration::from_secs(2));
        assert!(host.is_running("ask"));
    }

    #[test]
    fn notifications_from_the_plugin_arrive_on_the_event_channel() {
        let host = host_for("notify", "rpc_notify.py", &["library/written"]);
        host.ensure_running("notify").unwrap();
        let ev = wait_event(&host, |e| matches!(e, HostEvent::Incoming { msg: Incoming::Notification { method, .. }, .. } if method == "status/set")).expect("status/set");
        let HostEvent::Incoming { plugin, msg: Incoming::Notification { params, .. } } = ev else { unreachable!() };
        assert_eq!(plugin, "notify");
        assert_eq!(params["text"], "hello");
    }

    /// emit은 구독자에게만, 안 떠 있으면 띄워서 보낸다.
    #[test]
    fn emit_reaches_subscribers_only_and_spawns_lazy_ones() {
        let host = host_for("notify", "rpc_notify.py", &["library/written"]);
        assert_eq!(host.subscribers("library/written"), vec!["notify"]);
        assert!(host.subscribers("note/saved").is_empty());
        assert!(!host.is_running("notify"));
        let outcomes = host.emit("library/written", json!({"reason": "add", "entries": []}));
        assert_eq!(outcomes.len(), 1);
        assert!(outcomes[0].1.is_ok(), "{:?}", outcomes[0].1);
        let ev = wait_event(&host, |e| matches!(e, HostEvent::Incoming { msg: Incoming::Notification { method, .. }, .. } if method == "test/got")).expect("echoed");
        let HostEvent::Incoming { msg: Incoming::Notification { params, .. }, .. } = ev else { unreachable!() };
        assert_eq!(params["method"], "library/written");
        assert!(host.emit("note/saved", json!({})).is_empty(), "no subscriber, nothing sent");
    }

    #[test]
    fn a_plugin_that_dies_fails_the_call_reports_exited_and_is_restarted_next_time() {
        let host = host_for("die", "rpc_die.py", &[]);
        let r = host.call("die", "commands/run", json!({"command": "go", "trigger": "key"}), Some(Duration::from_secs(5)));
        assert!(matches!(r, Err(PluginError::Exited(_))), "{:?}", r);
        let ev = wait_event(&host, |e| matches!(e, HostEvent::Exited { .. })).expect("exited event");
        assert!(matches!(ev, HostEvent::Exited { status: Some(3), .. }), "{:?}", ev);
        assert!(!host.is_running("die"));
        let r = host.call("die", "commands/run", json!({"command": "go", "trigger": "key"}), Some(Duration::from_secs(5)));
        assert!(matches!(r, Err(PluginError::Exited(_))), "restarted and died again: {:?}", r);
    }

    #[test]
    fn shutdown_waits_briefly_for_a_polite_exit_and_kills_the_rest() {
        let host = host_for("echo", "rpc_echo.py", &[]);
        host.ensure_running("echo").unwrap();
        let pid = host.pid("echo").unwrap();
        host.shutdown();
        assert!(host.pid("echo").is_none());
        assert!(!alive(pid));
        let host = host_for("die", "rpc_die.py", &[]);
        host.ensure_running("die").unwrap();
        let pid = host.pid("die").unwrap();
        let t0 = Instant::now();
        host.shutdown();
        assert!(t0.elapsed() < Duration::from_secs(4), "killed after the 2s grace");
        assert!(!alive(pid));
    }

    /// 헬퍼 v2: 명령·팝업·상태 조각·fields/get·이벤트 데코레이터·initialize의 config.
    #[test]
    fn the_python_helper_speaks_v2() {
        let host = host_for("smoke", "helper_smoke.py", &["library/written"]);
        host.update_config_tables(BTreeMap::from([("smoke".to_string(), json!({"model": "m1"}))]));
        let r = host.call_with_ui_idle("smoke", "commands/run", json!({"command": "go", "trigger": "key"}), &mut Picks(vec![0]), PluginHost::IDLE_LIMIT).unwrap();
        assert_eq!(r["message"], "ran go with model m1");
        let ev = wait_event(&host, |e| matches!(e, HostEvent::Incoming { msg: Incoming::Notification { method, .. }, .. } if method == "status/set")).expect("status/set");
        let HostEvent::Incoming { msg: Incoming::Notification { params, .. }, .. } = ev else { unreachable!() };
        assert_eq!(params["text"], "picked 0");
        let r = host.call("smoke", "fields/get", json!({"keys": ["a"], "entries": []}), Some(Duration::from_secs(5))).unwrap();
        assert_eq!(r["fields"]["a"]["count"]["text"], "1");
        host.emit("library/written", json!({"reason": "add", "entries": [{"bibtex_key": "a"}]}));
        let ev = wait_event(&host, |e| matches!(e, HostEvent::Incoming { msg: Incoming::Notification { method, .. }, .. } if method == "fields/set")).expect("fields/set");
        let HostEvent::Incoming { msg: Incoming::Notification { params, .. }, .. } = ev else { unreachable!() };
        assert_eq!(params["fields"]["a"]["count"]["color"], "accent");
    }

    /// citations는 캐시에 있는 DOI는 즉시 답하고 없거나 오래된 것은 빈칸으로 두고 뒤에서 받는다. BIBOX_CITATIONS_OFFLINE=1이면 받지 않는다.
    #[test]
    fn citations_answers_from_cache_without_the_network() {
        let dir = std::env::temp_dir().join(format!("bibox-citations-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let plugin_dir = dir.join("citations");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("plugins/citations");
        for f in ["plugin.toml", "main.py", "bibox_plugin.py", "AGENT.md"] {
            std::fs::copy(src.join(f), plugin_dir.join(f)).unwrap();
        }
        std::fs::copy(fixtures().join("citations_cache.json"), plugin_dir.join("cache.json")).unwrap();
        let (manifests, problems) = crate::plugin::discover(&dir);
        assert!(problems.is_empty(), "{:?}", problems);
        let mut env = env();
        env.extra.insert("BIBOX_CITATIONS_OFFLINE".into(), "1".into());
        let host = PluginHost::new(manifests, BTreeMap::new(), env);
        let entries = json!([
            {"bibtex_key": "fresh", "doi": "10.1000/abc"},
            {"bibtex_key": "stale", "doi": "10.1000/old"},
            {"bibtex_key": "nodoi"},
        ]);
        let r = host.call("citations", "fields/get", json!({"keys": ["fresh", "stale", "nodoi"], "entries": entries}), Some(Duration::from_secs(5))).unwrap();
        assert_eq!(r["fields"]["fresh"]["count"]["text"], "★ 312");
        assert!(r["fields"].get("stale").is_none(), "stale is refetched later, not answered now");
        assert!(r["fields"].get("nodoi").is_none());
    }

    fn alive(pid: u32) -> bool {
        std::process::Command::new("kill").args(["-0", &pid.to_string()]).stderr(std::process::Stdio::null()).status().map(|s| s.success()).unwrap_or(false)
    }
}
