//! db.json 3방향 병합. git이 `merge.bibox.driver`로 부른다(`bibox merge-db %O %A %B`).
//! 스펙: docs/superpowers/specs/2026-10-10-db-merge-design.md

use std::collections::HashMap;
use std::path::{Path, PathBuf};
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
    let first_record = log.len();
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
        match winner {
            Side::Ours => o.updated_at.clone(),
            Side::Theirs => t.updated_at.clone(),
        }
    };
    // 기록은 ours 판으로 만들었다. doctor가 맞는 논문을 가리키도록 병합 뒤의 키로 바꾼다
    for r in &mut log[first_record..] {
        r["bibtex_key"] = Value::String(merged.bibtex_key.clone());
    }
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
        // 이 논문의 앞선 기록(칸 겹침)도 새 키를 가리키게 한다. id가 빈 옛 항목은 구별할 수 없어 둔다
        let id = entries[k].id.clone();
        if !id.is_empty() {
            for r in log.iter_mut().filter(|r| r["id"] == id.as_str()) {
                r["bibtex_key"] = Value::String(new_key.clone());
            }
        }
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
    run_guarded(|| merge_files(base, ours, theirs), base, ours, theirs)
}

/// 병합을 돌려 종료 코드로 바꾼다. 오류든 패닉이든 git의 줄 단위 병합을 남기고 1.
fn run_guarded(job: impl FnOnce() -> Result<usize, String> + std::panic::UnwindSafe, base: &Path, ours: &Path, theirs: &Path) -> i32 {
    match std::panic::catch_unwind(job) {
        Ok(Ok(records)) => {
            if records > 0 {
                eprintln!("bibox merge-db: {} field(s) kept by rule", records);
            }
            0
        }
        Ok(Err(e)) => {
            eprintln!("bibox merge-db: {}", e);
            text_merge(base, ours, theirs);
            1
        }
        Err(_) => {
            eprintln!("bibox merge-db: the merge panicked; leaving git's text merge");
            text_merge(base, ours, theirs);
            1
        }
    }
}

fn merge_files(base: &Path, ours: &Path, theirs: &Path) -> Result<usize, String> {
    let (b, o, t) = (read_strict(base)?, read_strict(ours)?, read_strict(theirs)?);
    let out = merge(&b, &o, &t, &crate::models::now_stamp())?;
    // 옆의 임시 파일에 다 쓴 뒤 이름을 바꾼다. 쓰다 실패해도 ours가 반쯤 쓰인 채 남지 않는다
    let json = serde_json::to_string_pretty(&out.db).map_err(|e| e.to_string())?;
    let tmp = atomic_temp(ours);
    std::fs::write(&tmp, json)
        .and_then(|_| std::fs::rename(&tmp, ours))
        .map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            format!("cannot write {}: {}", ours.display(), e)
        })?;
    Ok(out.records)
}

/// `ours` 바로 옆의 임시 파일 이름. 같은 디렉토리라 rename이 한 번에 바뀐다.
fn atomic_temp(ours: &Path) -> PathBuf {
    let name = ours.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    ours.with_file_name(format!("{}.bibox-{}.tmp", name, std::process::id()))
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

/// `home` 저장소에 대한 git. git hook 안처럼 GIT_DIR 등이 내보내져 있으면 -C보다 그쪽이 이겨
/// 엉뚱한 저장소에 쓰게 되므로 지운다.
fn git_in(home: &Path) -> Command {
    let mut c = Command::new("git");
    c.env_remove("GIT_DIR").env_remove("GIT_WORK_TREE").env_remove("GIT_INDEX_FILE").arg("-C").arg(home);
    c
}

fn git_out(home: &Path, args: &[&str]) -> Option<String> {
    let o = git_in(home).args(args).output().ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

fn git_config_get(home: &Path, key: &str) -> Option<String> {
    git_out(home, &["config", "--local", "--get", key])
}

fn git_config_set(home: &Path, key: &str, value: &str) -> Result<(), String> {
    let o = git_in(home).args(["config", "--local", key, value]).output().map_err(|e| e.to_string())?;
    if o.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&o.stderr).trim().to_string()) }
}

/// 홈이 저장소 루트일 때 그 git 디렉토리와 info/attributes 경로. git 한 번으로 묻는다.
pub struct RepoPaths {
    pub git_dir: PathBuf,
    pub attributes: PathBuf,
}

pub fn repo_root_paths(home: &Path) -> Option<RepoPaths> {
    let out = git_out(home, &["rev-parse", "--show-toplevel", "--absolute-git-dir", "--git-path", "info/attributes"])?;
    let mut lines = out.lines();
    let (top, git_dir, attributes) = (lines.next()?, lines.next()?, lines.next()?);
    if std::fs::canonicalize(top).ok()? != std::fs::canonicalize(home).ok()? {
        return None;
    }
    // --git-path는 -C 기준 상대 경로일 수 있다
    Some(RepoPaths { git_dir: PathBuf::from(git_dir), attributes: home.join(attributes) })
}

/// 진행 중인 rebase·merge가 있으면 그 이름. 그 사이에는 저장소를 건드리지 않는다.
pub fn in_progress(git_dir: &Path) -> Option<&'static str> {
    if git_dir.join("rebase-merge").exists() || git_dir.join("rebase-apply").exists() {
        Some("rebase")
    } else if git_dir.join("MERGE_HEAD").exists() {
        Some("merge")
    } else {
        None
    }
}

/// 이 기계만 읽는 `$GIT_DIR/info/attributes`. 커밋되는 `.gitattributes`는 pull --rebase 때 원격 쪽 것을 읽어서 쓸 수 없다.
fn attributes_path(home: &Path) -> Option<PathBuf> {
    git_out(home, &["rev-parse", "--git-path", "info/attributes"]).map(|p| home.join(p))
}

fn has_attr_line(attributes: &Path) -> bool {
    std::fs::read_to_string(attributes).map(|s| s.lines().any(|l| l.trim() == ATTR_LINE)).unwrap_or(false)
}

/// 빠진 것을 말한다. 없으면 None.
pub fn registration_problem(home: &Path, exe: &Path) -> Option<String> {
    let Some(attributes) = attributes_path(home) else {
        return Some(format!("{} is not a git repository", home.display()));
    };
    if !has_attr_line(&attributes) {
        return Some(format!("{} has no `{}`", attributes.display(), ATTR_LINE));
    }
    if git_config_get(home, "merge.bibox.driver").as_deref() != Some(driver_command(exe).as_str()) {
        return Some("this machine's git config does not run bibox to merge db.json".to_string());
    }
    None
}

/// info/attributes 줄과 이 기계의 git 설정을 맞춘다. 바꾼 게 있으면 true.
pub fn ensure_registered(home: &Path, exe: &Path) -> Result<bool, String> {
    let attributes = attributes_path(home).ok_or_else(|| format!("{} is not a git repository", home.display()))?;
    ensure_registered_at(home, &attributes, exe)
}

fn ensure_registered_at(home: &Path, attributes: &Path, exe: &Path) -> Result<bool, String> {
    let mut changed = false;
    if !has_attr_line(attributes) {
        let mut s = std::fs::read_to_string(attributes).unwrap_or_default();
        if !s.is_empty() && !s.ends_with('\n') {
            s.push('\n');
        }
        s.push_str(ATTR_LINE);
        s.push('\n');
        if let Some(dir) = attributes.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {}", dir.display(), e))?;
        }
        std::fs::write(attributes, s).map_err(|e| format!("cannot write {}: {}", attributes.display(), e))?;
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

/// 모든 명령 앞에서 부른다. 홈이 저장소 루트이고 rebase·merge 중이 아닐 때만, 실패는 조용히(doctor가 보고한다).
/// 작업 트리에는 아무것도 만들지 않는다(info/attributes와 .git/config뿐).
pub fn register_quietly(home: &Path) {
    let Some(paths) = repo_root_paths(home) else { return };
    if in_progress(&paths.git_dir).is_some() {
        return;
    }
    if let Ok(exe) = std::env::current_exe() {
        let _ = ensure_registered_at(home, &paths.attributes, &exe);
    }
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

    /// 기록의 bibtex_key는 병합 뒤 그 논문의 키다. theirs가 키 겹침에서 이기거나 나중에 접미사가 붙어도.
    #[test]
    fn records_carry_the_key_the_entry_ends_up_with() {
        let base = db(vec![e("1", "a", "A", None)]);
        let ours = db(vec![e("1", "a_o", "A", Some("2026-10-10T10:00:00+02:00"))]);
        let theirs = db(vec![e("1", "a_t", "A", Some("2026-10-10T08:05:00+00:00"))]);
        let out = merge(&base, &ours, &theirs, NOW).unwrap();
        assert_eq!(out.db.entries[0].bibtex_key, "a_t");
        assert_eq!(out.db.merge_log[0]["bibtex_key"], "a_t");

        // theirs 쪽 항목이 제목에서 겹친 뒤 키 겹침으로 이름이 바뀌면, 제목 기록도 새 키를 쓴다
        // theirs가 2의 키를 k로 바꾸고 제목도 고쳤는데 ours의 새 항목 1이 이미 k다: 2는 ka가 된다
        let base = db(vec![e("2", "b", "T", None)]);
        let ours = db(vec![e("2", "b", "T ours", Some("2026-10-10T10:00:00+02:00")), e("1", "k", "Mine", None)]);
        let theirs = db(vec![e("2", "k", "T theirs", Some("2026-10-10T08:05:00+00:00"))]);
        let out = merge(&base, &ours, &theirs, NOW).unwrap();
        let two = out.db.entries.iter().find(|x| x.id == "2").unwrap();
        assert_eq!(two.bibtex_key, "ka");
        let title_rec = out.db.merge_log.iter().find(|r| r["field"] == "title").unwrap();
        assert_eq!(title_rec["bibtex_key"].as_str(), Some(two.bibtex_key.as_str()));
    }

    /// 지우기와 고치기가 겹치면 고친 쪽이 남는다. ours가 고치고 theirs가 지운 방향.
    #[test]
    fn an_edit_on_our_side_survives_a_deletion_on_theirs() {
        let base = db(vec![e("1", "a", "A", None), e("2", "b", "B", None)]);
        let ours = db(vec![e("1", "a", "A2", Some("2026-10-10T10:00:00+02:00")), e("2", "b", "B", None)]);
        let theirs = db(vec![e("2", "b", "B", None)]);
        let out = merge(&base, &ours, &theirs, NOW).unwrap();
        assert_eq!(titles(&out.db), vec!["A2", "B"]);
        assert_eq!(out.records, 1);
        assert_eq!(out.db.merge_log[0]["rule"], "kept_over_delete");
        assert_eq!(out.db.merge_log[0]["id"], "1");
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

    /// 결과는 옆의 임시 파일에 다 쓴 뒤 이름을 바꿔 넣는다. 끝나면 임시 파일이 남지 않는다.
    #[test]
    fn the_result_replaces_ours_whole_and_leaves_no_temporary_file() {
        let d = scratch("atomic");
        std::fs::write(d.join("base"), serde_json::to_string(&db(vec![e("1", "a", "A", None)])).unwrap()).unwrap();
        std::fs::write(d.join("ours"), serde_json::to_string(&db(vec![e("1", "a", "A", None), e("2", "b", "B", None)])).unwrap()).unwrap();
        std::fs::write(d.join("theirs"), serde_json::to_string(&db(vec![e("1", "a", "A", None), e("3", "c", "C", None)])).unwrap()).unwrap();
        assert_eq!(run_driver(&d.join("base"), &d.join("ours"), &d.join("theirs")), 0);
        // theirs에만 있는 C는 theirs에서의 앞 항목(A) 바로 뒤에 놓인다
        assert_eq!(titles(&crate::storage::load_db(&d.join("ours")).unwrap()), vec!["A", "C", "B"]);
        let mut names: Vec<String> = std::fs::read_dir(&d).unwrap().map(|x| x.unwrap().file_name().to_string_lossy().to_string()).collect();
        names.sort();
        assert_eq!(names, vec!["base", "ours", "theirs"]);
        assert!(atomic_temp(&d.join("ours")).file_name().unwrap().to_string_lossy().starts_with("ours."), "the temporary sits next to ours");
    }

    /// 패닉도 오류와 같다: 도우미가 없는 기계처럼 충돌 표시를 남기고 1로 끝난다.
    #[test]
    fn a_panicking_merge_still_leaves_conflict_markers() {
        let d = scratch("panic");
        let good = serde_json::to_string(&db(vec![e("1", "a", "A", None)])).unwrap();
        let mine = serde_json::to_string(&db(vec![e("1", "a", "Mine", None)])).unwrap();
        let yours = serde_json::to_string(&db(vec![e("1", "a", "Yours", None)])).unwrap();
        std::fs::write(d.join("base"), format!("{}\n", good)).unwrap();
        std::fs::write(d.join("ours"), format!("{}\n", mine)).unwrap();
        std::fs::write(d.join("theirs"), format!("{}\n", yours)).unwrap();
        let code = run_guarded(|| -> Result<usize, String> { panic!("boom") }, &d.join("base"), &d.join("ours"), &d.join("theirs"));
        assert_eq!(code, 1);
        let out = std::fs::read_to_string(d.join("ours")).unwrap();
        assert!(out.contains("<<<<<<< ours") && out.contains(&mine) && out.contains(&yours), "{}", out);
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
    fn registration_writes_info_attributes_and_the_config_once() {
        let home = git_repo("reg");
        let attributes = home.join(".git/info/attributes");
        std::fs::write(&attributes, "*.pdf binary").unwrap();
        let exe = std::path::Path::new("/opt/bibox");
        assert!(registration_problem(&home, exe).is_some());
        assert_eq!(ensure_registered(&home, exe), Ok(true));
        assert_eq!(ensure_registered(&home, exe), Ok(false), "second time: nothing to change");
        assert_eq!(std::fs::read_to_string(&attributes).unwrap(), "*.pdf binary\ndb.json merge=bibox\n");
        assert!(!home.join(".gitattributes").exists(), "nothing in the work tree");
        assert_eq!(registration_problem(&home, exe), None);
        // 실행 파일이 옮겨지면 설정만 고친다
        assert_eq!(ensure_registered(&home, std::path::Path::new("/new/bibox")), Ok(true));
        assert!(registration_problem(&home, std::path::Path::new("/new/bibox")).is_none());
    }

    #[test]
    fn registration_creates_a_missing_info_directory() {
        let home = git_repo("noinfo");
        let _ = std::fs::remove_dir_all(home.join(".git/info"));
        assert_eq!(ensure_registered(&home, std::path::Path::new("/opt/bibox")), Ok(true));
        assert_eq!(std::fs::read_to_string(home.join(".git/info/attributes")).unwrap(), "db.json merge=bibox\n");
    }

    #[test]
    fn quiet_registration_skips_a_subdirectory_and_a_rebase_in_progress() {
        let home = git_repo("quiet");
        std::fs::create_dir_all(home.join("sub")).unwrap();
        register_quietly(&home.join("sub"));
        assert!(git_config_get(&home, "merge.bibox.driver").is_none(), "not the repository root");
        std::fs::create_dir_all(home.join(".git/rebase-merge")).unwrap();
        register_quietly(&home);
        assert!(git_config_get(&home, "merge.bibox.driver").is_none(), "a rebase is in progress");
        std::fs::remove_dir_all(home.join(".git/rebase-merge")).unwrap();
        register_quietly(&home);
        assert!(git_config_get(&home, "merge.bibox.driver").is_some());
        assert!(has_attr_line(&home.join(".git/info/attributes")));
    }
}
