//! 내장 플러그인이 `bibox plugin run <name>`으로 돌 때의 JSON-RPC 서버 루프. 파이썬 헬퍼와 같은 규칙:
//! initialize/shutdown은 루프가, 나머지 요청은 핸들러가, Ui의 요청은 답이 올 때까지 읽고 그 사이 줄은 뒤로 미룬다.

use std::collections::VecDeque;
use std::io::{BufRead, Write};

use serde_json::Value;

use crate::plugin::protocol::{FieldsMap, InitializeResult, PROTOCOL};
use crate::plugin::rpc::{self, Message, RpcError};

pub struct Ui<'a> {
    reader: &'a mut dyn BufRead,
    writer: &'a mut dyn Write,
    deferred: &'a mut VecDeque<Message>,
    next_id: &'a mut u64,
}

impl<'a> Ui<'a> {
    fn send(&mut self, msg: &Message) -> bool {
        let line = rpc::to_line(msg);
        self.writer.write_all(line.as_bytes()).is_ok() && self.writer.write_all(b"\n").is_ok() && self.writer.flush().is_ok()
    }

    fn ask(&mut self, method: &str, params: Value) -> Option<Value> {
        let id = *self.next_id;
        *self.next_id += 1;
        if !self.send(&Message::Request { id, method: method.to_string(), params }) { return None; }
        let mut buf = String::new();
        loop {
            buf.clear();
            match self.reader.read_line(&mut buf) {
                Ok(0) | Err(_) => return None,
                Ok(_) => {}
            }
            match rpc::parse_line(buf.trim_end()) {
                Ok(Message::Response { id: got, result }) if got == id => return Some(result),
                Ok(Message::Error { id: Some(got), .. }) if got == id => return None,
                Ok(other) => self.deferred.push_back(other),
                Err(_) => {}
            }
        }
    }

    pub fn pick(&mut self, title: &str, items: &[String]) -> Option<usize> {
        self.ask("window/pick", serde_json::json!({"title": title, "items": items}))?.get("index")?.as_u64().map(|n| n as usize)
    }
    pub fn prompt(&mut self, title: &str, default: &str) -> Option<String> {
        self.ask("window/prompt", serde_json::json!({"title": title, "default": default}))?.get("text")?.as_str().map(str::to_string)
    }
    pub fn confirm(&mut self, title: &str) -> bool {
        self.ask("window/confirm", serde_json::json!({"title": title})).and_then(|v| v.get("yes")?.as_bool()).unwrap_or(false)
    }
    pub fn progress(&mut self, text: &str) {
        self.send(&Message::Notification { method: "window/progress".into(), params: serde_json::json!({"text": text}) });
    }
    pub fn message(&mut self, text: &str) {
        self.send(&Message::Notification { method: "window/message".into(), params: serde_json::json!({"text": text}) });
    }
    pub fn status(&mut self, field: &str, text: &str, color: Option<&str>) {
        self.send(&Message::Notification { method: "status/set".into(), params: serde_json::json!({"field": field, "text": text, "color": color}) });
    }
    pub fn fields(&mut self, fields: &FieldsMap) {
        self.send(&Message::Notification { method: "fields/set".into(), params: serde_json::json!({"fields": fields}) });
    }
    pub fn refresh(&mut self) {
        self.send(&Message::Notification { method: "library/refresh".into(), params: serde_json::json!({}) });
    }
}

impl<'a> Ui<'a> {
    #[cfg(test)]
    pub fn for_test(reader: &'a mut dyn BufRead, writer: &'a mut dyn Write, deferred: &'a mut VecDeque<Message>, next_id: &'a mut u64) -> Ui<'a> {
        Ui { reader, writer, deferred, next_id }
    }
}

pub type Handler<'h> = &'h mut dyn FnMut(&str, Value, &mut Ui) -> Result<Value, RpcError>;

pub fn serve_io(name: &str, reader: &mut dyn BufRead, writer: &mut dyn Write, handler: Handler) {
    let mut deferred: VecDeque<Message> = VecDeque::new();
    let mut next_id: u64 = 1;
    let mut line = String::new();
    loop {
        let msg = if let Some(m) = deferred.pop_front() {
            m
        } else {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => return,
                Ok(_) => {}
            }
            let text = line.trim_end();
            if text.is_empty() { continue; }
            match rpc::parse_line(text) {
                Ok(m) => m,
                Err(e) => {
                    let _ = write_msg(writer, &Message::Error { id: None, error: e });
                    continue;
                }
            }
        };
        match msg {
            Message::Request { id, method, params } => {
                let result = match method.as_str() {
                    "initialize" => {
                        // 답을 먼저 쓰고, paths/config를 볼 수 있게 핸들러에도 한 번 넘긴다(결과는 버림)
                        let ok = serde_json::to_value(InitializeResult { name: name.to_string(), protocol: PROTOCOL }).unwrap_or(Value::Null);
                        if write_msg(writer, &Message::Response { id, result: ok }).is_err() { return; }
                        let mut ui = Ui { reader: &mut *reader, writer: &mut *writer, deferred: &mut deferred, next_id: &mut next_id };
                        let _ = handler("initialize", params, &mut ui);
                        continue;
                    }
                    "shutdown" => {
                        let _ = write_msg(writer, &Message::Response { id, result: Value::Object(Default::default()) });
                        return;
                    }
                    _ => {
                        let mut ui = Ui { reader: &mut *reader, writer: &mut *writer, deferred: &mut deferred, next_id: &mut next_id };
                        handler(&method, params, &mut ui)
                    }
                };
                let out = match result {
                    Ok(v) => Message::Response { id, result: v },
                    Err(e) => Message::Error { id: Some(id), error: e },
                };
                if write_msg(writer, &out).is_err() { return; }
            }
            Message::Notification { method, params } => {
                let mut ui = Ui { reader: &mut *reader, writer: &mut *writer, deferred: &mut deferred, next_id: &mut next_id };
                let _ = handler(&method, params, &mut ui);
            }
            Message::Response { .. } | Message::Error { .. } => {}
        }
    }
}

fn write_msg(writer: &mut dyn Write, msg: &Message) -> std::io::Result<()> {
    writer.write_all(rpc::to_line(msg).as_bytes())?;
    writer.write_all(b"\n")?;
    writer.flush()
}

pub fn serve(name: &str, handler: Handler) {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut reader = stdin.lock();
    let mut writer = stdout.lock();
    serve_io(name, &mut reader, &mut writer, handler);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Cursor;

    fn line(v: serde_json::Value) -> String { format!("{}\n", v) }

    /// initialize와 shutdown은 루프가 답하고(initialize의 params는 handler에도 한 번 보인다), 그 사이 요청은 handler로, 알림은 결과 없이 handler로.
    #[test]
    fn the_loop_answers_lifecycle_itself_and_routes_the_rest() {
        let input = line(json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocol": 2}}))
            + &line(json!({"jsonrpc": "2.0", "id": 2, "method": "commands/run", "params": {"command": "go"}}))
            + &line(json!({"jsonrpc": "2.0", "method": "library/written", "params": {"reason": "add"}}))
            + &line(json!({"jsonrpc": "2.0", "id": 3, "method": "nope/x", "params": {}}))
            + &line(json!({"jsonrpc": "2.0", "id": 4, "method": "shutdown", "params": {}}))
            + &line(json!({"jsonrpc": "2.0", "id": 5, "method": "commands/run", "params": {}}));
        let mut out = Vec::new();
        let mut seen: Vec<String> = Vec::new();
        {
            let mut h = |method: &str, params: Value, _ui: &mut Ui| -> Result<Value, RpcError> {
                seen.push(method.to_string());
                match method {
                    "commands/run" => Ok(json!({"message": format!("ran {}", params["command"])})),
                    "library/written" => Ok(Value::Null),
                    m => Err(RpcError::method_not_found(m)),
                }
            };
            serve_io("demo", &mut Cursor::new(input), &mut out, &mut h);
        }
        let lines: Vec<Value> = String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(lines[0]["id"], 1);
        assert_eq!(lines[0]["result"]["protocol"], 2);
        assert_eq!(lines[0]["result"]["name"], "demo");
        assert_eq!(lines[1]["result"]["message"], "ran \"go\"");
        assert_eq!(lines[2]["error"]["code"], -32601);
        assert_eq!(lines[3]["id"], 4, "shutdown is answered and the loop ends before id 5");
        assert_eq!(lines.len(), 4);
        assert_eq!(seen, vec!["initialize", "commands/run", "library/written", "nope/x"], "initialize is answered by the loop and then shown to the handler");
    }

    /// 핸들러 안에서 Ui가 요청을 보내면 답이 올 때까지 읽되, 그 사이에 온 bibox 알림은 버리지 않고 뒤에 처리한다.
    #[test]
    fn ui_requests_read_their_own_answer_and_keep_other_lines_for_later() {
        let input = line(json!({"jsonrpc": "2.0", "id": 1, "method": "commands/run", "params": {}}))
            + &line(json!({"jsonrpc": "2.0", "method": "config/changed", "params": {"config": {}}}))
            + &line(json!({"jsonrpc": "2.0", "id": 1, "result": {"index": 1}}));
        let mut out = Vec::new();
        let mut seen: Vec<String> = Vec::new();
        {
            let mut h = |method: &str, _p: Value, ui: &mut Ui| -> Result<Value, RpcError> {
                seen.push(method.to_string());
                if method == "commands/run" {
                    let i = ui.pick("Style", &["a".into(), "b".into()]);
                    ui.status("s", "done", None);
                    return Ok(json!({"message": format!("picked {:?}", i)}));
                }
                Ok(Value::Null)
            };
            serve_io("demo", &mut Cursor::new(input), &mut out, &mut h);
        }
        let text = String::from_utf8(out).unwrap();
        let lines: Vec<Value> = text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(lines[0]["method"], "window/pick");
        assert_eq!(lines[0]["id"], 1, "the plugin numbers its own requests from 1");
        assert_eq!(lines[1]["method"], "status/set");
        assert!(lines[1].get("id").is_none(), "status/set is a notification");
        assert_eq!(lines[2]["result"]["message"], "picked Some(1)");
        assert_eq!(seen, vec!["commands/run", "config/changed"], "the notification that arrived mid-pick is handled after");
    }
}
