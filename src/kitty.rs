//! kitty 그래픽 프로토콜의 유니코드 플레이스홀더 경로. ratatui-image의 kitty는 압축 없는 RGBA를 보내
//! 2240px 쪽 하나가 37.7MB(메인 스레드 정지 약 115ms)였다. 여기서는 RGB를 zlib로 눌러 보내고(글자 쪽은 20배 작다),
//! 인코딩은 워커 스레드에서 끝낸다. 메인은 완성된 문자열을 한 번 쓰고, 그 뒤 스크롤·pan은 플레이스홀더의
//! 행·열 diacritic만 바꾼다. 플레이스홀더 방식과 diacritic 표는 ratatui-image 11(MIT)에서 가져왔다.
//!
//! 한계: diacritic이 297개라 한 쪽에서 행·열 296까지만 가리킬 수 있다. `dpi_cap` 300의 A4는 150열·200행 안팎.

use std::fmt::Write as _;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

use base64::Engine as _;

/// 플레이스홀더 글자. 이 글자에 붙은 diacritic이 이미지의 (행, 열, id 상위 바이트)를 가리킨다.
pub const PLACEHOLDER: char = '\u{10EEEE}';
/// 한 청크의 base64 최대 길이(프로토콜 규정).
const CHUNK: usize = 4096;

static NEXT_ID: AtomicU32 = AtomicU32::new(1);

/// 이 프로세스에서 아직 안 쓴 이미지 id. 24비트 안에서 돈다(플레이스홀더의 fg 색 세 바이트).
pub fn next_id() -> u32 {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    (id % 0xFF_FFFF).max(1)
}

/// 터미널에 올릴 쪽 하나. `transmit`은 처음 그릴 때 한 번만 꺼내 쓴다.
pub struct Page {
    pub id: u32,
    pub width: u32,
    pub height: u32,
    transmit: Mutex<Option<String>>,
}

impl Page {
    /// 전송 시퀀스. 처음 한 번만 Some.
    pub fn take_transmit(&self) -> Option<String> {
        self.transmit.lock().unwrap_or_else(|p| p.into_inner()).take()
    }

    /// 플레이스홀더 한 줄. 이미지의 `row`행을 `pan_cols`열부터 `cols`칸. 셀 하나에 통째로 들어가며
    /// 끝의 커서 복원은 ratatui가 기대하는 자리(오른쪽 아래 구석)로 되돌린다.
    pub fn row(&self, row: u16, pan_cols: u16, cols: u16, restore_right: u16, restore_down: u16) -> String {
        let [id_extra, r, g, b] = self.id.to_be_bytes();
        let mut s = String::with_capacity(64 + cols as usize * 4);
        let _ = write!(s, "\x1b[s\x1b[38;2;{};{};{}m{}{}{}{}", r, g, b, PLACEHOLDER, diacritic(row), diacritic(pan_cols), diacritic(u16::from(id_extra)));
        for _ in 1..cols {
            s.push(PLACEHOLDER);
        }
        let _ = write!(s, "\x1b[u\x1b[{}C\x1b[{}B", restore_right, restore_down);
        s
    }
}

/// 그림을 zlib RGB로 눌러 4096자 청크의 전송 시퀀스로. 가상 배치(U=1)라 화면에는 아무것도 안 나오고,
/// 플레이스홀더가 놓인 칸에만 보인다. 워커 스레드에서 부른다.
pub fn encode(img: &image::DynamicImage, id: u32) -> Page {
    let rgb = img.to_rgb8();
    let (width, height) = (rgb.width(), rgb.height());
    let mut z = flate2::write::ZlibEncoder::new(Vec::with_capacity(rgb.len() / 8), flate2::Compression::fast());
    let _ = std::io::Write::write_all(&mut z, rgb.as_raw());
    let compressed = z.finish().unwrap_or_default();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&compressed);
    let chunks: Vec<&[u8]> = b64.as_bytes().chunks(CHUNK).collect();
    let mut out = String::with_capacity(b64.len() + chunks.len() * 40 + 64);
    for (i, chunk) in chunks.iter().enumerate() {
        let more = u8::from(i + 1 < chunks.len());
        if i == 0 {
            let _ = write!(out, "\x1b_Gq=2,i={},a=T,U=1,f=24,o=z,t=d,s={},v={},m={};", id, width, height, more);
        } else {
            let _ = write!(out, "\x1b_Gq=2,m={};", more);
        }
        out.push_str(std::str::from_utf8(chunk).unwrap_or(""));
        out.push_str("\x1b\\");
    }
    Page { id, width, height, transmit: Mutex::new(Some(out)) }
}

/// 이미지와 그 배치를 터미널에서 지운다. 캐시에서 빠질 때와 종료 때.
pub fn delete(id: u32) -> String {
    format!("\x1b_Gq=2,a=d,d=I,i={}\x1b\\", id)
}

/// 행·열 번호를 diacritic으로. 표 밖(297 이상)은 마지막 것으로 눌러 둔다.
pub fn diacritic(i: u16) -> char {
    DIACRITICS[usize::from(i).min(DIACRITICS.len() - 1)]
}

/// kitty 규정 표(rowcolumn-diacritics.txt), ratatui-image 11에서 가져옴.
static DIACRITICS: [char; 297] = [
    '\u{305}', '\u{30D}', '\u{30E}', '\u{310}', '\u{312}', '\u{33D}', '\u{33E}', '\u{33F}', '\u{346}', '\u{34A}', '\u{34B}', '\u{34C}',
    '\u{350}', '\u{351}', '\u{352}', '\u{357}', '\u{35B}', '\u{363}', '\u{364}', '\u{365}', '\u{366}', '\u{367}', '\u{368}', '\u{369}',
    '\u{36A}', '\u{36B}', '\u{36C}', '\u{36D}', '\u{36E}', '\u{36F}', '\u{483}', '\u{484}', '\u{485}', '\u{486}', '\u{487}', '\u{592}',
    '\u{593}', '\u{594}', '\u{595}', '\u{597}', '\u{598}', '\u{599}', '\u{59C}', '\u{59D}', '\u{59E}', '\u{59F}', '\u{5A0}', '\u{5A1}',
    '\u{5A8}', '\u{5A9}', '\u{5AB}', '\u{5AC}', '\u{5AF}', '\u{5C4}', '\u{610}', '\u{611}', '\u{612}', '\u{613}', '\u{614}', '\u{615}',
    '\u{616}', '\u{617}', '\u{657}', '\u{658}', '\u{659}', '\u{65A}', '\u{65B}', '\u{65D}', '\u{65E}', '\u{6D6}', '\u{6D7}', '\u{6D8}',
    '\u{6D9}', '\u{6DA}', '\u{6DB}', '\u{6DC}', '\u{6DF}', '\u{6E0}', '\u{6E1}', '\u{6E2}', '\u{6E4}', '\u{6E7}', '\u{6E8}', '\u{6EB}',
    '\u{6EC}', '\u{730}', '\u{732}', '\u{733}', '\u{735}', '\u{736}', '\u{73A}', '\u{73D}', '\u{73F}', '\u{740}', '\u{741}', '\u{743}',
    '\u{745}', '\u{747}', '\u{749}', '\u{74A}', '\u{7EB}', '\u{7EC}', '\u{7ED}', '\u{7EE}', '\u{7EF}', '\u{7F0}', '\u{7F1}', '\u{7F3}',
    '\u{816}', '\u{817}', '\u{818}', '\u{819}', '\u{81B}', '\u{81C}', '\u{81D}', '\u{81E}', '\u{81F}', '\u{820}', '\u{821}', '\u{822}',
    '\u{823}', '\u{825}', '\u{826}', '\u{827}', '\u{829}', '\u{82A}', '\u{82B}', '\u{82C}', '\u{82D}', '\u{951}', '\u{953}', '\u{954}',
    '\u{F82}', '\u{F83}', '\u{F86}', '\u{F87}', '\u{135D}', '\u{135E}', '\u{135F}', '\u{17DD}', '\u{193A}', '\u{1A17}', '\u{1A75}', '\u{1A76}',
    '\u{1A77}', '\u{1A78}', '\u{1A79}', '\u{1A7A}', '\u{1A7B}', '\u{1A7C}', '\u{1B6B}', '\u{1B6D}', '\u{1B6E}', '\u{1B6F}', '\u{1B70}', '\u{1B71}',
    '\u{1B72}', '\u{1B73}', '\u{1CD0}', '\u{1CD1}', '\u{1CD2}', '\u{1CDA}', '\u{1CDB}', '\u{1CE0}', '\u{1DC0}', '\u{1DC1}', '\u{1DC3}', '\u{1DC4}',
    '\u{1DC5}', '\u{1DC6}', '\u{1DC7}', '\u{1DC8}', '\u{1DC9}', '\u{1DCB}', '\u{1DCC}', '\u{1DD1}', '\u{1DD2}', '\u{1DD3}', '\u{1DD4}', '\u{1DD5}',
    '\u{1DD6}', '\u{1DD7}', '\u{1DD8}', '\u{1DD9}', '\u{1DDA}', '\u{1DDB}', '\u{1DDC}', '\u{1DDD}', '\u{1DDE}', '\u{1DDF}', '\u{1DE0}', '\u{1DE1}',
    '\u{1DE2}', '\u{1DE3}', '\u{1DE4}', '\u{1DE5}', '\u{1DE6}', '\u{1DFE}', '\u{20D0}', '\u{20D1}', '\u{20D4}', '\u{20D5}', '\u{20D6}', '\u{20D7}',
    '\u{20DB}', '\u{20DC}', '\u{20E1}', '\u{20E7}', '\u{20E9}', '\u{20F0}', '\u{2CEF}', '\u{2CF0}', '\u{2CF1}', '\u{2DE0}', '\u{2DE1}', '\u{2DE2}',
    '\u{2DE3}', '\u{2DE4}', '\u{2DE5}', '\u{2DE6}', '\u{2DE7}', '\u{2DE8}', '\u{2DE9}', '\u{2DEA}', '\u{2DEB}', '\u{2DEC}', '\u{2DED}', '\u{2DEE}',
    '\u{2DEF}', '\u{2DF0}', '\u{2DF1}', '\u{2DF2}', '\u{2DF3}', '\u{2DF4}', '\u{2DF5}', '\u{2DF6}', '\u{2DF7}', '\u{2DF8}', '\u{2DF9}', '\u{2DFA}',
    '\u{2DFB}', '\u{2DFC}', '\u{2DFD}', '\u{2DFE}', '\u{2DFF}', '\u{A66F}', '\u{A67C}', '\u{A67D}', '\u{A6F0}', '\u{A6F1}', '\u{A8E0}', '\u{A8E1}',
    '\u{A8E2}', '\u{A8E3}', '\u{A8E4}', '\u{A8E5}', '\u{A8E6}', '\u{A8E7}', '\u{A8E8}', '\u{A8E9}', '\u{A8EA}', '\u{A8EB}', '\u{A8EC}', '\u{A8ED}',
    '\u{A8EE}', '\u{A8EF}', '\u{A8F0}', '\u{A8F1}', '\u{AAB0}', '\u{AAB2}', '\u{AAB3}', '\u{AAB7}', '\u{AAB8}', '\u{AABE}', '\u{AABF}', '\u{AAC1}',
    '\u{FE20}', '\u{FE21}', '\u{FE22}', '\u{FE23}', '\u{FE24}', '\u{FE25}', '\u{FE26}', '\u{10A0F}', '\u{10A38}', '\u{1D185}', '\u{1D186}', '\u{1D187}',
    '\u{1D188}', '\u{1D189}', '\u{1D1AA}', '\u{1D1AB}', '\u{1D1AC}', '\u{1D1AD}', '\u{1D242}', '\u{1D243}', '\u{1D244}',
];

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(w: u32, h: u32) -> image::DynamicImage {
        image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(w, h, |x, y| image::Rgb([(x * 5) as u8, (y * 7) as u8, 128])))
    }

    /// 글자 쪽 흉내: 흰 바탕에 검은 줄
    fn text_like(w: u32, h: u32) -> image::DynamicImage {
        image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(w, h, |_, y| if y % 9 == 4 { image::Rgb([0, 0, 0]) } else { image::Rgb([255, 255, 255]) }))
    }

    /// 청크마다 APC 하나. 첫 청크에 형식·크기·압축 표시, 마지막에 m=0. 페이로드를 이어 풀면 원래 픽셀.
    #[test]
    fn a_transmit_is_chunked_zlib_rgb_that_inflates_back_to_the_pixels() {
        let img = gradient(64, 40);
        let page = encode(&img, 7);
        assert_eq!((page.id, page.width, page.height), (7, 64, 40));
        let t = page.take_transmit().expect("first take");
        assert!(page.take_transmit().is_none(), "sent once");
        let chunks: Vec<&str> = t.split("\x1b\\").filter(|c| !c.is_empty()).collect();
        assert!(chunks[0].starts_with("\x1b_Gq=2,i=7,a=T,U=1,f=24,o=z,t=d,s=64,v=40,m="), "{}", &chunks[0][..60]);
        let mut payload = String::new();
        for (i, c) in chunks.iter().enumerate() {
            let (head, data) = c.split_once(';').unwrap();
            assert!(head.starts_with("\x1b_G"));
            assert!(data.len() <= CHUNK, "chunk {} has {} chars", i, data.len());
            let last = i + 1 == chunks.len();
            assert!(head.ends_with(if last { "m=0" } else { "m=1" }), "chunk {}: {}", i, head);
            payload.push_str(data);
        }
        let z = base64::engine::general_purpose::STANDARD.decode(payload).unwrap();
        let mut raw = Vec::new();
        std::io::Read::read_to_end(&mut flate2::read::ZlibDecoder::new(&z[..]), &mut raw).unwrap();
        assert_eq!(raw, img.to_rgb8().into_raw());
    }

    /// 글자 쪽은 raw의 몇 % 로 줄어든다. 2240px 쪽이 37.7MB에서 2MB 안팎이 되는 근거.
    #[test]
    fn a_text_like_page_compresses_far_below_raw() {
        let img = text_like(640, 400);
        let t = encode(&img, 1).take_transmit().unwrap();
        let raw = 640 * 400 * 3;
        assert!(t.len() * 20 < raw, "{} bytes for {} raw", t.len(), raw);
    }

    /// 플레이스홀더 첫 칸이 (행, 열, id 상위 바이트)를 가리키고 나머지 칸은 물려받는다.
    #[test]
    fn a_placeholder_row_points_at_the_page_row_and_the_pan_column() {
        let page = encode(&gradient(8, 8), 0x0102_0304);
        let s = page.row(5, 2, 4, 3, 9);
        let expect_head = format!("\x1b[s\x1b[38;2;2;3;4m{}{}{}{}", PLACEHOLDER, diacritic(5), diacritic(2), diacritic(1));
        assert!(s.starts_with(&expect_head), "{:?}", s);
        assert_eq!(s.matches(PLACEHOLDER).count(), 4, "one placeholder per column");
        assert!(s.ends_with("\x1b[u\x1b[3C\x1b[9B"));
    }

    #[test]
    fn delete_names_the_image_and_ids_stay_within_24_bits() {
        assert_eq!(delete(42), "\x1b_Gq=2,a=d,d=I,i=42\x1b\\");
        let a = next_id();
        let b = next_id();
        assert!(a != b && a > 0 && b < 0x100_0000);
        assert_ne!(diacritic(0), diacritic(1));
        assert_eq!(diacritic(296), diacritic(1000), "past the table sticks to the last one");
    }
}
