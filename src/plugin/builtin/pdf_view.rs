//! pdf-view 내장 플러그인. 미리보기 패널의 PDF 탭. 호스트가 `views/render`로 "n쪽을 W픽셀 폭으로"라고 물으면
//! poppler(`pdfinfo`, `pdftoppm`, `pdftotext`)로 JPEG 또는 텍스트를 만든다. 스크롤·배율·캐시는 호스트 일.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::plugin::builtin::Builtin;
use serde_json::Value;

use crate::plugin::protocol::{ViewParams, ViewResult};
use crate::plugin::rpc::RpcError;
use crate::plugin::serve::{serve, Ui};

pub const MANIFEST: &str = r#"api = 2
name = "pdf-view"
description = "Show the attached PDF page by page in the preview panel"

[[views]]
title = "PDF"
run = "render"

[[commands]]
id = "render"
desc = "Render a page of the attached PDF"

[[settings]]
key = "max_zoom"
type = "int"
default = 400
desc = "Largest zoom in percent"

[[settings]]
key = "dpi_cap"
type = "int"
default = 300
desc = "Never rasterize above this dpi"
"#;

pub const GUIDE: &str = "TUI only: adds the PDF tab to the preview panel (Tab cycles Info, Note, PDF; `n`/`p` pages, `+`/`-`/`0` zoom, `H`/`L` pan). Not installed by default; `bibox plugin install pdf-view` turns it on. Needs poppler on PATH (`pdfinfo`, `pdftoppm`, `pdftotext`; `brew install poppler` or `apt install poppler-utils`), which `bibox doctor` checks. Settings under `[plugins.pdf-view]`: `max_zoom` (percent, default 400) and `dpi_cap` (default 300).";

pub const BUILTIN: Builtin = Builtin { name: "pdf-view", manifest: MANIFEST, run, seeded: false, guide: GUIDE };

/// doctor가 PATH에서 찾는 도구들
pub const TOOLS: [&str; 3] = ["pdfinfo", "pdftoppm", "pdftotext"];
pub const NEEDS_POPPLER: &str = "pdf-view needs poppler (pdftoppm, pdfinfo, pdftotext). brew install poppler";
const NOT_A_TAB: &str = "pdf-view renders the PDF tab; open the preview panel";
const NO_TEXT: &str = "(no text on this page)";

fn run() {
    // 지난 세션의 PNG. 호스트 캐시는 항목 하나뿐이라 디스크 것도 오래 둘 이유가 없다
    let _ = std::fs::remove_dir_all(cache_dir());
    let mut st = State::default();
    let mut handler = |method: &str, params: Value, _ui: &mut Ui| -> Result<Value, RpcError> { st.handle(method, params) };
    serve("pdf-view", &mut handler);
}

/// initialize에서 받은 pdfs 디렉토리와 dpi_cap. `config/changed`가 dpi_cap만 바꾼다.
#[derive(Default)]
struct State {
    pdfs: PathBuf,
    dpi_cap: u32,
}

impl State {
    fn set_config(&mut self, c: &Value) {
        self.dpi_cap = c.get("dpi_cap").and_then(Value::as_u64).map(|v| v as u32).unwrap_or(300);
    }

    /// 뷰 렌더는 팝업을 열 수 없어 `Ui`를 쓰지 않는다.
    fn handle(&mut self, method: &str, params: Value) -> Result<Value, RpcError> {
        match method {
            "initialize" => {
                self.pdfs = params.pointer("/paths/pdfs").and_then(Value::as_str).map(PathBuf::from).unwrap_or_default();
                self.set_config(params.get("config").unwrap_or(&Value::Null));
                Ok(Value::Null)
            }
            "config/changed" => {
                self.set_config(params.get("config").unwrap_or(&Value::Null));
                Ok(Value::Null)
            }
            "views/render" => {
                let p: ViewParams = serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;
                render(self, &p, &POPPLER).map(|r| serde_json::to_value(r).unwrap_or(Value::Null)).map_err(RpcError::plugin)
            }
            "commands/run" => Err(RpcError::plugin(NOT_A_TAB)),
            other => Err(RpcError::method_not_found(other)),
        }
    }
}

pub(crate) fn cache_dir() -> PathBuf {
    std::env::temp_dir().join("bibox-pdf-view")
}

/// 실행할 프로그램 이름. 테스트가 없는 이름을 넣어 poppler 부재를 흉내낸다.
struct Tools {
    pdfinfo: &'static str,
    pdftoppm: &'static str,
    pdftotext: &'static str,
}

const POPPLER: Tools = Tools { pdfinfo: "pdfinfo", pdftoppm: "pdftoppm", pdftotext: "pdftotext" };

// ── 순수 함수 ────────────────────────────────────────────────────────────────

pub(crate) struct Info {
    pub pages: u32,
    /// 첫 쪽의 폭(pt). `Page size:` 줄이 없으면 letter(612)
    pub width_pt: f64,
}

/// `pdfinfo` 출력. `Pages:` 줄이 없으면 None.
pub(crate) fn parse_pdfinfo(out: &str) -> Option<Info> {
    let mut pages = None;
    let mut width_pt = 612.0;
    for line in out.lines() {
        if let Some(v) = line.strip_prefix("Pages:") {
            pages = v.trim().parse::<u32>().ok();
        } else if let Some(v) = line.strip_prefix("Page size:") {
            if let Some(w) = v.split_whitespace().next().and_then(|w| w.parse::<f64>().ok()) {
                if w > 0.0 {
                    width_pt = w;
                }
            }
        }
    }
    pages.map(|pages| Info { pages, width_pt })
}

/// poppler에 시킬 픽셀 폭. 원하는 폭 그대로, 다만 `dpi_cap`을 넘는 폭은 잘라 낸다(쪽 폭 pt / 72 = 인치).
/// 정확한 폭으로 받아야 호스트가 리사이즈할 일이 없다(2240px 쪽 하나에 80ms였다).
pub(crate) fn render_width(width_px: u32, width_pt: f64, cap: u32) -> u32 {
    let wanted = width_px.max(1);
    if width_pt <= 0.0 {
        return wanted;
    }
    let max = (width_pt * cap.max(1) as f64 / 72.0).round().max(1.0) as u32;
    wanted.min(max)
}

/// `<dir>/<key>-<page>-<width>`. `pdftoppm -singlefile`이 `.jpg`를 붙인다.
pub(crate) fn image_prefix(dir: &Path, key: &str, page: u32, width_px: u32) -> PathBuf {
    let safe: String = key.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '_' }).collect();
    dir.join(format!("{}-{}-{}", safe, page, width_px))
}

fn first_line(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("").to_string()
}

// ── poppler ──────────────────────────────────────────────────────────────────

/// 프로그램이 없으면 설치 안내, 실패하면 첫 stderr 줄.
fn run_tool(program: &str, args: &[&str]) -> Result<Vec<u8>, String> {
    let out = Command::new(program).args(args).output().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound { NEEDS_POPPLER.to_string() } else { format!("{} failed: {}", program, e) }
    })?;
    if !out.status.success() {
        return Err(format!("{} failed: {}", program, first_line(&out.stderr)));
    }
    Ok(out.stdout)
}

fn pdfinfo(tools: &Tools, pdf: &Path) -> Result<Info, String> {
    let out = run_tool(tools.pdfinfo, &[&pdf.to_string_lossy()])?;
    parse_pdfinfo(&String::from_utf8_lossy(&out)).ok_or_else(|| format!("{} failed: no page count", tools.pdfinfo))
}

fn pdftotext(tools: &Tools, pdf: &Path, page: u32) -> Result<Vec<String>, String> {
    let p = page.to_string();
    let out = run_tool(tools.pdftotext, &["-f", &p, "-l", &p, "-layout", &pdf.to_string_lossy(), "-"])?;
    let text = String::from_utf8_lossy(&out);
    let mut lines: Vec<String> = text.lines().map(|l| l.trim_end_matches('\u{c}').trim_end().to_string()).collect();
    while lines.last().map(|l| l.is_empty()).unwrap_or(false) {
        lines.pop();
    }
    if lines.is_empty() {
        lines.push(NO_TEXT.to_string());
    }
    Ok(lines)
}

/// 이미 있으면 다시 만들지 않는다. JPEG인 이유: poppler의 PNG 인코딩이 래스터화(0.02s)의 열 배(0.19s)였다.
/// 품질 90은 글자 테두리를 확대해야 차이가 보이는 수준이고 쪽당 200KB 안팎.
fn pdftoppm(tools: &Tools, pdf: &Path, page: u32, width_px: u32, prefix: &Path) -> Result<PathBuf, String> {
    let jpg = prefix.with_extension("jpg");
    if jpg.is_file() {
        return Ok(jpg);
    }
    if let Some(dir) = prefix.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {}", dir.display(), e))?;
    }
    let p = page.to_string();
    let w = width_px.to_string();
    run_tool(tools.pdftoppm, &["-f", &p, "-l", &p, "-scale-to-x", &w, "-scale-to-y", "-1", "-jpeg", "-jpegopt", "quality=90", "-singlefile", &pdf.to_string_lossy(), &prefix.to_string_lossy()])?;
    if !jpg.is_file() {
        return Err(format!("{} wrote nothing at {}", tools.pdftoppm, jpg.display()));
    }
    Ok(jpg)
}

// ── 요청 처리 ────────────────────────────────────────────────────────────────

fn render(st: &State, p: &ViewParams, tools: &Tools) -> Result<ViewResult, String> {
    let Some(entry) = &p.entry else { return Err("no entry selected".to_string()) };
    let Some(fp) = &entry.file_path else { return Err("no PDF attached".to_string()) };
    let pdf = st.pdfs.join(fp);
    if !pdf.is_file() {
        return Err(format!("PDF not found: {}", pdf.display()));
    }
    let info = pdfinfo(tools, &pdf)?;
    let page = p.page.clamp(1, info.pages.max(1));
    if !p.images {
        let lines = pdftotext(tools, &pdf, page)?;
        return Ok(ViewResult { lines: Some(lines), pages: Some(info.pages), ..Default::default() });
    }
    let width = render_width(p.width_px, info.width_pt, st.dpi_cap);
    let prefix = image_prefix(&cache_dir(), &entry.bibtex_key, page, p.width_px);
    let jpg = pdftoppm(tools, &pdf, page, width, &prefix)?;
    Ok(ViewResult { image: Some(jpg), pages: Some(info.pages), ..Default::default() })
}

#[cfg(test)]
mod tests {
    use super::*;

    const INFO: &str = "Title:          x\nPages:          14\nPage size:      612 x 792 pts (letter)\nFile size:      1 bytes\n";

    #[test]
    fn pdfinfo_output_gives_pages_and_the_page_width() {
        let i = parse_pdfinfo(INFO).unwrap();
        assert_eq!(i.pages, 14);
        assert_eq!(i.width_pt, 612.0);
        assert!(parse_pdfinfo("Title: x\n").is_none(), "no Pages line");
        let i = parse_pdfinfo("Pages: 3\n").unwrap();
        assert_eq!((i.pages, i.width_pt), (3, 612.0), "missing size falls back to letter");
    }

    #[test]
    fn render_width_is_the_wanted_width_until_the_dpi_cap() {
        // 612pt = 8.5in. 300dpi면 2550px
        assert_eq!(render_width(850, 612.0, 300), 850);
        assert_eq!(render_width(8500, 612.0, 300), 2550, "capped at 300dpi");
        assert_eq!(render_width(0, 612.0, 300), 1, "zero width is never asked of poppler");
        assert_eq!(render_width(850, 0.0, 300), 850, "unknown page width means no cap");
    }

    #[test]
    fn image_prefix_is_per_entry_page_and_width() {
        let p = image_prefix(Path::new("/t"), "kim2025", 3, 840);
        assert_eq!(p, Path::new("/t/kim2025-3-840"));
        let p = image_prefix(Path::new("/t"), "we/ird key", 1, 1);
        assert_eq!(p, Path::new("/t/we_ird_key-1-1"), "keys never make directories");
    }

    fn state(pdfs: &Path) -> State {
        State { pdfs: pdfs.to_path_buf(), dpi_cap: 300 }
    }

    fn view(entry: Option<crate::models::Entry>, page: u32, width_px: u32, images: bool) -> ViewParams {
        ViewParams { view: "render".into(), entry, page, width_px, images }
    }

    /// protocol.rs 테스트의 `entry()`와 같은 꼴. `Entry`에 생성자가 없다.
    fn entry_with(fp: Option<&str>) -> crate::models::Entry {
        use crate::models::{Entry, EntryType};
        Entry {
            id: "1".into(), bibtex_key: "kim2025".into(), entry_type: EntryType::Article,
            title: Some("T".into()), author: vec!["Kim, J.".into()], year: Some(2025),
            journal: None, volume: None, number: None, pages: None, publisher: None, editor: None,
            edition: None, isbn: None, booktitle: None, doi: None, url: None, abstract_text: None,
            tags: vec![], howpublished: None, month: None, note: None, collections: vec![],
            file_path: fp.map(str::to_string), created_at: "2026-01-01 00:00:00".into(), updated_at: None,
        }
    }

    /// 테스트마다 다른 임시 디렉토리(tempfile 크레이트는 안 쓴다. hooks.rs와 같은 방식)
    fn scratch(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("bibox-pdf-view-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_command_is_told_to_open_the_preview_and_missing_pdfs_are_errors() {
        let mut st = state(Path::new("/p"));
        let r = st.handle("commands/run", serde_json::json!({"command": "render", "trigger": "key"}));
        assert_eq!(r.unwrap_err().message, "pdf-view renders the PDF tab; open the preview panel");
        assert_eq!(render(&st, &view(None, 1, 0, false), &POPPLER).unwrap_err(), "no entry selected");
        assert_eq!(render(&st, &view(Some(entry_with(None)), 1, 0, false), &POPPLER).unwrap_err(), "no PDF attached");
        let e = render(&st, &view(Some(entry_with(Some("gone.pdf"))), 1, 0, false), &POPPLER).unwrap_err();
        assert!(e.starts_with("PDF not found: /p/gone.pdf"), "{:?}", e);
        let r = st.handle("initialize", serde_json::json!({"paths": {"pdfs": "/q"}, "config": {"dpi_cap": 72}}));
        assert!(r.is_ok());
        assert_eq!((st.pdfs.as_path(), st.dpi_cap), (Path::new("/q"), 72));
        st.handle("config/changed", serde_json::json!({"config": {}})).unwrap();
        assert_eq!(st.dpi_cap, 300, "missing dpi_cap falls back to the default");
    }

    /// 쪽마다 빈 MediaBox만 있는 PDF. xref 오프셋을 계산해 두어 poppler가 경고 없이 읽는다.
    fn write_pdf(path: &Path, pages: usize) {
        let mut objs: Vec<String> = Vec::new();
        let kids: Vec<String> = (0..pages).map(|i| format!("{} 0 R", 3 + i)).collect();
        objs.push("<< /Type /Catalog /Pages 2 0 R >>".into());
        objs.push(format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), pages));
        for _ in 0..pages {
            objs.push("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>".into());
        }
        let mut out = String::from("%PDF-1.4\n");
        let mut offsets = Vec::new();
        for (i, o) in objs.iter().enumerate() {
            offsets.push(out.len());
            out.push_str(&format!("{} 0 obj\n{}\nendobj\n", i + 1, o));
        }
        let xref = out.len();
        out.push_str(&format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1));
        for off in offsets {
            out.push_str(&format!("{:010} 00000 n \n", off));
        }
        out.push_str(&format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n", objs.len() + 1, xref));
        std::fs::write(path, out).unwrap();
    }

    fn poppler_present() -> bool {
        TOOLS.iter().all(|t| crate::plugin::cli::which(t))
    }

    #[test]
    fn with_poppler_a_view_request_yields_text_or_a_jpeg_of_the_wanted_width() {
        if !poppler_present() {
            eprintln!("poppler not on PATH; skipping");
            return;
        }
        let dir = scratch("poppler");
        write_pdf(&dir.join("two.pdf"), 2);
        let entry = entry_with(Some("two.pdf"));
        let st = state(&dir);
        let t = render(&st, &view(Some(entry.clone()), 2, 0, false), &POPPLER).unwrap();
        assert_eq!(t.pages, Some(2));
        assert!(t.image.is_none());
        assert_eq!(t.lines.as_deref(), Some(&["(no text on this page)".to_string()][..]));

        let t = render(&st, &view(Some(entry.clone()), 9, 300, true), &POPPLER).unwrap();
        assert_eq!(t.pages, Some(2));
        let jpg = t.image.expect("jpeg path");
        assert!(jpg.starts_with(cache_dir()), "{}", jpg.display());
        // JPEG: poppler's PNG encoder alone took 0.19s of a 0.21s render (2026-09-15)
        assert!(jpg.to_string_lossy().ends_with("kim2025-2-300.jpg"), "page past the end is clamped: {}", jpg.display());
        let (w, h) = image::image_dimensions(&jpg).unwrap();
        assert_eq!(w, 300, "poppler scales to the exact width, so the host has nothing to resize");
        assert!(h > w, "portrait");
        let before = std::fs::metadata(&jpg).unwrap().modified().unwrap();
        let t = render(&st, &view(Some(entry), 2, 300, true), &POPPLER).unwrap();
        assert_eq!(t.image.as_deref(), Some(jpg.as_path()));
        assert_eq!(std::fs::metadata(&jpg).unwrap().modified().unwrap(), before, "an existing jpeg is reused");
        let _ = std::fs::remove_file(&jpg);
    }

    #[test]
    fn without_poppler_the_error_says_how_to_install_it() {
        let dir = scratch("nopoppler");
        write_pdf(&dir.join("one.pdf"), 1);
        // PATH를 바꾸면 병렬로 도는 다른 테스트의 `which`가 깨진다. 없는 프로그램 이름을 끼운다
        let tools = Tools { pdfinfo: "bibox-test-no-pdfinfo", pdftoppm: "bibox-test-no-pdftoppm", pdftotext: "bibox-test-no-pdftotext" };
        let e = render(&state(&dir), &view(Some(entry_with(Some("one.pdf"))), 1, 0, false), &tools).unwrap_err();
        assert_eq!(e, NEEDS_POPPLER);
    }
}
