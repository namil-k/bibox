//! pdf-view 내장 플러그인. 미리보기 패널의 PDF 탭. 호스트가 "n쪽을 W픽셀 폭으로"라고 물으면
//! poppler(`pdfinfo`, `pdftoppm`, `pdftotext`)로 PNG 또는 텍스트를 만든다. 스크롤·배율·캐시는 호스트 일.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::plugin::builtin::Builtin;
use crate::plugin::protocol::{Final, Request, TabResponse};
use crate::plugin::serve::{serve, Ui};

pub const MANIFEST: &str = r#"api = 1
name = "pdf-view"
description = "Show the attached PDF page by page in the preview panel"

[[tabs]]
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

pub const BUILTIN: Builtin = Builtin { name: "pdf-view", manifest: MANIFEST, run };

/// doctor가 PATH에서 찾는 도구들
pub const TOOLS: [&str; 3] = ["pdfinfo", "pdftoppm", "pdftotext"];
pub const NEEDS_POPPLER: &str = "pdf-view needs poppler (pdftoppm, pdfinfo, pdftotext). brew install poppler";
const NOT_A_TAB: &str = "pdf-view renders the PDF tab; open the preview panel";
const NO_TEXT: &str = "(no text on this page)";

fn run() {
    // 지난 세션의 PNG. 호스트 캐시는 항목 하나뿐이라 디스크 것도 오래 둘 이유가 없다
    let _ = std::fs::remove_dir_all(cache_dir());
    let mut handler = |req: &Request, ui: &mut Ui| handle(req, ui);
    serve(&mut handler);
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

fn err(s: impl Into<String>) -> Final {
    Final { error: Some(s.into()), ..Default::default() }
}

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

/// 원하는 픽셀 폭을 dpi로. 쪽 폭 pt를 72로 나눈 것이 인치. 1..=cap.
pub(crate) fn dpi_for(width_px: u32, width_pt: f64, cap: u32) -> u32 {
    if width_px == 0 || width_pt <= 0.0 {
        return 72.min(cap.max(1));
    }
    let dpi = (width_px as f64 * 72.0 / width_pt).round() as u32;
    dpi.clamp(1, cap.max(1))
}

/// `<dir>/<key>-<page>-<width>`. `pdftoppm -singlefile`이 `.png`를 붙인다.
pub(crate) fn png_prefix(dir: &Path, key: &str, page: u32, width_px: u32) -> PathBuf {
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

/// 이미 있으면 다시 만들지 않는다.
fn pdftoppm(tools: &Tools, pdf: &Path, page: u32, dpi: u32, prefix: &Path) -> Result<PathBuf, String> {
    let png = prefix.with_extension("png");
    if png.is_file() {
        return Ok(png);
    }
    if let Some(dir) = prefix.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {}", dir.display(), e))?;
    }
    let p = page.to_string();
    let r = dpi.to_string();
    run_tool(tools.pdftoppm, &["-f", &p, "-l", &p, "-r", &r, "-png", "-singlefile", &pdf.to_string_lossy(), &prefix.to_string_lossy()])?;
    if !png.is_file() {
        return Err(format!("{} wrote nothing at {}", tools.pdftoppm, png.display()));
    }
    Ok(png)
}

// ── 요청 처리 ────────────────────────────────────────────────────────────────

/// 탭 명령은 팝업을 열 수 없어 `Ui`를 쓰지 않는다.
pub(crate) fn handle(req: &Request, _ui: &mut Ui) -> Final {
    render(req, &POPPLER)
}

fn render(req: &Request, tools: &Tools) -> Final {
    if req.trigger != "tab" {
        return Final { message: Some(NOT_A_TAB.to_string()), ..Default::default() };
    }
    let Some(tab) = &req.tab else { return err("no tab request") };
    let Some(entry) = &req.context.entry else { return err("no entry selected") };
    let Some(fp) = &entry.file_path else { return err("no PDF attached") };
    let pdf = req.context.paths.pdfs.join(fp);
    if !pdf.is_file() {
        return err(format!("PDF not found: {}", pdf.display()));
    }
    let info = match pdfinfo(tools, &pdf) {
        Ok(i) => i,
        Err(e) => return err(e),
    };
    let page = tab.page.clamp(1, info.pages.max(1));
    let response = if !tab.images {
        match pdftotext(tools, &pdf, page) {
            Ok(lines) => TabResponse { lines: Some(lines), pages: Some(info.pages), ..Default::default() },
            Err(e) => return err(e),
        }
    } else {
        let cap = req.context.config.get("dpi_cap").and_then(|v| v.as_u64()).map(|v| v as u32).unwrap_or(300);
        let dpi = dpi_for(tab.width_px, info.width_pt, cap);
        let prefix = png_prefix(&cache_dir(), &entry.bibtex_key, page, tab.width_px);
        match pdftoppm(tools, &pdf, page, dpi, &prefix) {
            Ok(png) => TabResponse { image: Some(png), pages: Some(info.pages), ..Default::default() },
            Err(e) => return err(e),
        }
    };
    Final { tab: Some(response), ..Default::default() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::protocol::{Context, Paths, TabRequest};

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
    fn dpi_follows_the_wanted_width_and_stops_at_the_cap() {
        // 612pt = 8.5in. 850px wide -> 100dpi
        assert_eq!(dpi_for(850, 612.0, 300), 100);
        assert_eq!(dpi_for(8500, 612.0, 300), 300);
        assert_eq!(dpi_for(0, 612.0, 300), 72, "zero width is not a division by zero");
        assert_eq!(dpi_for(850, 0.0, 300), 72, "zero page width either");
        assert!(dpi_for(10, 612.0, 300) >= 1);
    }

    #[test]
    fn png_prefix_is_per_entry_page_and_width() {
        let p = png_prefix(Path::new("/t"), "kim2025", 3, 840);
        assert_eq!(p, Path::new("/t/kim2025-3-840"));
        let p = png_prefix(Path::new("/t"), "we/ird key", 1, 1);
        assert_eq!(p, Path::new("/t/we_ird_key-1-1"), "keys never make directories");
    }

    fn request(trigger: &str, entry: Option<crate::models::Entry>, tab: Option<TabRequest>, pdfs: &Path) -> Request {
        Request {
            r#type: "command".into(),
            id: "render".into(),
            trigger: trigger.into(),
            context: Context {
                focus: None,
                collection: None,
                entry,
                entries: vec![],
                config: serde_json::json!({}),
                paths: Paths { config_dir: "/c".into(), db: "/h/db.json".into(), notes: "/h/notes".into(), pdfs: pdfs.to_path_buf(), home: None },
                hook: None,
            },
            tab,
        }
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
    fn a_key_binding_is_told_to_open_the_preview_and_missing_pdfs_are_errors() {
        let f = render(&request("key", None, None, Path::new("/p")), &POPPLER);
        assert_eq!(f.message.as_deref(), Some("pdf-view renders the PDF tab; open the preview panel"));
        let tab = TabRequest { page: 1, width_px: 0, images: false };
        let f = render(&request("tab", None, Some(tab.clone()), Path::new("/p")), &POPPLER);
        assert_eq!(f.error.as_deref(), Some("no entry selected"));
        let f = render(&request("tab", Some(entry_with(None)), Some(tab.clone()), Path::new("/p")), &POPPLER);
        assert_eq!(f.error.as_deref(), Some("no PDF attached"));
        let f = render(&request("tab", Some(entry_with(Some("gone.pdf"))), Some(tab), Path::new("/p")), &POPPLER);
        assert!(f.error.as_deref().unwrap().starts_with("PDF not found: /p/gone.pdf"), "{:?}", f.error);
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
    fn with_poppler_a_tab_request_yields_text_or_a_png_of_about_the_wanted_width() {
        if !poppler_present() {
            eprintln!("poppler not on PATH; skipping");
            return;
        }
        let dir = scratch("poppler");
        write_pdf(&dir.join("two.pdf"), 2);
        let entry = entry_with(Some("two.pdf"));
        let f = render(&request("tab", Some(entry.clone()), Some(TabRequest { page: 2, width_px: 0, images: false }), &dir), &POPPLER);
        let t = f.tab.expect("tab response");
        assert_eq!(t.pages, Some(2));
        assert!(t.image.is_none());
        assert_eq!(t.lines.as_deref(), Some(&["(no text on this page)".to_string()][..]));

        let f = render(&request("tab", Some(entry.clone()), Some(TabRequest { page: 9, width_px: 300, images: true }), &dir), &POPPLER);
        let t = f.tab.expect("tab response");
        assert_eq!(t.pages, Some(2));
        let png = t.image.expect("png path");
        assert!(png.starts_with(cache_dir()), "{}", png.display());
        assert!(png.to_string_lossy().ends_with("kim2025-2-300.png"), "page past the end is clamped: {}", png.display());
        let (w, h) = image::image_dimensions(&png).unwrap();
        assert!((295..=305).contains(&w), "width {}", w);
        assert!(h > w, "portrait");
        let before = std::fs::metadata(&png).unwrap().modified().unwrap();
        let f = render(&request("tab", Some(entry), Some(TabRequest { page: 2, width_px: 300, images: true }), &dir), &POPPLER);
        assert_eq!(f.tab.unwrap().image.as_deref(), Some(png.as_path()));
        assert_eq!(std::fs::metadata(&png).unwrap().modified().unwrap(), before, "an existing png is reused");
        let _ = std::fs::remove_file(&png);
    }

    #[test]
    fn without_poppler_the_error_says_how_to_install_it() {
        let dir = scratch("nopoppler");
        write_pdf(&dir.join("one.pdf"), 1);
        // PATH를 바꾸면 병렬로 도는 다른 테스트의 `which`가 깨진다. 없는 프로그램 이름을 끼운다
        let tools = Tools { pdfinfo: "bibox-test-no-pdfinfo", pdftoppm: "bibox-test-no-pdftoppm", pdftotext: "bibox-test-no-pdftotext" };
        let f = render(&request("tab", Some(entry_with(Some("one.pdf"))), Some(TabRequest { page: 1, width_px: 0, images: false }), &dir), &tools);
        assert_eq!(f.error.as_deref(), Some(NEEDS_POPPLER));
    }
}
