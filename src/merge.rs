//! db.json 3방향 병합. git이 `merge.bibox.driver`로 부른다(`bibox merge-db %O %A %B`).
//! 스펙: docs/superpowers/specs/2026-10-10-db-merge-design.md

#![allow(dead_code)] // Task 5에서 merge-db 명령이 쓰면 지운다

use std::collections::HashMap;

use serde_json::{json, Map, Value};

use crate::models::{parse_stamp, Database, Entry, Stamp};

/// 3방향 집합 병합 칸. 나머지는 통째로 한 값(author도 순서가 있어 한 값).
const SET_FIELDS: [&str; 2] = ["tags", "collections"];

pub struct Outcome {
    pub db: Database,
    /// 이번 병합이 `merge_log`에 더한 기록 수
    pub records: usize,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Side {
    Ours,
    Theirs,
}

/// 항목을 맞추는 열쇠. 아주 옛 항목은 id가 비어 있어 키로 맞춘다.
fn ident(e: &Entry) -> String {
    if e.id.is_empty() { format!("key:{}", e.bibtex_key) } else { e.id.clone() }
}

fn fields(e: &Entry) -> Map<String, Value> {
    match serde_json::to_value(e) {
        Ok(Value::Object(m)) => m,
        _ => Map::new(),
    }
}

/// `updated_at`을 뺀 모든 칸이 같은가
fn same(a: &Entry, b: &Entry) -> bool {
    let (mut x, mut y) = (fields(a), fields(b));
    x.remove("updated_at");
    y.remove("updated_at");
    x == y
}

/// 더 늦게 고친 쪽. 둘 다 시간대가 있어야 비교하고, 아니면 ours.
fn newer(o: &Entry, t: &Entry) -> (Side, &'static str) {
    let at = |e: &Entry| e.updated_at.as_deref().map(parse_stamp).unwrap_or(Stamp::Unknown);
    match (at(o), at(t)) {
        (Stamp::Zoned(a), Stamp::Zoned(b)) if b > a => (Side::Theirs, "newer"),
        (Stamp::Zoned(a), Stamp::Zoned(b)) if a > b => (Side::Ours, "newer"),
        _ => (Side::Ours, "ours_on_tie"),
    }
}

/// 한쪽이 뗀 것은 떼고 양쪽이 붙인 것은 모두 붙인다. 순서는 ours 뒤에 theirs가 새로 붙인 것.
fn set_merge(b: Option<&Value>, o: Option<&Value>, t: Option<&Value>) -> Value {
    let list = |v: Option<&Value>| v.and_then(Value::as_array).cloned().unwrap_or_default();
    let (b, o, t) = (list(b), list(o), list(t));
    let mut out: Vec<Value> = o.iter().filter(|x| !b.contains(x) || t.contains(x)).cloned().collect();
    for x in &t {
        if !b.contains(x) && !out.contains(x) {
            out.push(x.clone());
        }
    }
    Value::Array(out)
}

#[allow(clippy::too_many_arguments)]
fn record(now: &str, e: &Entry, field: &str, kept: Value, kept_at: Option<&str>, dropped: Value, dropped_at: Option<&str>, rule: &str) -> Value {
    json!({
        "at": now, "id": e.id, "bibtex_key": e.bibtex_key, "field": field,
        "kept": kept, "kept_updated_at": kept_at, "dropped": dropped, "dropped_updated_at": dropped_at, "rule": rule,
    })
}

/// 같은 항목의 세 판을 칸마다 합친다. base가 없으면(양쪽이 같은 id로 따로 추가) 빈 항목으로 본다.
fn merge_entry(b: Option<&Entry>, o: &Entry, t: &Entry, now: &str, log: &mut Vec<Value>) -> Result<Entry, String> {
    let bm = b.map(fields).unwrap_or_default();
    let (om, tm) = (fields(o), fields(t));
    let (winner, rule) = newer(o, t);
    let mut out = Map::new();
    for k in om.keys() {
        let (bv, ov, tv) = (bm.get(k), om.get(k), tm.get(k));
        let v = match k.as_str() {
            "id" | "updated_at" => ov.cloned(),
            "created_at" => match b {
                Some(_) => bv.cloned(),
                None => {
                    let (os, ts) = (o.created_at.as_str(), t.created_at.as_str());
                    Some(Value::String(if crate::models::cmp_stamps(ts, os).is_lt() { ts } else { os }.to_string()))
                }
            },
            k if SET_FIELDS.contains(&k) => Some(set_merge(bv, ov, tv)),
            _ if ov == tv => ov.cloned(),
            _ if ov == bv => tv.cloned(),
            _ if tv == bv => ov.cloned(),
            _ => {
                let (kv, dv, ka, da) = match winner {
                    Side::Ours => (ov, tv, o.updated_at.as_deref(), t.updated_at.as_deref()),
                    Side::Theirs => (tv, ov, t.updated_at.as_deref(), o.updated_at.as_deref()),
                };
                let null = Value::Null;
                log.push(record(now, o, k, kv.unwrap_or(&null).clone(), ka, dv.unwrap_or(&null).clone(), da, rule));
                kv.cloned()
            }
        };
        if let Some(v) = v {
            out.insert(k.clone(), v);
        }
    }
    let mut merged: Entry = serde_json::from_value(Value::Object(out))
        .map_err(|e| format!("cannot rebuild entry {}: {}", ident(o), e))?;
    merged.updated_at = if same(&merged, o) {
        o.updated_at.clone()
    } else if same(&merged, t) {
        t.updated_at.clone()
    } else {
        match newer(o, t).0 {
            Side::Ours => o.updated_at.clone(),
            Side::Theirs => t.updated_at.clone(),
        }
    };
    Ok(merged)
}

/// 세 db를 합친다. `now`는 기록의 `at`.
pub fn merge(base: &Database, ours: &Database, theirs: &Database, now: &str) -> Result<Outcome, String> {
    // 한쪽에 같은 열쇠가 둘이면 어느 쪽을 버릴지 알 수 없어 병합을 거절한다
    let index = |d: &Database, side: &str| -> Result<HashMap<String, Entry>, String> {
        let mut m = HashMap::new();
        for e in &d.entries {
            if m.insert(ident(e), e.clone()).is_some() {
                return Err(format!("duplicate entry {} in {}", ident(e), side));
            }
        }
        Ok(m)
    };
    let (bmap, omap, tmap) = (index(base, "base")?, index(ours, "ours")?, index(theirs, "theirs")?);
    let mut log: Vec<Value> = Vec::new();

    let mut order: Vec<String> = Vec::new();
    for e in ours.entries.iter().chain(theirs.entries.iter()).chain(base.entries.iter()) {
        let i = ident(e);
        if !order.contains(&i) {
            order.push(i);
        }
    }
    let mut merged: HashMap<String, Entry> = HashMap::new();
    for i in &order {
        let result = match (bmap.get(i), omap.get(i), tmap.get(i)) {
            (None, Some(o), None) => Some(o.clone()),
            (None, None, Some(t)) => Some(t.clone()),
            (None, Some(o), Some(t)) => Some(merge_entry(None, o, t, now, &mut log)?),
            (Some(b), None, Some(t)) | (Some(b), Some(t), None) if same(b, t) => None,
            (Some(_), None, Some(kept)) | (Some(_), Some(kept), None) => {
                log.push(record(now, kept, "entry", Value::String("kept".into()), kept.updated_at.as_deref(), Value::String("deleted".into()), None, "kept_over_delete"));
                Some(kept.clone())
            }
            (Some(b), Some(o), Some(t)) => Some(merge_entry(Some(b), o, t, now, &mut log)?),
            (Some(_), None, None) | (None, None, None) => None,
        };
        if let Some(e) = result {
            merged.insert(i.clone(), e);
        }
    }

    // Task 4에서 theirs 위치 보존과 키 겹침으로 바꾼다. 지금은 ours 순서 뒤에 나머지.
    let entries: Vec<Entry> = order.iter().filter_map(|i| merged.remove(i)).collect();

    let records = log.len();
    let mut merge_log = ours.merge_log.clone();
    merge_log.extend(log);
    Ok(Outcome { db: Database { entries, merge_log }, records })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Database, Entry};

    pub(super) fn e(id: &str, key: &str, title: &str, at: Option<&str>) -> Entry {
        serde_json::from_value(serde_json::json!({
            "id": id, "bibtex_key": key, "entry_type": "article", "title": title,
            "author": ["Kim, J."], "year": 2020, "tags": [], "collections": [],
            "created_at": "2026-01-01 00:00:00", "updated_at": at
        })).unwrap()
    }
    pub(super) fn db(es: Vec<Entry>) -> Database { Database { entries: es, ..Default::default() } }
    fn titles(d: &Database) -> Vec<String> { d.entries.iter().map(|x| x.title.clone().unwrap_or_default()).collect() }
    const NOW: &str = "2026-10-10T12:00:00+02:00";

    #[test]
    fn one_side_adds_the_other_edits_both_survive() {
        let base = db(vec![e("1", "a", "A", None)]);
        let ours = db(vec![e("1", "a", "A2", Some("2026-10-10T10:00:00+02:00"))]);
        let theirs = db(vec![e("1", "a", "A", None), e("2", "b", "B", None)]);
        let out = merge(&base, &ours, &theirs, NOW).unwrap();
        assert_eq!(titles(&out.db), vec!["A2", "B"]);
        assert_eq!(out.records, 0);
    }

    #[test]
    fn different_fields_of_one_entry_both_survive() {
        let base = db(vec![e("1", "a", "A", None)]);
        let mut o = e("1", "a", "A", Some("2026-10-10T10:00:00+02:00"));
        o.tags = vec!["later".into()];
        let mut t = e("1", "a", "A", Some("2026-10-10T08:05:00+00:00"));
        t.year = Some(2021);
        let out = merge(&base, &db(vec![o]), &db(vec![t]), NOW).unwrap();
        let x = &out.db.entries[0];
        assert_eq!((x.tags.clone(), x.year), (vec!["later".to_string()], Some(2021)));
        assert_eq!(x.updated_at.as_deref(), Some("2026-10-10T08:05:00+00:00"), "mixed: the later of the two");
        assert_eq!(out.records, 0);
    }

    #[test]
    fn the_same_field_goes_to_the_newer_edit_and_the_loser_is_logged() {
        let base = db(vec![e("1", "a", "A", None)]);
        let o = e("1", "a", "Deep Learning", Some("2026-10-10T10:00:00+02:00"));
        let t = e("1", "a", "Deep learning.", Some("2026-10-10T08:05:00+00:00"));
        let out = merge(&base, &db(vec![o]), &db(vec![t]), NOW).unwrap();
        assert_eq!(titles(&out.db), vec!["Deep learning."]);
        assert_eq!(out.records, 1);
        let r = &out.db.merge_log[0];
        assert_eq!((r["field"].as_str(), r["rule"].as_str()), (Some("title"), Some("newer")));
        assert_eq!((r["kept"].as_str(), r["dropped"].as_str()), (Some("Deep learning."), Some("Deep Learning")));
        assert_eq!(r["at"], NOW);
    }

    #[test]
    fn without_offsets_ours_wins_the_tie() {
        let base = db(vec![e("1", "a", "A", None)]);
        let o = e("1", "a", "Ours", Some("2026-10-10 10:00:00"));
        let t = e("1", "a", "Theirs", Some("2026-10-10T23:00:00+00:00"));
        let out = merge(&base, &db(vec![o]), &db(vec![t]), NOW).unwrap();
        assert_eq!(titles(&out.db), vec!["Ours"]);
        assert_eq!(out.db.merge_log[0]["rule"], "ours_on_tie");
    }

    #[test]
    fn tags_merge_as_sets_removals_win() {
        let mut b = e("1", "a", "A", None);
        b.tags = vec!["x".into(), "y".into()];
        let mut o = b.clone();
        o.tags = vec!["x".into(), "y".into(), "o".into()]; // ours adds o
        let mut t = b.clone();
        t.tags = vec!["x".into(), "t".into()]; // theirs drops y, adds t
        let out = merge(&db(vec![b]), &db(vec![o]), &db(vec![t]), NOW).unwrap();
        assert_eq!(out.db.entries[0].tags, vec!["x", "o", "t"]);
        assert_eq!(out.records, 0);
    }

    #[test]
    fn a_deletion_beats_no_change_but_not_an_edit() {
        let base = db(vec![e("1", "a", "A", None), e("2", "b", "B", None)]);
        // ours deletes 1 (theirs untouched) and deletes 2 (theirs edited it)
        let ours = db(vec![]);
        let theirs = db(vec![e("1", "a", "A", None), e("2", "b", "B2", Some("2026-10-10T08:00:00+00:00"))]);
        let out = merge(&base, &ours, &theirs, NOW).unwrap();
        assert_eq!(titles(&out.db), vec!["B2"]);
        assert_eq!(out.records, 1);
        assert_eq!(out.db.merge_log[0]["rule"], "kept_over_delete");
    }

    #[test]
    fn both_deleting_or_neither_changing_is_quiet() {
        let base = db(vec![e("1", "a", "A", None)]);
        let out = merge(&base, &db(vec![]), &db(vec![]), NOW).unwrap();
        assert!(out.db.entries.is_empty());
        let out = merge(&base, &base, &base, NOW).unwrap();
        assert_eq!(titles(&out.db), vec!["A"]);
        assert_eq!(out.records, 0);
    }

    #[test]
    fn entries_without_an_id_are_matched_by_key() {
        let base = db(vec![e("", "a", "A", None), e("", "b", "B", None)]);
        let ours = db(vec![e("", "a", "A2", Some("2026-10-10T10:00:00+02:00")), e("", "b", "B", None)]);
        let theirs = db(vec![e("", "a", "A", None), e("", "b", "B3", Some("2026-10-10T10:00:00+02:00"))]);
        let out = merge(&base, &ours, &theirs, NOW).unwrap();
        assert_eq!(titles(&out.db), vec!["A2", "B3"]);
    }

    #[test]
    fn a_duplicate_identity_on_one_side_is_refused() {
        let base = db(vec![e("1", "a", "A", None)]);
        let ours = db(vec![e("1", "a", "A", None), e("1", "a2", "A2", None)]);
        let err = merge(&base, &ours, &base, NOW).err().expect("must refuse");
        assert!(err.contains("duplicate"), "{err}");
    }
}
