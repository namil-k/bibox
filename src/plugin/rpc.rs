//! JSON-RPC 2.0 한 줄 한 메시지. 플러그인과 bibox가 양쪽 다 요청과 알림을 보낸다.
//! LSP와 달리 Content-Length 헤더가 없다(셸 플러그인이 printf로 답할 수 있게).

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub type Id = u64;

pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
/// 플러그인 자체 오류. message가 상태 줄에 보인다.
pub const PLUGIN_ERROR: i64 = -32000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl RpcError {
    pub fn new(code: i64, message: impl Into<String>) -> RpcError {
        RpcError { code, message: message.into(), data: None }
    }
    pub fn plugin(message: impl Into<String>) -> RpcError { RpcError::new(PLUGIN_ERROR, message) }
    pub fn method_not_found(method: &str) -> RpcError { RpcError::new(METHOD_NOT_FOUND, format!("unknown method {}", method)) }
    pub fn invalid_params(message: impl Into<String>) -> RpcError { RpcError::new(INVALID_PARAMS, message) }
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.message, self.code)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    Request { id: Id, method: String, params: Value },
    Notification { method: String, params: Value },
    Response { id: Id, result: Value },
    Error { id: Option<Id>, error: RpcError },
}

/// 원문 머리 60자. 오류 문구에 넣어 어느 줄이 문제였는지 보이게.
pub fn head(line: &str) -> String {
    let mut s: String = line.chars().take(60).collect();
    if line.chars().count() > 60 { s.push('…'); }
    s
}

pub fn parse_line(line: &str) -> Result<Message, RpcError> {
    let v: Value = serde_json::from_str(line).map_err(|e| RpcError::new(PARSE_ERROR, format!("not json ({}): {}", e, head(line))))?;
    let Some(obj) = v.as_object() else {
        return Err(RpcError::new(INVALID_REQUEST, format!("expected an object: {}", head(line))));
    };
    let id = obj.get("id").and_then(Value::as_u64);
    let method = obj.get("method").and_then(Value::as_str).map(str::to_string);
    let params = obj.get("params").cloned().unwrap_or_else(|| Value::Object(Default::default()));
    if let Some(err) = obj.get("error") {
        let error: RpcError = serde_json::from_value(err.clone()).map_err(|e| RpcError::new(INVALID_REQUEST, format!("bad error object ({}): {}", e, head(line))))?;
        return Ok(Message::Error { id, error });
    }
    match (id, method) {
        (Some(id), Some(method)) => Ok(Message::Request { id, method, params }),
        (None, Some(method)) => Ok(Message::Notification { method, params }),
        (Some(id), None) => Ok(Message::Response { id, result: obj.get("result").cloned().unwrap_or(Value::Null) }),
        (None, None) => Err(RpcError::new(INVALID_REQUEST, format!("neither method nor id: {}", head(line)))),
    }
}

/// 전선 형태. 구조체라 필드 순서가 고정되어 `jsonrpc`가 늘 맨 앞에 온다.
#[derive(Serialize)]
struct Wire<'a> {
    jsonrpc: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    method: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<&'a Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<&'a Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'a RpcError>,
}

pub fn to_line(msg: &Message) -> String {
    let mut w = Wire { jsonrpc: "2.0", id: None, method: None, params: None, result: None, error: None };
    match msg {
        Message::Request { id, method, params } => { w.id = Some(*id); w.method = Some(method); w.params = Some(params); }
        Message::Notification { method, params } => { w.method = Some(method); w.params = Some(params); }
        Message::Response { id, result } => { w.id = Some(*id); w.result = Some(result); }
        Message::Error { id, error } => { w.id = *id; w.error = Some(error); }
    }
    // serde_json은 문자열 안의 개행을 \n으로 이스케이프하므로 한 줄이 보장된다
    serde_json::to_string(&w).unwrap_or_else(|_| "{}".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_request_a_notification_a_response_and_an_error_round_trip() {
        let req = Message::Request { id: 7, method: "commands/run".into(), params: json!({"command": "copy"}) };
        let line = to_line(&req);
        assert!(line.starts_with("{\"jsonrpc\":\"2.0\""), "{}", line);
        assert!(!line.contains('\n'));
        assert_eq!(parse_line(&line).unwrap(), req);

        let note = Message::Notification { method: "status/set".into(), params: json!({"text": "x"}) };
        assert_eq!(parse_line(&to_line(&note)).unwrap(), note);

        let resp = Message::Response { id: 7, result: json!({"message": "ok"}) };
        assert_eq!(parse_line(&to_line(&resp)).unwrap(), resp);

        let err = Message::Error { id: Some(7), error: RpcError::plugin("boom") };
        assert_eq!(parse_line(&to_line(&err)).unwrap(), err);
        assert_eq!(RpcError::plugin("boom").code, PLUGIN_ERROR);
    }

    /// 플러그인이 params를 빼면 `{}`로, 응답에 result가 없으면 null로 본다.
    #[test]
    fn missing_params_and_result_default_to_empty() {
        assert_eq!(parse_line(r#"{"jsonrpc":"2.0","method":"library/refresh"}"#).unwrap(), Message::Notification { method: "library/refresh".into(), params: json!({}) });
        assert_eq!(parse_line(r#"{"jsonrpc":"2.0","id":1}"#).unwrap(), Message::Response { id: 1, result: serde_json::Value::Null });
    }

    /// 깨진 줄은 -32700, 객체가 아니거나 method도 id도 없으면 -32600. 메시지에 원문 머리가 들어간다.
    #[test]
    fn bad_lines_are_parse_or_invalid_request_errors() {
        let e = parse_line("not json").unwrap_err();
        assert_eq!(e.code, PARSE_ERROR);
        assert!(e.message.contains("not json"));
        let e = parse_line("[1,2]").unwrap_err();
        assert_eq!(e.code, INVALID_REQUEST);
        let e = parse_line(r#"{"jsonrpc":"2.0","params":{}}"#).unwrap_err();
        assert_eq!(e.code, INVALID_REQUEST);
        assert_eq!(RpcError::method_not_found("x/y").code, METHOD_NOT_FOUND);
        assert!(RpcError::method_not_found("x/y").message.contains("x/y"));
    }
}
