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

    // ours 순서를 지키고, theirs에만 있는 항목은 theirs에서 바로 앞 항목의 뒤에(없으면 끝에)
    let mut placed: Vec<String> = ours.entries.iter().map(ident).filter(|i| merged.contains_key(i)).collect();
    let mut prev: Option<String> = None;
    for e in &theirs.entries {
        let i = ident(e);
        if !merged.contains_key(&i) {
            continue;
        }
        if !placed.contains(&i) {
            let at = prev.as_ref().and_then(|p| placed.iter().position(|x| x == p)).map(|p| p + 1).unwrap_or(placed.len());
            placed.insert(at, i.clone());
        }
        prev = Some(i);
    }
    for i in &order {
        if merged.contains_key(i) && !placed.contains(i) {
            placed.push(i.clone());
        }
    }
    let mut entries: Vec<Entry> = placed.iter().filter_map(|i| merged.remove(i)).collect();

    // 키 겹침: ours에서 그 키를 쓰던 항목이 지키고, 나머지는 접미사
    let ours_key: HashMap<String, String> = ours.entries.iter().map(|e| (ident(e), e.bibtex_key.clone())).collect();
    let mut taken: Vec<String> = entries.iter().map(|e| e.bibtex_key.clone()).collect();
    for k in 0..entries.len() {
        let key = entries[k].bibtex_key.clone();
        let holders: Vec<usize> = (0..entries.len()).filter(|&j| entries[j].bibtex_key == key).collect();
        if holders.len() < 2 {
            continue;
        }
        let keeper = holders.iter().copied().find(|&j| ours_key.get(&ident(&entries[j])) == Some(&key)).unwrap_or(holders[0]);
        if k == keeper {
            continue;
        }
        let refs: Vec<&str> = taken.iter().map(String::as_str).collect();
        let new_key = crate::storage::generate_unique_key_excluding(&refs, &key, "");
        taken.push(new_key.clone());
        entries[k].bibtex_key = new_key.clone();
        // 기록은 이름이 바뀐 항목의 새 키로 남긴다
        log.push(record(now, &entries[k], "bibtex_key", Value::String(new_key), entries[k].updated_at.as_deref(), Value::String(key.clone()), None, "key_taken"));
    }

    let records = log.len();
    let to_value = |v: &[Value]| Value::Array(v.to_vec());
    let mut merge_log: Vec<Value> = match set_merge(Some(&to_value(&base.merge_log)), Some(&to_value(&ours.merge_log)), Some(&to_value(&theirs.merge_log))) {
        Value::Array(a) => a,
        _ => Vec::new(),
    };
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

    #[test]
    fn theirs_only_entries_land_after_their_neighbour() {
        let base = db(vec![e("1", "a", "A", None), e("3", "c", "C", None)]);
        let ours = db(vec![e("1", "a", "A", None), e("3", "c", "C", None), e("4", "d", "D", None)]);
        let theirs = db(vec![e("1", "a", "A", None), e("2", "b", "B", None), e("3", "c", "C", None)]);
        let out = merge(&base, &ours, &theirs, NOW).unwrap();
        assert_eq!(titles(&out.db), vec!["A", "B", "C", "D"]);
    }

    #[test]
    fn a_taken_key_gets_a_suffix_on_the_theirs_side() {
        let base = db(vec![]);
        let ours = db(vec![e("1", "kim2025", "Ours paper", None)]);
        let theirs = db(vec![e("2", "kim2025", "Theirs paper", None)]);
        let out = merge(&base, &ours, &theirs, NOW).unwrap();
        let keys: Vec<&str> = out.db.entries.iter().map(|x| x.bibtex_key.as_str()).collect();
        assert_eq!(keys, vec!["kim2025", "kim2025a"]);
        let r = &out.db.merge_log[0];
        assert_eq!((r["rule"].as_str(), r["kept"].as_str(), r["dropped"].as_str()), (Some("key_taken"), Some("kim2025a"), Some("kim2025")));
    }

    #[test]
    fn the_ours_side_holder_keeps_a_taken_key_even_when_placed_later() {
        let base = db(vec![e("1", "a", "A", None)]);
        let ours = db(vec![e("1", "a", "A", None), e("3", "kim2025", "Y", None)]);
        let theirs = db(vec![e("1", "a", "A", None), e("2", "kim2025", "Z", None)]);
        let out = merge(&base, &ours, &theirs, NOW).unwrap();
        assert_eq!(titles(&out.db), vec!["A", "Z", "Y"]);
        let keys: Vec<&str> = out.db.entries.iter().map(|x| x.bibtex_key.as_str()).collect();
        assert_eq!(keys, vec!["a", "kim2025a", "kim2025"]);
        assert_eq!(out.records, 1);
        let r = &out.db.merge_log[0];
        assert_eq!((r["id"].as_str(), r["bibtex_key"].as_str()), (Some("2"), Some("kim2025a")));
        assert_eq!((r["rule"].as_str(), r["kept"].as_str(), r["dropped"].as_str()), (Some("key_taken"), Some("kim2025a"), Some("kim2025")));
    }

    #[test]
    fn two_theirs_side_holders_get_the_next_suffixes() {
        let base = db(vec![e("1", "a", "A", None)]);
        let ours = db(vec![e("1", "a", "A", None), e("3", "kim2025", "Y", None)]);
        let theirs = db(vec![e("1", "a", "A", None), e("2", "kim2025", "Z", None), e("4", "kim2025", "W", None)]);
        let out = merge(&base, &ours, &theirs, NOW).unwrap();
        assert_eq!(titles(&out.db), vec!["A", "Z", "W", "Y"]);
        let keys: Vec<&str> = out.db.entries.iter().map(|x| x.bibtex_key.as_str()).collect();
        assert_eq!(keys, vec!["a", "kim2025a", "kim2025b", "kim2025"]);
        let logged: Vec<(&str, &str)> = out.db.merge_log.iter().map(|r| (r["id"].as_str().unwrap(), r["bibtex_key"].as_str().unwrap())).collect();
        assert_eq!(logged, vec![("2", "kim2025a"), ("4", "kim2025b")]);
    }

    #[test]
    fn merge_logs_from_both_sides_are_kept_once() {
        let shared = serde_json::json!({"field": "title", "rule": "newer", "at": "2026-10-01T00:00:00+00:00"});
        let mine = serde_json::json!({"field": "year", "rule": "newer", "at": "2026-10-02T00:00:00+00:00"});
        let yours = serde_json::json!({"field": "doi", "rule": "newer", "at": "2026-10-03T00:00:00+00:00"});
        let mut base = db(vec![]);
        base.merge_log = vec![shared.clone()];
        let mut ours = db(vec![]);
        ours.merge_log = vec![shared.clone(), mine.clone()];
        let mut theirs = db(vec![]);
        theirs.merge_log = vec![shared.clone(), yours.clone()];
        let out = merge(&base, &ours, &theirs, NOW).unwrap();
        assert_eq!(out.db.merge_log, vec![shared, mine, yours]);
        assert_eq!(out.records, 0);
    }

    #[test]
    fn a_log_cleared_on_one_side_stays_cleared() {
        let shared = serde_json::json!({"field": "title", "rule": "newer"});
        let mut base = db(vec![]);
        base.merge_log = vec![shared.clone()];
        let ours = db(vec![]); // doctor --fix cleared it here
        let mut theirs = db(vec![]);
        theirs.merge_log = vec![shared];
        assert!(merge(&base, &ours, &theirs, NOW).unwrap().db.merge_log.is_empty());
    }
}
