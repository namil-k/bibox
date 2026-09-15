//! 플러그인 프로토콜 v2의 params와 result. 메서드 이름과 짝은 스펙 부록 A.
//! 파싱은 rpc.rs가 하고 여기는 모양만 정한다.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::models::Entry;

pub const PROTOCOL: u32 = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Paths {
    pub config_dir: PathBuf,
    pub db: PathBuf,
    pub notes: PathBuf,
    pub pdfs: PathBuf,
    pub home: Option<PathBuf>,
}

// ── bibox → 플러그인 ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Capabilities {
    pub images: bool,
    pub status_bar: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitializeParams {
    pub protocol: u32,
    pub bibox: String,
    pub paths: Paths,
    pub config: Value,
    pub capabilities: Capabilities,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InitializeResult {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub protocol: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandParams {
    pub command: String,
    /// key | menu | cli
    pub trigger: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry: Option<Entry>,
    #[serde(default)]
    pub entries: Vec<Entry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collection: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CommandResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldsGetParams {
    pub keys: Vec<String>,
    pub entries: Vec<Entry>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldValue {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

/// 항목 키 → 필드 id → 값
pub type FieldsMap = BTreeMap<String, BTreeMap<String, FieldValue>>;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FieldsResult {
    #[serde(default)]
    pub fields: FieldsMap,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewParams {
    pub view: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry: Option<Entry>,
    pub page: u32,
    pub width_px: u32,
    pub images: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ViewResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lines: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pages: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddingParams {
    pub entry: Entry,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AddingResult {
    #[serde(default)]
    pub entry: Option<Entry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WrittenParams {
    pub reason: String,
    pub entries: Vec<Entry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteSavedParams {
    pub entry: Entry,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectedParams {
    pub entry: Entry,
}

// ── 플러그인 → bibox ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct StatusSetParams {
    pub field: String,
    pub text: String,
    #[serde(default)]
    pub color: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FieldsSetParams {
    pub fields: FieldsMap,
}

/// `window/pick` `window/prompt` `window/confirm` `window/progress`. 셋은 요청, progress는 알림.
#[derive(Debug, Clone, PartialEq)]
pub enum UiRequest {
    Pick { title: Option<String>, items: Vec<String> },
    Prompt { title: Option<String>, default: Option<String> },
    Confirm { title: Option<String> },
    Progress { text: String },
}

impl UiRequest {
    pub fn from_method(method: &str, params: &Value) -> Option<UiRequest> {
        let title = params.get("title").and_then(Value::as_str).map(str::to_string);
        match method {
            "window/pick" => {
                let items = params.get("items")?.as_array()?.iter().map(|v| v.as_str().map(str::to_string)).collect::<Option<Vec<_>>>()?;
                Some(UiRequest::Pick { title, items })
            }
            "window/prompt" => Some(UiRequest::Prompt { title, default: params.get("default").and_then(Value::as_str).map(str::to_string) }),
            "window/confirm" => Some(UiRequest::Confirm { title }),
            "window/progress" => Some(UiRequest::Progress { text: params.get("text")?.as_str()?.to_string() }),
            _ => None,
        }
    }

}

#[derive(Debug, Clone, PartialEq)]
pub enum UiAnswer {
    Index { index: Option<usize> },
    Text { text: Option<String> },
    Yes { yes: bool },
    Ack {},
}

impl UiAnswer {
    pub fn cancel_for(req: &UiRequest) -> UiAnswer {
        match req {
            UiRequest::Pick { .. } => UiAnswer::Index { index: None },
            UiRequest::Prompt { .. } => UiAnswer::Text { text: None },
            UiRequest::Confirm { .. } => UiAnswer::Yes { yes: false },
            UiRequest::Progress { .. } => UiAnswer::Ack {},
        }
    }

    pub fn to_result(&self) -> Value {
        match self {
            UiAnswer::Index { index } => serde_json::json!({"index": index}),
            UiAnswer::Text { text } => serde_json::json!({"text": text}),
            UiAnswer::Yes { yes } => serde_json::json!({"yes": yes}),
            UiAnswer::Ack {} => serde_json::json!({}),
        }
    }

}

/// 전부 아니면 전무. `db`는 현재 항목들, `pending_new`는 `before_add`에서 추가 중인 항목
/// (아직 `db`에 없다). 통과하면 `updated_at`이 찍힌 항목들을 돌려준다.
pub fn validate_apply(db: &[Entry], pending_new: Option<&Entry>, incoming: &[Value]) -> Result<Vec<Entry>, String> {
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let mut out: Vec<Entry> = Vec::with_capacity(incoming.len());

    for (i, v) in incoming.iter().enumerate() {
        let Some(id) = v.get("id").and_then(Value::as_str) else {
            return Err(format!("entries[{}].id is missing", i));
        };
        let existing = db
            .iter()
            .find(|d| d.id == id)
            .or_else(|| pending_new.filter(|p| p.id == id));
        let Some(existing) = existing else {
            return Err(format!("entries[{}].id not found: {}", i, id));
        };

        let mut e: Entry = match serde_json::from_value(v.clone()) {
            Ok(e) => e,
            Err(err) => {
                return Err(match v.as_object().and_then(|o| locate_bad_field(existing, o)) {
                    Some(field) => format!("entries[{}].{}: {}", i, field, err),
                    None => format!("entries[{}]: {}", i, err),
                });
            }
        };

        if existing.created_at != e.created_at {
            return Err(format!("entries[{}].created_at cannot change", i));
        }

        let key_taken_in_db = db.iter().any(|d| d.id != e.id && d.bibtex_key == e.bibtex_key)
            || pending_new.is_some_and(|p| p.id != e.id && p.bibtex_key == e.bibtex_key);
        let key_taken_in_batch = out.iter().any(|o| o.bibtex_key == e.bibtex_key);
        if key_taken_in_db || key_taken_in_batch {
            return Err(format!("entries[{}].bibtex_key \"{}\" is already used", i, e.bibtex_key));
        }

        e.updated_at = Some(now.clone());
        out.push(e);
    }
    Ok(out)
}

/// serde의 `from_value` 오류에는 필드 이름이 없다. 오류 경로에서만, 기존 항목에 들어온 값을
/// 필드 하나씩 덮어 보며 처음 깨지는 필드를 찾는다. 모르는 키는 `Entry`가 무시하므로 안 걸린다.
fn locate_bad_field(existing: &Entry, incoming: &serde_json::Map<String, Value>) -> Option<String> {
    let base = serde_json::to_value(existing).ok()?;
    for (k, val) in incoming {
        let mut probe = base.clone();
        probe[k] = val.clone();
        if serde_json::from_value::<Entry>(probe).is_err() {
            return Some(k.clone());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 요청 params는 그대로 직렬화되어야 플러그인이 문서대로 읽는다. 선택 필드는 빠진다.
    #[test]
    fn command_params_serialize_with_the_documented_names() {
        let p = CommandParams { command: "copy".into(), trigger: "key".into(), entry: None, entries: vec![], focus: Some("entries".into()), collection: None };
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["command"], "copy");
        assert_eq!(v["trigger"], "key");
        assert_eq!(v["focus"], "entries");
        assert!(v.get("collection").is_none() || v["collection"].is_null());
        let r: CommandResult = serde_json::from_value(json!({"message": "done"})).unwrap();
        assert_eq!(r.message.as_deref(), Some("done"));
        let r: CommandResult = serde_json::from_value(json!({})).unwrap();
        assert!(r.message.is_none());
    }

    #[test]
    fn fields_maps_nest_entry_key_then_field_id() {
        let r: FieldsResult = serde_json::from_value(json!({"fields": {"kim2025": {"count": {"text": "★ 312", "color": "yellow"}, "read": {"text": "✓"}}}})).unwrap();
        let k = &r.fields["kim2025"];
        assert_eq!(k["count"].text, "★ 312");
        assert_eq!(k["count"].color.as_deref(), Some("yellow"));
        assert!(k["read"].color.is_none());
        let empty: FieldsResult = serde_json::from_value(json!({})).unwrap();
        assert!(empty.fields.is_empty());
    }

    #[test]
    fn window_requests_parse_from_method_and_params_and_answers_become_results() {
        let r = UiRequest::from_method("window/pick", &json!({"title": "Style", "items": ["a", "b"]})).unwrap();
        assert!(matches!(r, UiRequest::Pick { ref items, .. } if items.len() == 2));
        assert!(UiRequest::from_method("window/nope", &json!({})).is_none());
        assert!(UiRequest::from_method("window/pick", &json!({"items": "not a list"})).is_none());
        assert_eq!(UiAnswer::Index { index: Some(1) }.to_result(), json!({"index": 1}));
        assert_eq!(UiAnswer::Text { text: None }.to_result(), json!({"text": null}));
        assert_eq!(UiAnswer::cancel_for(&r).to_result(), json!({"index": null}));
    }

    #[test]
    fn view_results_accept_lines_or_an_image() {
        let v: ViewResult = serde_json::from_value(json!({"lines": ["a"], "pages": 3})).unwrap();
        assert_eq!(v.lines.as_deref(), Some(&["a".to_string()][..]));
        assert_eq!(v.pages, Some(3));
        let v: ViewResult = serde_json::from_value(json!({"image": "/tmp/p.jpg"})).unwrap();
        assert_eq!(v.image.as_deref(), Some(std::path::Path::new("/tmp/p.jpg")));
    }

    use crate::models::EntryType;

    fn entry(id: &str, key: &str) -> Entry {
        Entry {
            id: id.into(), bibtex_key: key.into(), entry_type: EntryType::Article,
            title: Some("T".into()), author: vec!["Kim, J.".into()], year: Some(2025),
            journal: None, volume: None, number: None, pages: None, publisher: None, editor: None,
            edition: None, isbn: None, booktitle: None, doi: None, url: None, abstract_text: None,
            tags: vec![], howpublished: None, month: None, note: None, collections: vec![],
            file_path: None, created_at: "2026-01-01 00:00:00".into(), updated_at: None,
        }
    }

    #[test]
    fn apply_accepts_a_changed_entry_and_stamps_updated_at() {
        let db = vec![entry("1", "a"), entry("2", "b")];
        let mut v = serde_json::to_value(entry("1", "a")).unwrap();
        v["title"] = serde_json::json!("New title");
        let out = validate_apply(&db, None, &[v]).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].title.as_deref(), Some("New title"));
        assert!(out[0].updated_at.is_some());
    }
    #[test]
    fn apply_rejects_bad_types_with_the_index_and_field() {
        let db = vec![entry("1", "a")];
        let mut v = serde_json::to_value(entry("1", "a")).unwrap();
        v["year"] = serde_json::json!("twenty");
        let e = validate_apply(&db, None, &[v]).unwrap_err();
        assert!(e.starts_with("entries[0]"), "{}", e);
        assert!(e.contains("year"), "{}", e);
    }
    #[test]
    fn apply_rejects_unknown_ids_and_changed_created_at() {
        let db = vec![entry("1", "a")];
        let e = validate_apply(&db, None, &[serde_json::to_value(entry("9", "z")).unwrap()]).unwrap_err();
        assert!(e.contains("entries[0].id") && e.contains("not found"), "{}", e);

        let mut v = serde_json::to_value(entry("1", "a")).unwrap();
        v["created_at"] = serde_json::json!("1999-01-01 00:00:00");
        let e = validate_apply(&db, None, &[v]).unwrap_err();
        assert!(e.contains("created_at"), "{}", e);
    }
    #[test]
    fn apply_rejects_a_citekey_that_collides_with_another_entry_or_within_the_batch() {
        let db = vec![entry("1", "a"), entry("2", "b")];
        let mut v = serde_json::to_value(entry("1", "a")).unwrap();
        v["bibtex_key"] = serde_json::json!("b");
        let e = validate_apply(&db, None, &[v]).unwrap_err();
        assert!(e.contains("bibtex_key") && e.contains("\"b\""), "{}", e);

        let mut v1 = serde_json::to_value(entry("1", "a")).unwrap();
        v1["bibtex_key"] = serde_json::json!("zz");
        let mut v2 = serde_json::to_value(entry("2", "b")).unwrap();
        v2["bibtex_key"] = serde_json::json!("zz");
        let e = validate_apply(&db, None, &[v1, v2]).unwrap_err();
        assert!(e.contains("entries[1]"), "{}", e);
    }
    #[test]
    fn apply_is_all_or_nothing() {
        let db = vec![entry("1", "a"), entry("2", "b")];
        let good = serde_json::to_value(entry("1", "a")).unwrap();
        let bad = serde_json::to_value(entry("9", "z")).unwrap();
        assert!(validate_apply(&db, None, &[good, bad]).is_err());
    }
    #[test]
    fn before_add_lets_the_pending_entry_through_and_checks_its_key_against_the_db() {
        let db = vec![entry("1", "a")];
        let pending = entry("new", "fresh");
        let ok = validate_apply(&db, Some(&pending), &[serde_json::to_value(&pending).unwrap()]).unwrap();
        assert_eq!(ok[0].id, "new");

        let mut v = serde_json::to_value(&pending).unwrap();
        v["bibtex_key"] = serde_json::json!("a");
        assert!(validate_apply(&db, Some(&pending), &[v]).unwrap_err().contains("bibtex_key"));
    }
}
