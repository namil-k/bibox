//! 플러그인 미리보기 탭의 상태. 쪽·배율·스크롤·pan과 캐시, 창 계산은 여기(순수), 그리기와
//! 플러그인 호출은 tui.rs. 플러그인은 "n쪽을 W픽셀 폭으로"만 안다.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::plugin::TabResponse;

/// 스크롤이 "이전 쪽의 바닥"을 뜻하는 자리. 이미지 높이를 알게 되면 `clamp`가 실제 값으로 바꾼다.
pub const BOTTOM: u32 = u32::MAX;
pub const ZOOM_STEP: u32 = 25;
pub const ZOOM_MIN: u32 = 25;

/// 패널 안쪽 크기(칸)와 칸의 픽셀 크기. `rows`는 상태 줄을 뺀 것.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Viewport {
    pub cols: u16,
    pub rows: u16,
    pub cell_w: u16,
    pub cell_h: u16,
}

impl Viewport {
    pub fn view_w(&self) -> u32 { self.cols as u32 * self.cell_w as u32 }
    pub fn view_h(&self) -> u32 { self.rows as u32 * self.cell_h as u32 }
}

/// 요청할 이미지 폭. 100%가 패널 폭.
pub fn width_px(vp: &Viewport, zoom_pct: u32) -> u32 {
    vp.view_w() * zoom_pct / 100
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// 이미지에서 보이는 부분. 이미지보다 창이 크면 이미지 전체, 스크롤·pan은 끝에서 멈춘다.
pub fn window(vp: &Viewport, img_w: u32, img_h: u32, scroll_px: u32, pan_px: u32) -> Window {
    let w = img_w.min(vp.view_w());
    let h = img_h.min(vp.view_h());
    let x = pan_px.min(img_w.saturating_sub(w));
    let y = scroll_px.min(img_h.saturating_sub(h));
    Window { x, y, w, h }
}

/// 플러그인이 준 이미지를 요청한 폭에 정확히 맞춘다. dpi 반올림으로 몇 픽셀 어긋난 것을 바로잡는다.
pub fn fit_width(img: image::DynamicImage, width_px: u32) -> image::DynamicImage {
    if width_px == 0 || img.width() == width_px || img.width() == 0 {
        return img;
    }
    let h = (img.height() as u64 * width_px as u64 / img.width() as u64).max(1) as u32;
    img.resize_exact(width_px, h, image::imageops::FilterType::Triangle)
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CacheKey {
    pub entry_key: String,
    pub page: u32,
    pub width_px: u32,
}

pub enum Content {
    Image(image::DynamicImage),
    Lines(Vec<String>),
}

#[derive(Default)]
pub struct Cache {
    map: HashMap<CacheKey, Content>,
}

impl Cache {
    pub fn get(&self, k: &CacheKey) -> Option<&Content> { self.map.get(k) }
    pub fn insert(&mut self, k: CacheKey, v: Content) { self.map.insert(k, v); }
    /// 항목이 바뀌면 그 항목 것만 남긴다.
    pub fn retain_entry(&mut self, key: &str) { self.map.retain(|k, _| k.entry_key == key); }
}

/// 플러그인 응답에서 내용의 출처와 쪽수. image가 lines보다 우선.
#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    Image(PathBuf),
    Lines(Vec<String>),
}

pub fn parse_response(tab: Option<TabResponse>) -> Result<(Source, u32), String> {
    let Some(t) = tab else { return Err("empty tab response".to_string()) };
    let pages = t.pages.unwrap_or(1).max(1);
    if let Some(p) = t.image {
        return Ok((Source::Image(p), pages));
    }
    if let Some(l) = t.lines {
        return Ok((Source::Lines(l), pages));
    }
    Err("empty tab response".to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabState {
    pub entry_key: Option<String>,
    pub page: u32,
    pub pages: u32,
    pub zoom_pct: u32,
    /// 이미지 모드는 픽셀, 텍스트 모드는 줄
    pub scroll: u32,
    pub pan: u32,
    pub pending: Option<CacheKey>,
    pub error: Option<String>,
}

impl Default for TabState {
    fn default() -> Self { Self::new() }
}

impl TabState {
    pub fn new() -> Self {
        TabState { entry_key: None, page: 1, pages: 1, zoom_pct: 100, scroll: 0, pan: 0, pending: None, error: None }
    }

    /// 항목이 바뀌면 첫 쪽 꼭대기. 배율은 유지.
    pub fn on_entry(&mut self, key: Option<&str>) {
        if self.entry_key.as_deref() == key {
            return;
        }
        self.entry_key = key.map(str::to_string);
        self.page = 1;
        self.pages = 1;
        self.scroll = 0;
        self.pan = 0;
        self.pending = None;
        self.error = None;
    }

    pub fn set_pages(&mut self, n: u32) {
        self.pages = n.max(1);
        if self.page > self.pages {
            self.page = self.pages;
            self.scroll = 0;
        }
    }

    /// 바닥에서 한 번 더 누르면 다음 쪽. 넘겼으면 true.
    pub fn scroll_down(&mut self, step: u32, extent: u32, view: u32) -> bool {
        let max = extent.saturating_sub(view);
        if self.scroll >= max {
            if self.page < self.pages {
                self.go(self.page + 1);
                return true;
            }
            self.scroll = max;
            return false;
        }
        self.scroll = (self.scroll + step).min(max);
        false
    }

    /// 꼭대기에서 한 번 더 누르면 이전 쪽의 바닥(`BOTTOM`). 넘겼으면 true.
    pub fn scroll_up(&mut self, step: u32, _view: u32) -> bool {
        if self.scroll == 0 {
            if self.page > 1 {
                self.go(self.page - 1);
                self.scroll = BOTTOM;
                return true;
            }
            return false;
        }
        self.scroll = self.scroll.saturating_sub(step);
        false
    }

    /// 쪽을 옮기면 꼭대기로, 그리고 지난 오류를 지운다(다시 요청할 기회).
    fn go(&mut self, page: u32) {
        self.page = page;
        self.scroll = 0;
        self.error = None;
    }

    pub fn next_page(&mut self) -> bool {
        if self.page >= self.pages { return false; }
        self.go(self.page + 1);
        true
    }

    pub fn prev_page(&mut self) -> bool {
        if self.page <= 1 { return false; }
        self.go(self.page - 1);
        true
    }

    pub fn first_page(&mut self) { self.go(1); }
    pub fn last_page(&mut self) { self.go(self.pages); }

    pub fn zoom(&mut self, delta: i32, max: u32) {
        let z = self.zoom_pct as i64 + delta as i64;
        self.zoom_pct = z.clamp(ZOOM_MIN as i64, max.max(ZOOM_MIN) as i64) as u32;
        self.error = None;
    }

    pub fn zoom_reset(&mut self) { self.zoom_pct = 100; self.pan = 0; self.error = None; }

    pub fn pan(&mut self, delta: i32, extent: u32, view: u32) {
        let max = extent.saturating_sub(view) as i64;
        self.pan = (self.pan as i64 + delta as i64).clamp(0, max) as u32;
    }

    /// 내용 크기를 알게 된 뒤 스크롤과 pan을 범위 안으로. `BOTTOM`은 실제 바닥으로.
    pub fn clamp(&mut self, extent: u32, view: u32, pan_extent: u32, pan_view: u32) {
        self.scroll = self.scroll.min(extent.saturating_sub(view));
        self.pan = self.pan.min(pan_extent.saturating_sub(pan_view));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vp() -> Viewport {
        Viewport { cols: 80, rows: 30, cell_w: 10, cell_h: 20 }
    }

    #[test]
    fn width_follows_the_panel_and_the_zoom() {
        assert_eq!(width_px(&vp(), 100), 800);
        assert_eq!(width_px(&vp(), 125), 1000);
        assert_eq!(width_px(&vp(), 50), 400);
    }

    #[test]
    fn the_window_is_clamped_to_the_image() {
        // 800x1200 이미지, 창 800x600
        let w = window(&vp(), 800, 1200, 0, 0);
        assert_eq!((w.x, w.y, w.w, w.h), (0, 0, 800, 600));
        let w = window(&vp(), 800, 1200, 900, 0);
        assert_eq!(w.y, 600, "scroll past the end stops at the bottom");
        let w = window(&vp(), 800, 400, 50, 0);
        assert_eq!((w.y, w.h), (0, 400), "a short page shows whole and ignores scroll");
        let w = window(&vp(), 1000, 1200, 0, 500);
        assert_eq!((w.x, w.w), (200, 800), "pan stops at the right edge");
        let w = window(&vp(), 600, 1200, 0, 100);
        assert_eq!((w.x, w.w), (0, 600), "a narrow page never pans");
    }

    #[test]
    fn scrolling_down_flips_the_page_only_at_the_bottom() {
        let mut s = TabState::new();
        s.set_pages(3);
        assert!(!s.scroll_down(60, 1200, 600));
        assert_eq!(s.scroll, 60);
        for _ in 0..9 { assert!(!s.scroll_down(60, 1200, 600)); }
        assert_eq!((s.page, s.scroll), (1, 600), "stops at the bottom first");
        assert!(s.scroll_down(60, 1200, 600), "one more press flips");
        assert_eq!((s.page, s.scroll), (2, 0));
        s.scroll = 570;
        assert!(!s.scroll_down(60, 1200, 600));
        assert_eq!(s.scroll, 600, "a step past the bottom lands on the bottom, not the next page");
        s.page = 3;
        s.scroll = 600;
        assert!(!s.scroll_down(60, 1200, 600), "last page has nowhere to go");
        assert_eq!((s.page, s.scroll), (3, 600));
    }

    #[test]
    fn scrolling_up_flips_to_the_bottom_of_the_previous_page() {
        let mut s = TabState::new();
        s.set_pages(3);
        s.page = 2;
        s.scroll = 100;
        assert!(!s.scroll_up(60, 600));
        assert_eq!(s.scroll, 40);
        s.scroll_up(60, 600);
        assert_eq!(s.scroll, 0);
        assert!(s.scroll_up(60, 600), "at the top, flips");
        assert_eq!((s.page, s.scroll), (1, BOTTOM));
        s.clamp(1200, 600, 800, 800);
        assert_eq!(s.scroll, 600, "BOTTOM becomes the real bottom once the image is known");
        assert!(!s.scroll_up(60, 600) || s.page == 1);
        s.scroll = 0;
        assert!(!s.scroll_up(60, 600), "first page top stays");
    }

    #[test]
    fn page_keys_and_zoom_and_pan() {
        let mut s = TabState::new();
        s.set_pages(14);
        assert!(s.next_page());
        assert_eq!((s.page, s.scroll), (2, 0));
        s.scroll = 300;
        assert!(s.prev_page());
        assert_eq!((s.page, s.scroll), (1, 0));
        assert!(!s.prev_page());
        s.last_page();
        assert_eq!(s.page, 14);
        assert!(!s.next_page());
        s.first_page();
        assert_eq!(s.page, 1);
        s.zoom(25, 400);
        assert_eq!(s.zoom_pct, 125);
        for _ in 0..20 { s.zoom(25, 400); }
        assert_eq!(s.zoom_pct, 400);
        for _ in 0..30 { s.zoom(-25, 400); }
        assert_eq!(s.zoom_pct, 25);
        s.pan = 40;
        s.zoom_reset();
        assert_eq!((s.zoom_pct, s.pan), (100, 0));
        s.pan(80, 1000, 800);
        assert_eq!(s.pan, 80);
        s.pan(800, 1000, 800);
        assert_eq!(s.pan, 200, "clamped to the overflow");
        s.pan(-999, 1000, 800);
        assert_eq!(s.pan, 0);
    }

    #[test]
    fn a_new_entry_resets_position_but_keeps_zoom_and_pages_clamp() {
        let mut s = TabState::new();
        s.set_pages(14);
        s.page = 9;
        s.scroll = 100;
        s.pan = 30;
        s.zoom_pct = 150;
        s.on_entry(Some("a"));
        assert_eq!((s.page, s.scroll, s.pan, s.zoom_pct, s.pages), (1, 0, 0, 150, 1));
        assert_eq!(s.entry_key.as_deref(), Some("a"));
        s.on_entry(Some("a"));
        s.page = 5;
        s.set_pages(3);
        assert_eq!(s.page, 3, "pages shrinking pulls the page back");
    }

    #[test]
    fn responses_prefer_image_and_reject_empty_ones() {
        use crate::plugin::TabResponse;
        let (src, pages) = parse_response(Some(TabResponse { image: Some("/tmp/a.png".into()), lines: Some(vec!["x".into()]), pages: Some(4) })).unwrap();
        assert!(matches!(src, Source::Image(p) if p == PathBuf::from("/tmp/a.png")));
        assert_eq!(pages, 4);
        let (src, pages) = parse_response(Some(TabResponse { image: None, lines: Some(vec!["x".into()]), pages: None })).unwrap();
        assert!(matches!(src, Source::Lines(l) if l == vec!["x".to_string()]));
        assert_eq!(pages, 1);
        assert!(parse_response(Some(TabResponse::default())).is_err());
        assert!(parse_response(None).is_err());
    }

    #[test]
    fn the_cache_keeps_only_the_current_entry() {
        let mut c = Cache::default();
        c.insert(CacheKey { entry_key: "a".into(), page: 1, width_px: 800 }, Content::Lines(vec![]));
        c.insert(CacheKey { entry_key: "b".into(), page: 1, width_px: 800 }, Content::Lines(vec![]));
        c.retain_entry("b");
        assert!(c.get(&CacheKey { entry_key: "a".into(), page: 1, width_px: 800 }).is_none());
        assert!(c.get(&CacheKey { entry_key: "b".into(), page: 1, width_px: 800 }).is_some());
    }

    #[test]
    fn fit_width_scales_to_the_exact_width_and_keeps_the_ratio() {
        let img = image::DynamicImage::new_rgb8(200, 400);
        let out = fit_width(img, 100);
        assert_eq!((out.width(), out.height()), (100, 200));
        let same = fit_width(image::DynamicImage::new_rgb8(100, 50), 100);
        assert_eq!((same.width(), same.height()), (100, 50), "already right: untouched");
        let zero = fit_width(image::DynamicImage::new_rgb8(10, 10), 0);
        assert_eq!(zero.width(), 10, "width 0 means no resize");
    }
}
