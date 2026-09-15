//! 라이브러리 이벤트를 구독한 플러그인에게 보낸다. `library/adding`만 요청(항목을 고칠 수 있다), 나머지는 알림.

use std::path::PathBuf;
use std::sync::Arc;

use crate::config::Config;
use crate::models::Entry;
use crate::plugin::protocol::{AddingParams, AddingResult, NoteSavedParams, WrittenParams};
use crate::plugin::{PluginHost, UiSink};

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

}

#[derive(Debug)]
pub struct Outcome {
    pub plugin: String,
    pub result: Result<(), String>,
}

pub struct Events {
    pub host: Arc<PluginHost>,
    pub db_path: PathBuf,
}

impl Events {
    /// CLI용. 매니페스트 문제는 무시한다(doctor와 TUI 시작 화면이 보여준다).
    pub fn from_config(config: &Config) -> Events {
        // 플러그인 안에서 불린 bibox는 외부 플러그인에게 다시 이벤트를 보내지 않는다(프로세스 경계를 넘는 무한 루프 방지). 내장은 그대로.
        let in_hook = std::env::var_os("BIBOX_IN_HOOK").is_some();
        let (host, _) = PluginHost::discover_with(config, in_hook);
        Events { host: Arc::new(host), db_path: crate::config::resolve_db_path(config) }
    }

    pub fn adding(&self, mut entry: Entry, ui: &mut dyn UiSink) -> (Entry, Vec<Outcome>) {
        let mut out = Vec::new();
        for plugin in self.host.subscribers("library/adding") {
            let params = serde_json::to_value(AddingParams { entry: entry.clone() }).unwrap_or_default();
            let result = self.host.call_with_ui(&plugin, "library/adding", params, ui).map_err(|e| e.to_string()).and_then(|v| {
                let r: AddingResult = serde_json::from_value(v).map_err(|e| format!("bad library/adding result: {}", e))?;
                if let Some(e) = r.entry { entry = e; }
                Ok(())
            });
            out.push(Outcome { plugin, result });
        }
        (entry, out)
    }

    pub fn written(&self, reason: WriteReason, entries: Vec<Entry>) -> Vec<Outcome> {
        let params = serde_json::to_value(WrittenParams { reason: reason.as_str().to_string(), entries }).unwrap_or_default();
        self.host.emit("library/written", params).into_iter().map(|(plugin, r)| Outcome { plugin, result: r.map_err(|e| e.to_string()) }).collect()
    }

    pub fn note_saved(&self, entry: Entry, path: PathBuf) -> Vec<Outcome> {
        let params = serde_json::to_value(NoteSavedParams { entry, path }).unwrap_or_default();
        self.host.emit("note/saved", params).into_iter().map(|(plugin, r)| Outcome { plugin, result: r.map_err(|e| e.to_string()) }).collect()
    }

    /// CLI 끝에서: 플러그인이 알림을 다 처리하고 나가길 기다린 뒤 남은 메시지를 sink로.
    pub fn finish_cli(&self, sink: &mut dyn UiSink) {
        self.host.shutdown_with(std::time::Duration::from_secs(60));
        while let Some(ev) = self.host.try_recv_event() {
            self.host.answer_from_sink(ev, sink);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::manifest::parse_manifest;
    use crate::plugin::{NoUiSink, PluginEnv};
    use std::collections::BTreeMap;

    fn fixtures() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/plugins")
    }

    fn events_for(script: &str, subscribe: &str) -> Events {
        static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("bibox-events-{}-{}", std::process::id(), seq)).join("p");
        std::fs::create_dir_all(&dir).unwrap();
        let text = format!("api = 2\nname = \"p\"\nrun = \"python3 {}\"\n[events]\nsubscribe = [\"{}\"]\n", fixtures().join(script).display(), subscribe);
        let mut problems = vec![];
        let m = parse_manifest(&dir, &text, &mut problems).unwrap();
        let env = PluginEnv { bin: "/bin/true".into(), config_dir: "/tmp".into(), db: "/tmp/db.json".into(), notes: "/tmp/n".into(), pdfs: "/tmp/p".into(), home: None, extra: BTreeMap::new() };
        Events { host: Arc::new(PluginHost::new(vec![m], BTreeMap::new(), env)), db_path: "/tmp/db.json".into() }
    }

    fn entry(key: &str) -> Entry {
        use crate::models::EntryType;
        Entry {
            id: "1".into(), bibtex_key: key.into(), entry_type: EntryType::Article,
            title: Some("T".into()), author: vec!["Kim, J.".into()], year: Some(2025),
            journal: None, volume: None, number: None, pages: None, publisher: None, editor: None,
            edition: None, isbn: None, booktitle: None, doi: None, url: None, abstract_text: None,
            tags: vec![], howpublished: None, month: None, note: None, collections: vec![],
            file_path: None, created_at: "2026-01-01 00:00:00".into(), updated_at: None,
        }
    }

    /// rpc_adding.py는 library/adding에 title을 "fixed"로 바꿔 돌려준다.
    #[test]
    fn adding_returns_the_entry_a_subscriber_fixed() {
        let ev = events_for("rpc_adding.py", "library/adding");
        let (e, outcomes) = ev.adding(entry("kim2025"), &mut NoUiSink);
        assert_eq!(e.title.as_deref(), Some("fixed"));
        assert_eq!(outcomes.len(), 1);
        assert!(outcomes[0].result.is_ok());
    }

    #[test]
    fn adding_without_subscribers_returns_the_entry_untouched_and_starts_nothing() {
        let ev = events_for("rpc_adding.py", "library/written");
        let (e, outcomes) = ev.adding(entry("kim2025"), &mut NoUiSink);
        assert_eq!(e.bibtex_key, "kim2025");
        assert!(outcomes.is_empty());
        assert!(!ev.host.is_running("p"));
    }

    #[test]
    fn adding_keeps_the_original_when_the_plugin_dies() {
        let ev = events_for("rpc_die.py", "library/adding");
        let (e, outcomes) = ev.adding(entry("kim2025"), &mut NoUiSink);
        assert_eq!(e.bibtex_key, "kim2025");
        assert!(outcomes[0].result.is_err());
    }

    #[test]
    fn written_notifies_subscribers_and_finish_cli_waits_for_them() {
        let ev = events_for("rpc_notify.py", "library/written");
        let outcomes = ev.written(WriteReason::Add, vec![entry("a")]);
        assert_eq!(outcomes.len(), 1);
        assert!(outcomes[0].result.is_ok());
        ev.finish_cli(&mut NoUiSink);
        assert!(!ev.host.is_running("p"));
    }
}
