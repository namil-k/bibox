//! 플러그인 필드: 선언과 사용자 덮어쓰기, 세션 캐시, 행·상태 바 레이아웃. 순수 계산만.
//! 값은 플러그인이, 자리는 선언과 config가, 그리기는 tui.rs가 한다.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde_json::Value;

use crate::plugin::manifest::Manifest;
use crate::plugin::protocol::{FieldValue, FieldsMap};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    Row(u8),
    Status,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor {
    Key,
    Pdf,
    Year,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Right,
    Left,
    After(Anchor),
}

pub fn parse_place(s: &str) -> Option<Place> {
    match s {
        "row.1" => Some(Place::Row(1)),
        "row.2" => Some(Place::Row(2)),
        "row.3" => Some(Place::Row(3)),
        "status" => Some(Place::Status),
        _ => None,
    }
}

pub fn parse_align(s: &str) -> Option<Align> {
    match s {
        "right" => Some(Align::Right),
        "left" => Some(Align::Left),
        "after:key" => Some(Align::After(Anchor::Key)),
        "after:pdf" => Some(Align::After(Anchor::Pdf)),
        "after:year" => Some(Align::After(Anchor::Year)),
        _ => None,
    }
}

/// 매니페스트 선언에 사용자 덮어쓰기를 얹은 것. 그리기는 이것만 본다.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldDecl {
    pub plugin: String,
    pub id: String,
    pub place: Place,
    pub align: Align,
    pub width: u16,
    pub color: Option<String>,
    pub enabled: bool,
}

/// 플러그인 이름순, 선언 순. `[plugins.<name>.fields.<id>]`의 place/align/width/enabled로 덮어쓴다.
/// 잘못된 덮어쓰기는 무시한다(doctor가 따로 알린다).
pub fn decls(manifests: &[Manifest], config_tables: &BTreeMap<String, Value>) -> Vec<FieldDecl> {
    let mut sorted: Vec<&Manifest> = manifests.iter().collect();
    sorted.sort_by(|a, b| a.name.cmp(&b.name));
    let mut out = Vec::new();
    for m in sorted {
        let overrides = config_tables.get(&m.name).and_then(|t| t.get("fields"));
        for f in &m.fields {
            let (Some(mut place), Some(mut align)) = (parse_place(&f.place), parse_align(&f.align)) else { continue };
            let mut width = f.width;
            let mut enabled = true;
            if let Some(o) = overrides.and_then(|o| o.get(&f.id)) {
                if let Some(p) = o.get("place").and_then(Value::as_str).and_then(parse_place) { place = p; }
                if let Some(a) = o.get("align").and_then(Value::as_str).and_then(parse_align) { align = a; }
                if let Some(w) = o.get("width").and_then(Value::as_u64) { width = w.clamp(1, 80) as u16; }
                if let Some(e) = o.get("enabled").and_then(Value::as_bool) { enabled = e; }
            }
            out.push(FieldDecl { plugin: m.name.clone(), id: f.id.clone(), place, align, width, color: f.color.clone(), enabled });
        }
    }
    out
}

/// 세션 캐시. 키마다 한 번만 묻고, 플러그인이 push한 값은 덮어쓴다.
#[derive(Default)]
pub struct FieldStore {
    /// (플러그인, 항목 키) → 필드 id → 값
    values: HashMap<(String, String), BTreeMap<String, FieldValue>>,
    asked: HashSet<String>,
}

impl FieldStore {
    pub fn set_many(&mut self, plugin: &str, fields: &FieldsMap) {
        for (key, byid) in fields {
            let slot = self.values.entry((plugin.to_string(), key.clone())).or_default();
            for (id, v) in byid {
                slot.insert(id.clone(), v.clone());
            }
        }
    }

    pub fn get(&self, key: &str, plugin: &str, id: &str) -> Option<&FieldValue> {
        self.values.get(&(plugin.to_string(), key.to_string()))?.get(id)
    }

    /// 아직 안 물은 키를 순서대로 `max`개까지. 돌려준 키는 물은 것으로 친다.
    pub fn wanted(&mut self, keys: &[String], max: usize) -> Vec<String> {
        let mut out = Vec::new();
        for k in keys {
            if out.len() >= max { break; }
            if self.asked.insert(k.clone()) {
                out.push(k.clone());
            }
        }
        out
    }

    /// 다시 묻게(쓰기 뒤, 명시적 새로고침).
    pub fn forget(&mut self, keys: &[String]) {
        for k in keys {
            self.asked.remove(k);
        }
    }

    pub fn clear_plugin(&mut self, plugin: &str) {
        self.values.retain(|(p, _), _| p != plugin);
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Placed {
    pub anchor: Option<Anchor>,
    pub text: String,
    pub color: Option<String>,
    pub width: u16,
}

/// 한 항목의 `row`줄에 놓을 값. (앵커 뒤에 붙일 것, 오른쪽 정렬할 것). 값이 없거나 꺼진 필드는 없다.
pub fn row_cells(decls: &[FieldDecl], store: &FieldStore, key: &str, row: u8) -> (Vec<Placed>, Vec<Placed>) {
    let mut after = Vec::new();
    let mut right = Vec::new();
    for d in decls.iter().filter(|d| d.enabled && d.place == Place::Row(row)) {
        let Some(v) = store.get(key, &d.plugin, &d.id) else { continue };
        if v.text.is_empty() { continue; }
        let cell = Placed { anchor: None, text: v.text.clone(), color: v.color.clone().or_else(|| d.color.clone()), width: d.width };
        match d.align {
            Align::After(a) => after.push(Placed { anchor: Some(a), ..cell }),
            Align::Left => after.push(Placed { anchor: Some(Anchor::Key), ..cell }),
            Align::Right => right.push(cell),
        }
    }
    (after, right)
}

fn clip(text: &str, width: u16) -> String {
    let n = text.chars().count();
    if n <= width as usize { return text.to_string(); }
    if width == 0 { return String::new(); }
    let mut s: String = text.chars().take(width as usize - 1).collect();
    s.push('…');
    s
}

/// 오른쪽 정렬. 두 칸 띄워 잇고, 안 들어가는 셀은 뒤부터 버린다. 앞에 패딩을 붙여 `avail`칸을 꽉 채운다.
pub fn fit_right(avail: u16, cells: &[(String, u16)]) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut used: usize = 0;
    for (text, width) in cells {
        let t = clip(text, *width);
        let n = t.chars().count();
        let need = if parts.is_empty() { n } else { n + 2 };
        if used + need > avail as usize { break; }
        used += need;
        parts.push(t);
    }
    if parts.is_empty() { return String::new(); }
    let body = parts.join("  ");
    format!("{}{}", " ".repeat(avail as usize - used), body)
}

/// 하단 한 줄. 오른쪽 조각이 먼저 자리를 갖고, 힌트는 남는 폭에 잘라 넣는다.
pub fn status_line(width: u16, hints: &str, segments: &[String]) -> String {
    let right = if segments.is_empty() { String::new() } else { segments.join("  ") };
    let rn = right.chars().count();
    let w = width as usize;
    if rn >= w {
        return right.chars().rev().take(w).collect::<Vec<_>>().into_iter().rev().collect();
    }
    let avail = if rn == 0 { w } else { w.saturating_sub(rn + 2) };
    let left: String = hints.chars().take(avail).collect();
    let pad = w.saturating_sub(left.chars().count() + rn);
    format!("{}{}{}", left, " ".repeat(pad), right)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::protocol::FieldValue;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn manifest(name: &str, fields: &[(&str, &str, &str, u16)]) -> crate::plugin::manifest::Manifest {
        let specs = fields.iter().map(|(id, place, align, width)| format!("[[fields]]\nid = \"{}\"\nplace = \"{}\"\nalign = \"{}\"\nwidth = {}\n", id, place, align, width)).collect::<String>();
        let text = format!("api = 2\nname = \"{}\"\nrun = \"sh x\"\n{}", name, specs);
        let mut problems = vec![];
        crate::plugin::manifest::parse_manifest(std::path::Path::new(&format!("/tmp/plugins/{}", name)), &text, &mut problems).unwrap()
    }

    #[test]
    fn places_and_aligns_parse() {
        assert_eq!(parse_place("row.2"), Some(Place::Row(2)));
        assert_eq!(parse_place("status"), Some(Place::Status));
        assert_eq!(parse_place("row.4"), None);
        assert_eq!(parse_align("after:key"), Some(Align::After(Anchor::Key)));
        assert_eq!(parse_align("left"), Some(Align::Left));
        assert_eq!(parse_align("after:title"), None);
    }

    /// 선언은 플러그인 이름순, config의 [plugins.x.fields.id]가 place/align/width/enabled를 덮어쓴다.
    #[test]
    fn user_config_overrides_a_declared_slot_and_can_disable_it() {
        let ms = vec![manifest("zeta", &[("z", "row.1", "right", 6)]), manifest("alpha", &[("a", "row.1", "right", 8), ("b", "status", "right", 10)])];
        let mut tables = BTreeMap::new();
        tables.insert("alpha".to_string(), json!({"fields": {"a": {"place": "row.3", "width": 4}, "b": {"enabled": false}}, "other": 1}));
        let d = decls(&ms, &tables);
        assert_eq!(d.iter().map(|x| format!("{}.{}", x.plugin, x.id)).collect::<Vec<_>>(), vec!["alpha.a", "alpha.b", "zeta.z"]);
        assert_eq!((d[0].place, d[0].width, d[0].enabled), (Place::Row(3), 4, true));
        assert!(!d[1].enabled);
        assert_eq!((d[2].place, d[2].width), (Place::Row(1), 6));
        let mut bad = BTreeMap::new();
        bad.insert("zeta".to_string(), json!({"fields": {"z": {"place": "row.9"}}}));
        assert_eq!(decls(&ms, &bad)[2].place, Place::Row(1), "a bad override keeps the declaration");
    }

    #[test]
    fn the_store_asks_each_key_once_until_forgotten_and_keeps_values_per_plugin() {
        let mut s = FieldStore::default();
        let keys = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        assert_eq!(s.wanted(&keys, 2), vec!["a", "b"], "at most max keys, in order");
        assert_eq!(s.wanted(&keys, 10), vec!["c"], "already asked keys are not asked again");
        assert!(s.wanted(&keys, 10).is_empty());
        let mut m: crate::plugin::protocol::FieldsMap = BTreeMap::new();
        m.entry("a".into()).or_default().insert("count".into(), FieldValue { text: "3".into(), color: None });
        s.set_many("cit", &m);
        assert_eq!(s.get("a", "cit", "count").map(|v| v.text.as_str()), Some("3"));
        assert!(s.get("a", "other", "count").is_none());
        s.forget(&["a".to_string()]);
        assert_eq!(s.wanted(&keys, 10), vec!["a"], "forgotten keys are asked again");
        s.clear_plugin("cit");
        assert!(s.get("a", "cit", "count").is_none());
    }

    #[test]
    fn row_cells_split_anchored_and_right_aligned_values_and_skip_missing_or_disabled() {
        let ms = vec![manifest("cit", &[("count", "row.1", "right", 8), ("read", "row.1", "after:key", 3), ("note", "row.3", "right", 5)])];
        let mut tables = BTreeMap::new();
        tables.insert("cit".to_string(), json!({"fields": {"note": {"enabled": false}}}));
        let d = decls(&ms, &tables);
        let mut s = FieldStore::default();
        let mut m: crate::plugin::protocol::FieldsMap = BTreeMap::new();
        let e = m.entry("k".into()).or_default();
        e.insert("count".into(), FieldValue { text: "★ 12".into(), color: Some("yellow".into()) });
        e.insert("read".into(), FieldValue { text: "✓".into(), color: None });
        e.insert("note".into(), FieldValue { text: "n".into(), color: None });
        s.set_many("cit", &m);
        let (after, right) = row_cells(&d, &s, "k", 1);
        assert_eq!(after.len(), 1);
        assert_eq!((after[0].anchor, after[0].text.as_str()), (Some(Anchor::Key), "✓"));
        assert_eq!((right.len(), right[0].text.as_str(), right[0].color.as_deref()), (1, "★ 12", Some("yellow")));
        let (after3, right3) = row_cells(&d, &s, "k", 3);
        assert!(after3.is_empty() && right3.is_empty(), "disabled field draws nothing");
        let (a, r) = row_cells(&d, &s, "unknown", 1);
        assert!(a.is_empty() && r.is_empty());
    }

    #[test]
    fn fit_right_pads_truncates_and_drops_from_the_end() {
        assert_eq!(fit_right(20, &[("★ 12".into(), 8), ("✓".into(), 3)]), "          ★ 12  ✓");
        assert_eq!(fit_right(6, &[("verylongtext".into(), 5)]), " very…");
        assert_eq!(fit_right(5, &[("abcde".into(), 8), ("xy".into(), 3)]), "abcde", "the second cell does not fit and is dropped");
        assert_eq!(fit_right(0, &[("a".into(), 1)]), "");
        assert_eq!(fit_right(4, &[]), "");
    }

    #[test]
    fn the_status_line_keeps_segments_and_truncates_hints_first() {
        assert_eq!(status_line(40, "h←collections  q quit", &["↑2".into(), "S2 480".into()]), "h←collections  q quit         ↑2  S2 480");
        let s = status_line(24, "h←collections  q quit", &["↑2".into(), "S2 480".into()]);
        assert_eq!(s.chars().count(), 24);
        assert!(s.ends_with("↑2  S2 480"), "{:?}", s);
        assert!(s.starts_with("h←collec"), "{:?}", s);
        assert_eq!(status_line(10, "hints", &[]), "hints     ");
        assert_eq!(status_line(4, "hints", &["seg".into()]), " seg", "no room for hints at all");
    }
}
