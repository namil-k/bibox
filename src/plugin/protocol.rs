use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

use crate::models::Entry;

// ── bibox -> 플러그인 ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Paths {
    pub config_dir: PathBuf,
    pub db: PathBuf,
    pub notes: PathBuf,
    pub pdfs: PathBuf,
    pub home: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Context {
    pub focus: Option<String>,
    pub collection: Option<String>,
    pub entry: Option<Entry>,
    pub entries: Vec<Entry>,
    pub config: Value,
    pub paths: Paths,
    pub hook: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub r#type: String,
    pub id: String,
    pub trigger: String,
    pub context: Context,
}

// ── 플러그인 -> bibox ────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "ui", rename_all = "lowercase")]
pub enum UiRequest {
    Pick {
        #[serde(default)]
        title: Option<String>,
        items: Vec<String>,
    },
    Prompt {
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        default: Option<String>,
    },
    Confirm {
        #[serde(default)]
        title: Option<String>,
    },
    Progress {
        text: String,
    },
}

/// bibox -> 플러그인 답. `untagged`라 각 변형이 `{"index":..}` 같은 평범한 객체로 나간다.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum UiAnswer {
    Index { index: Option<usize> },
    Text { text: Option<String> },
    Yes { yes: bool },
    Ack {},
}

impl UiAnswer {
    /// UI를 못 띄우는 상황(백그라운드 훅, 비대화형 stdin, 사용자 취소)의 답.
    pub fn cancel_for(req: &UiRequest) -> UiAnswer {
        match req {
            UiRequest::Pick { .. } => UiAnswer::Index { index: None },
            UiRequest::Prompt { .. } => UiAnswer::Text { text: None },
            UiRequest::Confirm { .. } => UiAnswer::Yes { yes: false },
            UiRequest::Progress { .. } => UiAnswer::Ack {},
        }
    }
}

/// 최종 응답. 모든 필드가 선택이고 `{}`도 유효하다. 모르는 필드는 무시한다
/// (api 2 플러그인이 보내는 필드가 api 1 bibox를 깨지 않도록).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Final {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub apply: Option<Vec<Value>>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub refresh: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PluginMsg {
    Ui(UiRequest),
    Final(Final),
}

/// 한 줄을 읽는다. 오류 문자열은 그대로 사용자에게 보이므로 원인을 담는다.
pub fn parse_plugin_line(line: &str) -> Result<PluginMsg, String> {
    let v: Value = serde_json::from_str(line).map_err(|e| format!("not json ({}): {}", e, head(line)))?;
    let Some(obj) = v.as_object() else {
        return Err(format!("expected a JSON object, got: {}", head(line)));
    };
    if obj.contains_key("ui") {
        let req: UiRequest = serde_json::from_value(v.clone())
            .map_err(|e| format!("bad ui request ({}): {}", e, head(line)))?;
        Ok(PluginMsg::Ui(req))
    } else {
        let f: Final = serde_json::from_value(v).map_err(|e| format!("bad final response ({}): {}", e, head(line)))?;
        Ok(PluginMsg::Final(f))
    }
}

/// 진단에 넣을 앞 80자. 트레이스백이 통째로 화면에 뜨지 않게 한다.
pub fn head(s: &str) -> String {
    let s = s.trim_end();
    if s.chars().count() <= 80 {
        s.to_string()
    } else {
        format!("{}...", s.chars().take(80).collect::<String>())
    }
}

// ── apply 검증 ───────────────────────────────────────────────────────────────

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
    fn a_request_serializes_with_type_id_trigger_and_context() {
        let req = Request {
            r#type: "command".into(),
            id: "tidy".into(),
            trigger: "key".into(),
            context: Context {
                focus: Some("entries".into()), collection: None, entry: Some(entry("1", "a")),
                entries: vec![entry("1", "a")], config: serde_json::json!({"model": "x"}),
                paths: Paths { config_dir: "/c".into(), db: "/c/db.json".into(), notes: "/n".into(), pdfs: "/p".into(), home: None },
                hook: None,
            },
        };
        let v: serde_json::Value = serde_json::to_value(&req).unwrap();
        assert_eq!(v["type"], "command");
        assert_eq!(v["id"], "tidy");
        assert_eq!(v["trigger"], "key");
        assert_eq!(v["context"]["focus"], "entries");
        assert_eq!(v["context"]["entry"]["bibtex_key"], "a");
        assert_eq!(v["context"]["entries"].as_array().unwrap().len(), 1);
        assert_eq!(v["context"]["config"]["model"], "x");
        assert_eq!(v["context"]["paths"]["db"], "/c/db.json");
        assert!(v["context"]["paths"]["home"].is_null());
        assert!(v["context"]["hook"].is_null());
        assert!(!serde_json::to_string(&req).unwrap().contains('\n'));
    }

    #[test]
    fn ui_lines_parse_into_the_four_requests() {
        assert_eq!(
            parse_plugin_line(r#"{"ui":"pick","title":"Style","items":["APA","IEEE"]}"#).unwrap(),
            PluginMsg::Ui(UiRequest::Pick { title: Some("Style".into()), items: vec!["APA".into(), "IEEE".into()] })
        );
        assert_eq!(
            parse_plugin_line(r#"{"ui":"prompt","default":"x"}"#).unwrap(),
            PluginMsg::Ui(UiRequest::Prompt { title: None, default: Some("x".into()) })
        );
        assert_eq!(
            parse_plugin_line(r#"{"ui":"confirm","title":"Sure?"}"#).unwrap(),
            PluginMsg::Ui(UiRequest::Confirm { title: Some("Sure?".into()) })
        );
        assert_eq!(
            parse_plugin_line(r#"{"ui":"progress","text":"3/12"}"#).unwrap(),
            PluginMsg::Ui(UiRequest::Progress { text: "3/12".into() })
        );
    }

    #[test]
    fn a_line_without_ui_is_a_final_and_empty_object_is_valid() {
        assert_eq!(parse_plugin_line("{}").unwrap(), PluginMsg::Final(Final::default()));
        let f = parse_plugin_line(r#"{"message":"done","refresh":true,"apply":[{"id":"1"}]}"#).unwrap();
        match f {
            PluginMsg::Final(f) => {
                assert_eq!(f.message.as_deref(), Some("done"));
                assert!(f.refresh);
                assert_eq!(f.apply.unwrap().len(), 1);
                assert!(f.error.is_none());
            }
            _ => panic!("expected final"),
        }
    }

    #[test]
    fn garbage_and_unknown_ui_kinds_are_protocol_errors() {
        assert!(parse_plugin_line("not json").unwrap_err().contains("not json"));
        assert!(parse_plugin_line("[1,2]").unwrap_err().contains("object"));
        assert!(parse_plugin_line(r#"{"ui":"table","rows":[]}"#).unwrap_err().contains("table"));
    }

    #[test]
    fn answers_serialize_to_the_documented_shapes() {
        assert_eq!(serde_json::to_string(&UiAnswer::Index { index: Some(2) }).unwrap(), r#"{"index":2}"#);
        assert_eq!(serde_json::to_string(&UiAnswer::Index { index: None }).unwrap(), r#"{"index":null}"#);
        assert_eq!(serde_json::to_string(&UiAnswer::Text { text: None }).unwrap(), r#"{"text":null}"#);
        assert_eq!(serde_json::to_string(&UiAnswer::Yes { yes: true }).unwrap(), r#"{"yes":true}"#);
        assert_eq!(serde_json::to_string(&UiAnswer::Ack {}).unwrap(), "{}");
    }

    #[test]
    fn cancel_for_matches_the_request_kind() {
        assert_eq!(UiAnswer::cancel_for(&UiRequest::Pick { title: None, items: vec![] }), UiAnswer::Index { index: None });
        assert_eq!(UiAnswer::cancel_for(&UiRequest::Prompt { title: None, default: None }), UiAnswer::Text { text: None });
        assert_eq!(UiAnswer::cancel_for(&UiRequest::Confirm { title: None }), UiAnswer::Yes { yes: false });
        assert_eq!(UiAnswer::cancel_for(&UiRequest::Progress { text: String::new() }), UiAnswer::Ack {});
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
    #[test]
    fn a_request_round_trips_through_json() {
        let req = Request {
            r#type: "command".into(),
            id: "sync".into(),
            trigger: "hook:after_write".into(),
            context: Context {
                focus: None, collection: None, entry: Some(entry("1", "a")), entries: vec![entry("1", "a")],
                config: serde_json::json!({"include_pdfs": true}),
                paths: Paths { config_dir: "/c".into(), db: "/h/db.json".into(), notes: "/h/notes".into(), pdfs: "/h/pdfs".into(), home: Some("/h".into()) },
                hook: Some(serde_json::json!({"reason": "add", "keys": ["a"]})),
            },
        };
        let line = serde_json::to_string(&req).unwrap();
        let back: Request = serde_json::from_str(&line).unwrap();
        assert_eq!(back.r#type, "command");
        assert_eq!(back.id, "sync");
        assert_eq!(back.context.paths.home.as_deref(), Some(std::path::Path::new("/h")));
        assert_eq!(back.context.hook.unwrap()["reason"], "add");
        assert_eq!(back.context.entries[0].bibtex_key, "a");
    }

    #[test]
    fn a_final_serializes_without_empty_fields() {
        assert_eq!(serde_json::to_string(&Final::default()).unwrap(), "{}");
        let f = Final { message: Some("ok".into()), refresh: true, ..Default::default() };
        assert_eq!(serde_json::to_string(&f).unwrap(), r#"{"message":"ok","refresh":true}"#);
        let f = Final { error: Some("bad".into()), ..Default::default() };
        assert_eq!(serde_json::to_string(&f).unwrap(), r#"{"error":"bad"}"#);
    }

    #[test]
    fn a_ui_request_serializes_with_the_ui_tag() {
        let r = UiRequest::Progress { text: "pulling".into() };
        assert_eq!(serde_json::to_string(&r).unwrap(), r#"{"ui":"progress","text":"pulling"}"#);
        let r = UiRequest::Pick { title: Some("Style".into()), items: vec!["APA".into()] };
        assert_eq!(serde_json::to_string(&r).unwrap(), r#"{"ui":"pick","title":"Style","items":["APA"]}"#);
    }
}
