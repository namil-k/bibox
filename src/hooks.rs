use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::Arc;

use crate::config::Config;
use crate::models::Entry;
use crate::plugin::manifest::HookKind;
use crate::plugin::protocol::{validate_apply, Final};
use crate::plugin::{NoUiSink, PluginHost, UiSink};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteReason {
    Add,
    Edit,
    Delete,
    Import,
    Undo,
    Redo,
    Other,
}

impl WriteReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            WriteReason::Add => "add",
            WriteReason::Edit => "edit",
            WriteReason::Delete => "delete",
            WriteReason::Import => "import",
            WriteReason::Undo => "undo",
            WriteReason::Redo => "redo",
            WriteReason::Other => "other",
        }
    }

    /// 기존 auto-commit 메시지와 같은 모양. "bibox: add kim2025", "bibox: import 12 entries".
    pub fn commit_message(&self, keys: &[String]) -> String {
        match (self, keys.len()) {
            (WriteReason::Undo, _) => "bibox: undo".to_string(),
            (WriteReason::Redo, _) => "bibox: redo".to_string(),
            (WriteReason::Other, _) => "bibox: update".to_string(),
            (WriteReason::Import, n) => format!("bibox: import {} entries", n),
            (r, 1) => format!("bibox: {} {}", r.as_str(), keys[0]),
            (r, n) => format!("bibox: {} {} entries", r.as_str(), n),
        }
    }
}

/// 훅 하나의 결과. `source`는 "git" 또는 "<plugin>.<command>".
#[derive(Debug)]
pub struct HookOutcome {
    pub source: String,
    pub result: Result<Final, String>,
}

/// 저장 이벤트를 받아 git auto-commit과 플러그인 훅을 같은 자리에서 발화한다.
/// `Clone`이라 백그라운드 스레드로 들고 갈 수 있다.
#[derive(Clone)]
pub struct HookRunner {
    pub host: Arc<PluginHost>,
    pub git: bool,
    pub db_path: PathBuf,
}

impl HookRunner {
    /// CLI용. 매니페스트 문제는 무시한다(doctor와 TUI 시작 화면이 보여준다).
    pub fn from_config(config: &Config) -> HookRunner {
        // 훅 안에서 불린 bibox는 플러그인 훅을 다시 발화하지 않는다(프로세스 경계를 넘는 무한 루프 방지).
        // 헬퍼가 `$BIBOX_BIN`을 되부를 때 이 변수를 붙인다. git auto-commit은 그대로 한다.
        let host = if std::env::var_os("BIBOX_IN_HOOK").is_some() {
            PluginHost::empty(crate::plugin::PluginEnv::from_config(config))
        } else {
            PluginHost::discover(config).0
        };
        HookRunner {
            host: Arc::new(host),
            git: config.git,
            db_path: crate::config::resolve_db_path(config),
        }
    }

    fn run_hooks(&self, kind: HookKind, entry: Option<Entry>, entries: Vec<Entry>, hook: Value, ui: &mut dyn UiSink) -> Vec<HookOutcome> {
        let trigger = format!("hook:{}", kind.name());
        let mut out = Vec::new();
        for &id in self.host.hooks(kind) {
            let Some(cmd) = self.host.commands().get(id) else { continue };
            let ctx = self.host.context(&cmd.plugin, None, None, entry.clone(), entries.clone(), Some(hook.clone()));
            let result = self.host.invoke(id, &trigger, ctx, ui).map_err(|e| e.to_string());
            out.push(HookOutcome { source: cmd.full_name(), result });
        }
        out
    }

    /// 동기. 앞 플러그인의 `apply`가 다음 플러그인의 입력이다. 실패하면 원래 항목으로 계속.
    pub fn before_add(&self, entry: Entry, ui: &mut dyn UiSink) -> (Entry, Vec<HookOutcome>) {
        let mut current = entry;
        let mut outcomes = Vec::new();
        if self.host.hooks(HookKind::BeforeAdd).is_empty() {
            return (current, outcomes);
        }
        // citekey 유일성은 실제 DB와 대조한다. 파일이 없으면(테스트, 첫 실행) 빈 목록.
        let db_entries: Vec<Entry> = crate::storage::load_db(&self.db_path).map(|d| d.entries).unwrap_or_default();
        for &id in self.host.hooks(HookKind::BeforeAdd) {
            let Some(cmd) = self.host.commands().get(id) else { continue };
            let hook = serde_json::json!({ "reason": "add" });
            let ctx = self.host.context(&cmd.plugin, None, None, Some(current.clone()), vec![current.clone()], Some(hook));
            let result = self.host.invoke(id, "hook:before_add", ctx, ui);
            let outcome = match result {
                Ok(f) => {
                    if f.error.is_none() {
                        if let Some(apply) = &f.apply {
                            match validate_apply(&db_entries, Some(&current), apply) {
                                Ok(mut v) if v.len() == 1 => current = v.remove(0),
                                Ok(_) => {}
                                Err(e) => {
                                    outcomes.push(HookOutcome { source: cmd.full_name(), result: Err(format!("apply rejected: {}", e)) });
                                    continue;
                                }
                            }
                        }
                    }
                    Ok(f)
                }
                Err(e) => Err(e.to_string()),
            };
            outcomes.push(HookOutcome { source: cmd.full_name(), result: outcome });
        }
        (current, outcomes)
    }

    /// 동기. git auto-commit이 먼저, 그다음 플러그인 훅(알파벳순). UI 없음.
    pub fn after_write(&self, reason: WriteReason, entries: Vec<Entry>) -> Vec<HookOutcome> {
        let keys: Vec<String> = entries.iter().map(|e| e.bibtex_key.clone()).collect();
        let mut out = Vec::new();
        if self.git {
            if let Err(w) = crate::git::auto_commit_quiet(&self.db_path, &reason.commit_message(&keys)) {
                out.push(HookOutcome { source: "git".to_string(), result: Err(w) });
            }
        }
        let hook = serde_json::json!({ "reason": reason.as_str(), "keys": keys });
        out.extend(self.run_hooks(HookKind::AfterWrite, entries.first().cloned(), entries, hook, &mut NoUiSink));
        out
    }

    pub fn after_write_background(&self, reason: WriteReason, entries: Vec<Entry>, tx: Sender<HookOutcome>) {
        let me = self.clone();
        std::thread::spawn(move || {
            for o in me.after_write(reason, entries) {
                let _ = tx.send(o);
            }
        });
    }

    pub fn after_note_save(&self, entry: Entry, note_path: PathBuf) -> Vec<HookOutcome> {
        let hook = serde_json::json!({ "note_path": note_path });
        self.run_hooks(HookKind::AfterNoteSave, Some(entry.clone()), vec![entry], hook, &mut NoUiSink)
    }

    pub fn after_note_save_background(&self, entry: Entry, note_path: PathBuf, tx: Sender<HookOutcome>) {
        let me = self.clone();
        std::thread::spawn(move || {
            for o in me.after_note_save(entry, note_path) {
                let _ = tx.send(o);
            }
        });
    }
}

/// CLI에서 after 훅의 `apply`를 반영한다. 훅을 다시 발화하지 않는다(깊이 1).
pub fn apply_from_hook(db_path: &Path, incoming: &[Value]) -> Result<usize, String> {
    let mut db = crate::storage::load_db(db_path).map_err(|e| e.to_string())?;
    let updated = validate_apply(&db.entries, None, incoming)?;
    let n = updated.len();
    for u in updated {
        if let Some(slot) = db.entries.iter_mut().find(|e| e.id == u.id) {
            *slot = u;
        }
    }
    crate::storage::save_db(&db, db_path).map_err(|e| e.to_string())?;
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::EntryType;
    use crate::plugin::manifest::parse_manifest;
    use std::path::Path;

    fn entry(id: &str, key: &str) -> Entry {
        Entry {
            id: id.into(), bibtex_key: key.into(), entry_type: EntryType::Article,
            title: Some("T".into()), author: vec![], year: None, journal: None, volume: None,
            number: None, pages: None, publisher: None, editor: None, edition: None, isbn: None,
            booktitle: None, doi: None, url: None, abstract_text: None, tags: vec![],
            howpublished: None, month: None, note: None, collections: vec![], file_path: None,
            created_at: "2026-01-01 00:00:00".into(), updated_at: None,
        }
    }

    #[test]
    fn commit_messages_match_the_old_auto_commit_texts() {
        assert_eq!(WriteReason::Add.commit_message(&["kim2025".into()]), "bibox: add kim2025");
        assert_eq!(WriteReason::Edit.commit_message(&["kim2025".into()]), "bibox: edit kim2025");
        assert_eq!(WriteReason::Delete.commit_message(&["kim2025".into()]), "bibox: delete kim2025");
        assert_eq!(WriteReason::Import.commit_message(&["a".into(), "b".into(), "c".into()]), "bibox: import 3 entries");
        assert_eq!(WriteReason::Edit.commit_message(&["a".into(), "b".into()]), "bibox: edit 2 entries");
        assert_eq!(WriteReason::Undo.commit_message(&[]), "bibox: undo");
        assert_eq!(WriteReason::Other.commit_message(&[]), "bibox: update");
    }

    fn runner_with(script: &str, hook: &str) -> (HookRunner, PathBuf) {
        static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("bibox-hooks-{}-{}-{}", hook, std::process::id(), seq)).join("h");
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
        std::fs::create_dir_all(&dir).unwrap();
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/plugins");
        let text = format!(
            "api = 1\nname = \"h\"\nrun = \"sh {}\"\n[[commands]]\nid = \"go\"\ndesc = \"Go\"\n[[hooks]]\non = \"{}\"\nrun = \"go\"\n",
            fixtures.join(script).display(), hook
        );
        let mut problems = vec![];
        let m = parse_manifest(&dir, &text, &mut problems).unwrap();
        let env = crate::plugin::PluginEnv {
            bin: "/bin/true".into(), config_dir: dir.clone(), db: dir.join("db.json"),
            notes: dir.join("n"), pdfs: dir.join("p"), home: None,
        };
        let host = Arc::new(crate::plugin::PluginHost::new(vec![m], Default::default(), env));
        (HookRunner { host, git: false, db_path: dir.join("db.json") }, dir)
    }

    #[test]
    fn after_write_runs_every_hook_and_reports_its_final() {
        let (r, _) = runner_with("echo.sh", "after_write");
        let out = r.after_write(WriteReason::Edit, vec![entry("1", "a")]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].source, "h.go");
        assert!(out[0].result.as_ref().unwrap().message.as_ref().unwrap().contains("api=1"));
        r.host.shutdown();
    }

    #[test]
    fn after_write_with_no_hooks_is_empty() {
        let (r, _) = runner_with("echo.sh", "before_add");
        assert!(r.after_write(WriteReason::Edit, vec![]).is_empty());
    }

    #[test]
    fn a_crashing_after_hook_is_reported_not_propagated() {
        let (r, _) = runner_with("crash.sh", "after_write");
        let out = r.after_write(WriteReason::Add, vec![entry("1", "a")]);
        assert!(out[0].result.as_ref().unwrap_err().contains("exited with code 1"));
    }

    #[test]
    fn after_write_in_the_background_delivers_through_the_channel() {
        let (r, _) = runner_with("echo.sh", "after_write");
        let (tx, rx) = std::sync::mpsc::channel();
        r.after_write_background(WriteReason::Edit, vec![entry("1", "a")], tx);
        let o = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        assert_eq!(o.source, "h.go");
        r.host.shutdown();
    }

    #[test]
    fn before_add_returns_the_original_entry_when_the_hook_fails() {
        let (r, _) = runner_with("crash.sh", "before_add");
        let (e, out) = r.before_add(entry("1", "a"), &mut crate::plugin::NoUiSink);
        assert_eq!(e.bibtex_key, "a");
        assert!(out[0].result.is_err());
    }

    #[test]
    fn before_add_takes_a_valid_apply_from_the_hook() {
        let dir = std::env::temp_dir().join(format!("bibox-hooks-apply-{}", std::process::id())).join("h");
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("apply.sh"), "#!/bin/sh\nwhile IFS= read -r line; do\n  e=$(printf '%s' \"$line\" | sed 's/.*\"entry\":\\({[^}]*}\\).*/\\1/' | sed 's/\"title\":\"T\"/\"title\":\"Tidied\"/')\n  printf '{\"apply\":[%s]}\\n' \"$e\"\ndone\n").unwrap();
        let text = "api = 1\nname = \"h\"\nrun = \"sh apply.sh\"\n[[commands]]\nid = \"go\"\ndesc = \"Go\"\n[[hooks]]\non = \"before_add\"\nrun = \"go\"\n";
        let mut problems = vec![];
        let m = parse_manifest(&dir, text, &mut problems).unwrap();
        let env = crate::plugin::PluginEnv { bin: "/bin/true".into(), config_dir: dir.clone(), db: dir.join("db.json"), notes: dir.join("n"), pdfs: dir.join("p"), home: None };
        let host = Arc::new(crate::plugin::PluginHost::new(vec![m], Default::default(), env));
        let r = HookRunner { host, git: false, db_path: dir.join("db.json") };
        let (e, out) = r.before_add(entry("1", "a"), &mut crate::plugin::NoUiSink);
        assert!(out[0].result.is_ok(), "{:?}", out[0].result);
        assert_eq!(e.title.as_deref(), Some("Tidied"));
        assert!(e.updated_at.is_some());
        r.host.shutdown();
    }

    #[test]
    fn apply_from_hook_writes_the_db_without_recursing() {
        let dir = std::env::temp_dir().join(format!("bibox-hooks-db-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("db.json");
        let db = crate::models::Database { entries: vec![entry("1", "a")] };
        crate::storage::save_db(&db, &db_path).unwrap();
        let mut v = serde_json::to_value(entry("1", "a")).unwrap();
        v["title"] = serde_json::json!("From hook");
        assert_eq!(apply_from_hook(&db_path, &[v]).unwrap(), 1);
        let db = crate::storage::load_db(&db_path).unwrap();
        assert_eq!(db.entries[0].title.as_deref(), Some("From hook"));
        assert!(apply_from_hook(&db_path, &[serde_json::json!({"id": "zzz"})]).is_err());
    }

    #[test]
    fn from_config_inside_a_hook_has_no_plugin_hooks_but_keeps_git() {
        std::env::set_var("BIBOX_IN_HOOK", "1");
        let mut config = crate::config::Config::default();
        config.git = true;
        let r = HookRunner::from_config(&config);
        std::env::remove_var("BIBOX_IN_HOOK");
        assert!(r.host.commands().is_empty());
        assert!(r.git);
    }
}
