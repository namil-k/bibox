//! db.json 3방향 병합. git이 `merge.bibox.driver`로 부른다(`bibox merge-db %O %A %B`).
//! 스펙: docs/superpowers/specs/2026-10-10-db-merge-design.md

use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

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
        // 빈 키는 겹쳐도 바꿀 이름이 없다. 병합마다 헛기록이 쌓이지 않게 건너뛴다
        if key.is_empty() {
            continue;
        }
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

/// driver 입력은 엄격하게 읽는다. 깨진 항목을 건너뛰면 결과에서 그 논문이 사라진다.
fn read_strict(p: &Path) -> Result<Database, String> {
    let s = std::fs::read_to_string(p).map_err(|e| format!("cannot read {}: {}", p.display(), e))?;
    if s.trim().is_empty() {
        return Ok(Database::default());
    }
    serde_json::from_str::<Database>(&s).map_err(|e| format!("{} is not a bibox library: {}", p.display(), e))
}

/// git이 부르는 진입점(`merge-db %O %A %B`). 결과는 `ours`에 쓴다. 0이면 병합됨, 1이면 git이 충돌로 처리한다.
pub fn run_driver(base: &Path, ours: &Path, theirs: &Path) -> i32 {
    match merge_files(base, ours, theirs) {
        Ok(records) => {
            if records > 0 {
                eprintln!("bibox merge-db: {} field(s) kept by rule", records);
            }
            0
        }
        Err(e) => {
            eprintln!("bibox merge-db: {}", e);
            text_merge(base, ours, theirs);
            1
        }
    }
}

fn merge_files(base: &Path, ours: &Path, theirs: &Path) -> Result<usize, String> {
    let (b, o, t) = (read_strict(base)?, read_strict(ours)?, read_strict(theirs)?);
    let out = merge(&b, &o, &t, &crate::models::now_stamp())?;
    crate::storage::save_db(&out.db, ours).map_err(|e| format!("cannot write {}: {}", ours.display(), e))?;
    Ok(out.records)
}

/// 실패하면 도우미가 없는 기계와 똑같이 보이게 git의 줄 단위 병합(충돌 표시 포함)을 `ours`에 쓴다.
/// ours만 남은 멀쩡한 db.json을 그대로 커밋하는 일을 막는다. 종료 코드는 충돌 수라 보지 않는다.
fn text_merge(base: &Path, ours: &Path, theirs: &Path) {
    let _ = Command::new("git")
        .args(["merge-file", "-L", "ours", "-L", "base", "-L", "theirs"])
        .arg(ours)
        .arg(base)
        .arg(theirs)
        .output();
}

pub const ATTR_LINE: &str = "db.json merge=bibox";

/// git이 `sh -c`로 돌리는 명령. 경로는 작은따옴표로 감싼다.
pub fn driver_command(exe: &Path) -> String {
    format!("'{}' merge-db %O %A %B", exe.display().to_string().replace('\'', r"'\''"))
}

fn git_config_get(home: &Path, key: &str) -> Option<String> {
    let o = Command::new("git").arg("-C").arg(home).args(["config", "--local", "--get", key]).output().ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

fn git_config_set(home: &Path, key: &str, value: &str) -> Result<(), String> {
    let o = Command::new("git").arg("-C").arg(home).args(["config", "--local", key, value]).output().map_err(|e| e.to_string())?;
    if o.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&o.stderr).trim().to_string()) }
}

fn has_attr_line(home: &Path) -> bool {
    std::fs::read_to_string(home.join(".gitattributes")).map(|s| s.lines().any(|l| l.trim() == ATTR_LINE)).unwrap_or(false)
}

/// 빠진 것을 말한다. 없으면 None.
pub fn registration_problem(home: &Path, exe: &Path) -> Option<String> {
    if !has_attr_line(home) {
        return Some(format!(".gitattributes has no `{}`", ATTR_LINE));
    }
    if git_config_get(home, "merge.bibox.driver").as_deref() != Some(driver_command(exe).as_str()) {
        return Some("this machine's git config does not run bibox to merge db.json".to_string());
    }
    None
}

/// `.gitattributes` 줄과 이 기계의 git 설정을 맞춘다. 바꾼 게 있으면 true.
pub fn ensure_registered(home: &Path, exe: &Path) -> Result<bool, String> {
    let mut changed = false;
    if !has_attr_line(home) {
        let path = home.join(".gitattributes");
        let mut s = std::fs::read_to_string(&path).unwrap_or_default();
        if !s.is_empty() && !s.ends_with('\n') {
            s.push('\n');
        }
        s.push_str(ATTR_LINE);
        s.push('\n');
        std::fs::write(&path, s).map_err(|e| format!("cannot write {}: {}", path.display(), e))?;
        changed = true;
    }
    let cmd = driver_command(exe);
    if git_config_get(home, "merge.bibox.driver").as_deref() != Some(cmd.as_str()) {
        git_config_set(home, "merge.bibox.name", "bibox entry-level merge")?;
        git_config_set(home, "merge.bibox.driver", &cmd)?;
        changed = true;
    }
    Ok(changed)
}

pub struct Finding {
    pub kind: &'static str,
    pub key: Option<String>,
    pub detail: String,
}

fn shown(v: &Value) -> String {
    match v {
        Value::String(s) => format!("\"{}\"", s),
        Value::Null => "nothing".to_string(),
        other => other.to_string(),
    }
}

/// 기록 하나를 doctor의 한 줄로.
pub fn describe(r: &Value) -> String {
    let s = |k: &str| r.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let at = |k: &str| r.get(k).and_then(Value::as_str).map(|t| format!(" ({})", crate::models::display_stamp(t))).unwrap_or_default();
    match s("rule").as_str() {
        "key_taken" => format!(
            "renamed to {new} because {old} was taken; if notes/{old}.md is this paper's note, rename it to {new}.md",
            new = s("kept"), old = s("dropped")
        ),
        "kept_over_delete" => "kept: one side deleted it while the other edited it".to_string(),
        _ => format!(
            "{} kept {}{}, dropped {}{}",
            s("field"), shown(r.get("kept").unwrap_or(&Value::Null)), at("kept_updated_at"),
            shown(r.get("dropped").unwrap_or(&Value::Null)), at("dropped_updated_at")
        ),
    }
}

/// doctor 결과. 홈이 git 저장소 루트가 아니면 등록 검사는 건너뛴다.
pub fn doctor_findings(home: Option<&Path>, db: &Database, exe: &Path) -> Vec<Finding> {
    let mut out = Vec::new();
    if let Some(home) = home.filter(|h| h.join(".git").exists()) {
        let problem = if !exe.exists() {
            Some(format!("{} does not exist", exe.display()))
        } else {
            registration_problem(home, exe)
        };
        if let Some(p) = problem {
            out.push(Finding { kind: "merge_driver", key: None, detail: p });
        }
    }
    for r in &db.merge_log {
        out.push(Finding { kind: "merge_log", key: r.get("bibtex_key").and_then(Value::as_str).map(str::to_string), detail: describe(r) });
    }
    out
}

#[cfg(test)]
mod tests {

    #[test]
    fn records_read_as_one_line_each() {
        let r = serde_json::json!({"field": "title", "kept": "Deep learning.", "kept_updated_at": "2026-04-10 13:17:02",
            "dropped": "Deep Learning", "dropped_updated_at": null, "rule": "newer", "bibtex_key": "lee2015deep"});
        assert_eq!(describe(&r), r#"title kept "Deep learning." (2026-04-10 13:17), dropped "Deep Learning""#);
        let k = serde_json::json!({"field": "bibtex_key", "kept": "kim2025a", "dropped": "kim2025", "rule": "key_taken"});
        assert_eq!(describe(&k), "renamed to kim2025a because kim2025 was taken; if notes/kim2025.md is this paper's note, rename it to kim2025a.md");
        let d = serde_json::json!({"field": "entry", "rule": "kept_over_delete", "bibtex_key": "x"});
        assert_eq!(describe(&d), "kept: one side deleted it while the other edited it");
    }

    #[test]
    fn doctor_lists_the_log_and_a_missing_registration() {
        let home = git_repo("doctor");
        let mut d = db(vec![]);
        d.merge_log.push(serde_json::json!({"field": "year", "kept": 2021, "dropped": 2020, "rule": "newer", "bibtex_key": "kim2020a"}));
        let exe = std::env::current_exe().unwrap();
        let f = doctor_findings(Some(&home), &d, &exe);
        let kinds: Vec<&str> = f.iter().map(|x| x.kind).collect();
        assert_eq!(kinds, vec!["merge_driver", "merge_log"]);
        assert_eq!(f[1].key.as_deref(), Some("kim2020a"));
        ensure_registered(&home, &exe).unwrap();
        assert_eq!(doctor_findings(Some(&home), &db(vec![]), &exe).len(), 0);
        assert_eq!(doctor_findings(None, &db(vec![]), &exe).len(), 0, "no home: nothing to register");
    }
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
    fn empty_keys_are_not_a_collision() {
        let base = db(vec![]);
        let ours = db(vec![e("1", "", "A", None)]);
        let theirs = db(vec![e("2", "", "B", None)]);
        let out = merge(&base, &ours, &theirs, NOW).unwrap();
        assert_eq!(titles(&out.db), vec!["A", "B"]);
        assert!(out.db.entries.iter().all(|x| x.bibtex_key.is_empty()));
        assert_eq!(out.records, 0);
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

    fn scratch(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("bibox-merge-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// 도우미가 실패하면 git의 줄 단위 병합 결과가 ours에 남는다. 겹치지 않으면 깨끗한 텍스트 병합이다.
    #[test]
    fn a_malformed_input_is_refused_and_ours_gets_git_text_merge() {
        let d = scratch("bad");
        let good = serde_json::to_string(&db(vec![e("1", "a", "A", None)])).unwrap();
        // 항목 하나가 Entry가 아니다(author 없음)
        let bad = r#"{"entries":[{"id":"2","bibtex_key":"b"}]}"#;
        std::fs::write(d.join("base"), format!("{}\n", good)).unwrap();
        std::fs::write(d.join("ours"), format!("{}\n", good)).unwrap();
        std::fs::write(d.join("theirs"), format!("{}\n", bad)).unwrap();
        assert_eq!(run_driver(&d.join("base"), &d.join("ours"), &d.join("theirs")), 1);
        // ours는 base 그대로라 git은 theirs 쪽을 받는다. bibox가 고쳐 쓴 ours만의 결과가 아니다
        assert_eq!(std::fs::read_to_string(d.join("ours")).unwrap(), format!("{}\n", bad));
    }

    #[test]
    fn a_duplicate_id_is_refused_and_ours_gets_conflict_markers() {
        let d = scratch("dup");
        let good = serde_json::to_string(&db(vec![e("1", "a", "A", None)])).unwrap();
        let dup = serde_json::to_string(&db(vec![e("1", "a", "A", None), e("1", "b", "B", None)])).unwrap();
        let edited = serde_json::to_string(&db(vec![e("1", "a", "A edited", None)])).unwrap();
        std::fs::write(d.join("base"), format!("{}\n", good)).unwrap();
        std::fs::write(d.join("ours"), format!("{}\n", dup)).unwrap();
        std::fs::write(d.join("theirs"), format!("{}\n", edited)).unwrap();
        assert_eq!(run_driver(&d.join("base"), &d.join("ours"), &d.join("theirs")), 1);
        let out = std::fs::read_to_string(d.join("ours")).unwrap();
        assert!(out.starts_with("<<<<<<< ours\n") && out.contains(&dup) && out.contains(&edited) && out.contains(">>>>>>> theirs"), "{}", out);
    }

    #[test]
    fn an_empty_base_with_a_bad_side_still_gets_conflict_markers() {
        let d = scratch("emptybad");
        let good = serde_json::to_string(&db(vec![e("1", "a", "A", None)])).unwrap();
        std::fs::write(d.join("base"), "").unwrap();
        std::fs::write(d.join("ours"), "{not json\n").unwrap();
        std::fs::write(d.join("theirs"), format!("{}\n", good)).unwrap();
        assert_eq!(run_driver(&d.join("base"), &d.join("ours"), &d.join("theirs")), 1);
        let out = std::fs::read_to_string(d.join("ours")).unwrap();
        assert!(out.contains("<<<<<<< ours") && out.contains("{not json") && out.contains(&good), "{}", out);
    }

    #[test]
    fn an_empty_base_means_both_sides_added_everything() {
        let d = scratch("emptybase");
        std::fs::write(d.join("base"), "").unwrap();
        std::fs::write(d.join("ours"), serde_json::to_string(&db(vec![e("1", "a", "A", None)])).unwrap()).unwrap();
        std::fs::write(d.join("theirs"), serde_json::to_string(&db(vec![e("2", "b", "B", None)])).unwrap()).unwrap();
        assert_eq!(run_driver(&d.join("base"), &d.join("ours"), &d.join("theirs")), 0);
        let out = crate::storage::load_db(&d.join("ours")).unwrap();
        assert_eq!(titles(&out), vec!["A", "B"]);
    }

    #[test]
    fn the_driver_command_quotes_the_executable_path() {
        let c = driver_command(std::path::Path::new("/Users/x/Library/Application Support/it's/bibox"));
        assert_eq!(c, r"'/Users/x/Library/Application Support/it'\''s/bibox' merge-db %O %A %B");
    }

    fn git_repo(tag: &str) -> std::path::PathBuf {
        let d = scratch(tag);
        assert!(std::process::Command::new("git").arg("-C").arg(&d).args(["init", "-q"]).status().unwrap().success());
        d
    }

    #[test]
    fn registration_writes_the_attribute_and_the_config_once() {
        let home = git_repo("reg");
        std::fs::write(home.join(".gitattributes"), "*.pdf binary\n").unwrap();
        let exe = std::path::Path::new("/opt/bibox");
        assert!(registration_problem(&home, exe).is_some());
        assert_eq!(ensure_registered(&home, exe), Ok(true));
        assert_eq!(ensure_registered(&home, exe), Ok(false), "second time: nothing to change");
        assert_eq!(std::fs::read_to_string(home.join(".gitattributes")).unwrap(), "*.pdf binary\ndb.json merge=bibox\n");
        assert_eq!(registration_problem(&home, exe), None);
        // 실행 파일이 옮겨지면 설정만 고친다
        assert_eq!(ensure_registered(&home, std::path::Path::new("/new/bibox")), Ok(true));
        assert!(registration_problem(&home, std::path::Path::new("/new/bibox")).is_none());
    }
}
