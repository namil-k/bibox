//! 플러그인 쪽 루프. bibox 안의 내장 플러그인이 쓰고, 나중에 `bibox-plugin` 크레이트로 나간다.
//! stdin 줄마다 `Request` 하나, 답으로 `Final` 한 줄. 그 사이 `Ui`가 팝업 요청을 주고받는다.

use serde_json::Value;
use std::io::{BufRead, Write};

use crate::plugin::protocol::{Final, Request, UiRequest};

/// 핸들러가 bibox에 팝업을 요청하는 손잡이. 답은 `serde_json::Value`로 읽는다.
/// `UiAnswer`는 `untagged`라 `{}`가 `Index { None }`으로 읽히는 함정이 있어 타입으로 되읽지 않는다.
pub struct Ui<'a> {
    reader: &'a mut dyn BufRead,
    writer: &'a mut dyn Write,
}

impl<'a> Ui<'a> {
    /// 요청 한 줄을 쓰고 답 한 줄을 읽는다. EOF나 깨진 답이면 `None`(호출자가 취소값으로 바꾼다).
    fn ask(&mut self, req: &UiRequest) -> Option<Value> {
        let line = serde_json::to_string(req).ok()?;
        self.writer.write_all(line.as_bytes()).ok()?;
        self.writer.write_all(b"\n").ok()?;
        self.writer.flush().ok()?;
        let mut buf = String::new();
        match self.reader.read_line(&mut buf) {
            Ok(0) | Err(_) => None,
            Ok(_) => serde_json::from_str(buf.trim_end()).ok(),
        }
    }

    // pick/prompt/confirm은 아직 어떤 내장 플러그인도 부르지 않는다. 처음 쓰는 곳이 생기면
    // `#[cfg(test)]`를 지운다(`#[allow(dead_code)]`로 덮지 않는 규칙).
    #[cfg(test)]
    pub fn pick(&mut self, title: &str, items: &[String]) -> Option<usize> {
        let req = UiRequest::Pick { title: Some(title.to_string()), items: items.to_vec() };
        self.ask(&req)?.get("index")?.as_u64().map(|n| n as usize)
    }

    #[cfg(test)]
    pub fn prompt(&mut self, title: &str, default: &str) -> Option<String> {
        let req = UiRequest::Prompt { title: Some(title.to_string()), default: Some(default.to_string()) };
        self.ask(&req)?.get("text")?.as_str().map(str::to_string)
    }

    #[cfg(test)]
    pub fn confirm(&mut self, title: &str) -> bool {
        let req = UiRequest::Confirm { title: Some(title.to_string()) };
        self.ask(&req).and_then(|v| v.get("yes")?.as_bool()).unwrap_or(false)
    }

    pub fn progress(&mut self, text: &str) {
        let _ = self.ask(&UiRequest::Progress { text: text.to_string() });
    }
}

/// 테스트 가능한 루프. 요청 한 줄 -> 핸들러 -> 최종 응답 한 줄. EOF에서 돌아온다.
pub fn serve_io(reader: &mut dyn BufRead, writer: &mut dyn Write, handler: &mut dyn FnMut(&Request, &mut Ui) -> Final) {
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        let text = line.trim_end();
        if text.is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Request>(text) {
            Ok(req) => {
                // 명시적 재빌림. 그냥 `Ui { reader, writer }`라고 쓰면 참조가 이동해 다음 반복에서 못 쓴다.
                let mut ui = Ui { reader: &mut *reader, writer: &mut *writer };
                handler(&req, &mut ui)
            }
            Err(e) => Final { error: Some(format!("bad request: {}", e)), ..Default::default() },
        };
        let out = serde_json::to_string(&response).unwrap_or_else(|_| "{}".to_string());
        if writer.write_all(out.as_bytes()).is_err() || writer.write_all(b"\n").is_err() || writer.flush().is_err() {
            return;
        }
    }
}

/// 실제 진입점. stdin/stdout에 붙인다.
pub fn serve(handler: &mut dyn FnMut(&Request, &mut Ui) -> Final) {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut reader = stdin.lock();
    let mut writer = stdout.lock();
    serve_io(&mut reader, &mut writer, handler);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn request_line(id: &str) -> String {
        format!(
            r#"{{"type":"command","id":"{}","trigger":"key","context":{{"focus":null,"collection":null,"entry":null,"entries":[],"config":{{}},"paths":{{"config_dir":"/c","db":"/c/db.json","notes":"/c/n","pdfs":"/c/p","home":null}},"hook":null}}}}"#,
            id
        )
    }

    #[test]
    fn serve_answers_each_request_with_one_final_line_and_stops_at_eof() {
        let input = format!("{}\n{}\n", request_line("a"), request_line("b"));
        let mut reader = Cursor::new(input.into_bytes());
        let mut out: Vec<u8> = Vec::new();
        let mut handler = |req: &Request, _ui: &mut Ui| Final { message: Some(format!("got {}", req.id)), ..Default::default() };
        serve_io(&mut reader, &mut out, &mut handler);
        let text = String::from_utf8(out).unwrap();
        assert_eq!(text, "{\"message\":\"got a\"}\n{\"message\":\"got b\"}\n");
    }

    #[test]
    fn a_bad_request_line_gets_an_error_and_the_loop_continues() {
        let input = format!("not json\n{}\n", request_line("a"));
        let mut reader = Cursor::new(input.into_bytes());
        let mut out: Vec<u8> = Vec::new();
        let mut handler = |_req: &Request, _ui: &mut Ui| Final::default();
        serve_io(&mut reader, &mut out, &mut handler);
        let lines: Vec<&str> = std::str::from_utf8(&out).unwrap().lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("\"error\"") && lines[0].contains("bad request"));
        assert_eq!(lines[1], "{}");
    }

    #[test]
    fn ui_pick_writes_a_request_and_reads_the_index_back() {
        // 요청 한 줄 뒤에 bibox의 답 한 줄이 이어진다
        let input = format!("{}\n{{\"index\":1}}\n", request_line("a"));
        let mut reader = Cursor::new(input.into_bytes());
        let mut out: Vec<u8> = Vec::new();
        let mut handler = |_req: &Request, ui: &mut Ui| {
            let picked = ui.pick("Style", &["APA".to_string(), "IEEE".to_string()]);
            Final { message: Some(format!("{:?}", picked)), ..Default::default() }
        };
        serve_io(&mut reader, &mut out, &mut handler);
        let text = String::from_utf8(out).unwrap();
        assert_eq!(text, "{\"ui\":\"pick\",\"title\":\"Style\",\"items\":[\"APA\",\"IEEE\"]}\n{\"message\":\"Some(1)\"}\n");
    }

    #[test]
    fn ui_answers_that_are_null_or_missing_become_cancel_values() {
        let input = format!("{}\n{{\"index\":null}}\n{{\"text\":null}}\n{{\"yes\":false}}\n{{}}\n", request_line("a"));
        let mut reader = Cursor::new(input.into_bytes());
        let mut out: Vec<u8> = Vec::new();
        let mut handler = |_req: &Request, ui: &mut Ui| {
            let a = ui.pick("t", &["x".to_string()]);
            let b = ui.prompt("t", "d");
            let c = ui.confirm("t");
            ui.progress("p");
            Final { message: Some(format!("{:?} {:?} {}", a, b, c)), ..Default::default() }
        };
        serve_io(&mut reader, &mut out, &mut handler);
        let last = std::str::from_utf8(&out).unwrap().lines().last().unwrap().to_string();
        assert_eq!(last, "{\"message\":\"None None false\"}");
    }

    #[test]
    fn ui_calls_after_eof_return_cancel_values_instead_of_blocking() {
        let input = format!("{}\n", request_line("a"));
        let mut reader = Cursor::new(input.into_bytes());
        let mut out: Vec<u8> = Vec::new();
        let mut handler = |_req: &Request, ui: &mut Ui| {
            assert_eq!(ui.pick("t", &["x".to_string()]), None);
            assert!(!ui.confirm("t"));
            Final::default()
        };
        serve_io(&mut reader, &mut out, &mut handler);
    }
}
