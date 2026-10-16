//! Wire format of the Codex app-server protocol.
//!
//! The app server speaks JSON-RPC-shaped frames over a WebSocket, but without
//! the `"jsonrpc"` field: requests are `{"id","method","params"}`, responses
//! `{"id","result"}` or `{"id","error"}`, notifications `{"method","params"}`.
//! The server also sends requests of its own (tool calls, approvals) that the
//! client answers with a response frame carrying the same id.
//!
//! Everything here is pure (no IO) so the framing and the defensive parsing of
//! server payloads are unit-tested.

use serde_json::{Map, Value, json};

/// A frame received from the app server.
#[derive(Clone, Debug, PartialEq)]
pub enum Incoming {
    /// The answer to one of our requests.
    Response {
        id: i64,
        result: Result<Value, RpcError>,
    },
    /// A server event (`item/agentMessage/delta`, `turn/completed`, …).
    Notification { method: String, params: Value },
    /// A server → client request (`item/tool/call`, approvals) that must be
    /// answered with [`response_frame`] / [`error_frame`] using `id` verbatim.
    Request {
        id: Value,
        method: String,
        params: Value,
    },
}

/// The `error` member of a failed response.
#[derive(Clone, Debug, PartialEq)]
pub struct RpcError {
    pub message: String,
}

/// Encode a client request.
pub fn request_frame(id: i64, method: &str, params: Value) -> String {
    json!({ "id": id, "method": method, "params": params }).to_string()
}

/// Encode a client notification (no id, no reply expected).
pub fn notification_frame(method: &str, params: Value) -> String {
    json!({ "method": method, "params": params }).to_string()
}

/// Encode the successful answer to a server request.
pub fn response_frame(id: &Value, result: Value) -> String {
    json!({ "id": id, "result": result }).to_string()
}

/// Encode the failed answer to a server request.
pub fn error_frame(id: &Value, code: i64, message: &str) -> String {
    json!({ "id": id, "error": { "code": code, "message": message } }).to_string()
}

/// Decode one text frame. Returns `None` for anything that is not a JSON
/// object of a recognised shape (including responses with non-integer ids,
/// which can't belong to us).
pub fn parse_frame(text: &str) -> Option<Incoming> {
    let Value::Object(mut object) = serde_json::from_str::<Value>(text).ok()? else {
        return None;
    };
    let method = object
        .get("method")
        .and_then(Value::as_str)
        .map(str::to_string);
    let id = object.remove("id").filter(|id| !id.is_null());
    let params = object
        .remove("params")
        .filter(|p| !p.is_null())
        .unwrap_or_else(|| Value::Object(Map::new()));

    match (method, id) {
        (Some(method), Some(id)) => Some(Incoming::Request { id, method, params }),
        (Some(method), None) => Some(Incoming::Notification { method, params }),
        (None, Some(id)) => {
            let id = id
                .as_i64()
                .or_else(|| id.as_str().and_then(|s| s.parse().ok()))?;
            let result = match object.remove("error").filter(|e| !e.is_null()) {
                Some(error) => Err(RpcError {
                    message: error
                        .get("message")
                        .and_then(Value::as_str)
                        .filter(|m| !m.is_empty())
                        .unwrap_or("Codex app-server request failed.")
                        .to_string(),
                }),
                None => Ok(object
                    .remove("result")
                    .unwrap_or_else(|| Value::Object(Map::new()))),
            };
            Some(Incoming::Response { id, result })
        }
        (None, None) => None,
    }
}

// ---- Payload helpers --------------------------------------------------------

/// Depth-first search for the first string member called `name`.
pub fn find_string<'a>(name: &str, value: &'a Value) -> Option<&'a str> {
    match value {
        Value::Object(map) => {
            if let Some(direct) = map.get(name).and_then(Value::as_str) {
                return Some(direct);
            }
            map.values().find_map(|child| find_string(name, child))
        }
        Value::Array(items) => items.iter().find_map(|child| find_string(name, child)),
        _ => None,
    }
}

/// The turn a payload belongs to: `turnId`, `turn_id`, or `turn.id`, searched
/// recursively like the Swift client did.
pub fn turn_identifier(value: &Value) -> Option<String> {
    match value {
        Value::Object(map) => {
            for key in ["turnId", "turn_id"] {
                if let Some(id) = map.get(key).and_then(Value::as_str) {
                    return Some(id.to_string());
                }
            }
            if let Some(id) = map
                .get("turn")
                .and_then(|t| t.get("id"))
                .and_then(Value::as_str)
            {
                return Some(id.to_string());
            }
            map.values().find_map(turn_identifier)
        }
        Value::Array(items) => items.iter().find_map(turn_identifier),
        _ => None,
    }
}

/// The thread a notification or server request belongs to, when it says so
/// at the top level (`threadId` or `thread.id`).
pub fn thread_identifier(params: &Value) -> Option<&str> {
    params
        .get("threadId")
        .and_then(Value::as_str)
        .or_else(|| params.pointer("/thread/id").and_then(Value::as_str))
}

/// `item/completed` for an agent message: `(item id, full text)`.
pub fn completed_agent_message(params: &Value) -> Option<(Option<&str>, &str)> {
    let item = params.get("item")?;
    if item.get("type").and_then(Value::as_str) != Some("agentMessage") {
        return None;
    }
    let text = item.get("text").and_then(Value::as_str)?;
    Some((item.get("id").and_then(Value::as_str), text))
}

/// For `turn/completed`: the failure message when `turn.status == "failed"`.
pub fn turn_error_message(params: &Value) -> Option<String> {
    let turn = params.get("turn")?;
    if turn.get("status").and_then(Value::as_str) != Some("failed") {
        return None;
    }
    match turn.get("error").filter(|e| !e.is_null()) {
        Some(error) => Some(error_message(&json!({ "error": error }))),
        None => Some("The turn failed before Codex returned an answer.".into()),
    }
}

/// The human-readable message of an `error` payload. Upstream API errors are
/// often a JSON document inside `message`; unwrap that when present.
pub fn error_message(params: &Value) -> String {
    let raw = params
        .pointer("/error/message")
        .and_then(Value::as_str)
        .unwrap_or("Unknown app-server error.");
    serde_json::from_str::<Value>(raw)
        .ok()
        .and_then(|v| {
            v.pointer("/error/message")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| raw.to_string())
}

/// Whether a `thread/start` failure means this app-server build doesn't
/// understand `dynamicTools` (so the thread should be retried without them).
pub fn is_dynamic_tools_unsupported_error(message: &str) -> bool {
    let m = message.to_lowercase();
    let mentions_tools =
        m.contains("dynamictools") || m.contains("dynamic tools") || m.contains("dynamic_tools");
    let mentions_unsupported = ["unsupported", "unknown", "unrecognized", "invalid"]
        .iter()
        .any(|w| m.contains(w));
    (mentions_tools && mentions_unsupported)
        || (m.contains("capability") && m.contains("unsupported"))
}

/// Remove a trailing `**Session Summary**` block (and a separator line right
/// before it) that some Codex prompts append to answers.
pub fn strip_session_summary(text: &str) -> String {
    let Some(at) = text.find("**Session Summary**") else {
        return text.to_string();
    };
    let prefix = text[..at].trim();
    let mut lines: Vec<&str> = prefix.split('\n').collect();
    if lines
        .last()
        .is_some_and(|line| !line.chars().any(char::is_alphanumeric))
    {
        lines.pop();
    }
    lines.join("\n").trim().to_string()
}

/// The newest agent message of the last turn in a `thread/read` response.
pub fn latest_agent_message(response: &Value) -> Option<String> {
    let turns = response.pointer("/thread/turns")?.as_array()?;
    let items = turns.last()?.get("items")?.as_array()?;
    items.iter().rev().find_map(|item| {
        (item.get("type").and_then(Value::as_str) == Some("agentMessage"))
            .then(|| item.get("text").and_then(Value::as_str))
            .flatten()
            .filter(|t| !t.trim().is_empty())
            .map(str::to_string)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_have_no_jsonrpc_field() {
        let frame: Value =
            serde_json::from_str(&request_frame(7, "turn/start", json!({"threadId": "t"})))
                .unwrap();
        assert_eq!(
            frame,
            json!({"id": 7, "method": "turn/start", "params": {"threadId": "t"}})
        );
        assert!(frame.get("jsonrpc").is_none());

        let reply: Value =
            serde_json::from_str(&response_frame(&json!("abc"), json!({"ok": true}))).unwrap();
        assert_eq!(reply, json!({"id": "abc", "result": {"ok": true}}));

        let err: Value = serde_json::from_str(&error_frame(&json!(3), -32601, "nope")).unwrap();
        assert_eq!(
            err,
            json!({"id": 3, "error": {"code": -32601, "message": "nope"}})
        );

        let note: Value =
            serde_json::from_str(&notification_frame("initialized", json!({}))).unwrap();
        assert_eq!(note, json!({"method": "initialized", "params": {}}));
    }

    #[test]
    fn parses_every_frame_kind() {
        assert_eq!(
            parse_frame(r#"{"id":1,"result":{"thread":{"id":"x"}}}"#),
            Some(Incoming::Response {
                id: 1,
                result: Ok(json!({"thread": {"id": "x"}}))
            })
        );
        assert_eq!(
            parse_frame(r#"{"id":2,"error":{"code":-1,"message":"bad"}}"#),
            Some(Incoming::Response {
                id: 2,
                result: Err(RpcError {
                    message: "bad".into()
                })
            })
        );
        assert_eq!(
            parse_frame(r#"{"id":3,"error":{}}"#),
            Some(Incoming::Response {
                id: 3,
                result: Err(RpcError {
                    message: "Codex app-server request failed.".into()
                })
            })
        );
        assert_eq!(
            parse_frame(r#"{"id":4}"#),
            Some(Incoming::Response {
                id: 4,
                result: Ok(json!({}))
            })
        );
        assert_eq!(
            parse_frame(r#"{"method":"turn/started","params":{"turn":{"id":"t1"}}}"#),
            Some(Incoming::Notification {
                method: "turn/started".into(),
                params: json!({"turn": {"id": "t1"}})
            })
        );
        assert_eq!(
            parse_frame(r#"{"method":"x"}"#),
            Some(Incoming::Notification {
                method: "x".into(),
                params: json!({})
            })
        );
        assert_eq!(
            parse_frame(r#"{"id":"srv-9","method":"item/tool/call","params":{"tool":"t"}}"#),
            Some(Incoming::Request {
                id: json!("srv-9"),
                method: "item/tool/call".into(),
                params: json!({"tool": "t"})
            })
        );
    }

    #[test]
    fn rejects_garbage() {
        assert_eq!(parse_frame("not json"), None);
        assert_eq!(parse_frame("[1,2]"), None);
        assert_eq!(parse_frame("{}"), None);
        assert_eq!(parse_frame(r#"{"id":{"weird":1},"result":{}}"#), None);
        assert_eq!(parse_frame(r#"{"id":null,"result":{}}"#), None);
    }

    #[test]
    fn payload_helpers() {
        let delta = json!({"threadId": "th", "turnId": "tu", "itemId": "i", "delta": "Hel"});
        assert_eq!(find_string("delta", &delta), Some("Hel"));
        assert_eq!(
            find_string("delta", &json!({"a": [{"b": {"delta": "deep"}}]})),
            Some("deep")
        );
        assert_eq!(turn_identifier(&delta).as_deref(), Some("tu"));
        assert_eq!(
            turn_identifier(&json!({"turn": {"id": "z"}})).as_deref(),
            Some("z")
        );
        assert_eq!(
            turn_identifier(&json!({"x": {"turn_id": "s"}})).as_deref(),
            Some("s")
        );
        assert_eq!(turn_identifier(&json!({"x": 1})), None);
        assert_eq!(thread_identifier(&delta), Some("th"));
        assert_eq!(
            thread_identifier(&json!({"thread": {"id": "a"}})),
            Some("a")
        );

        let completed = json!({"item": {"type": "agentMessage", "id": "m1", "text": "Hi"}});
        assert_eq!(
            completed_agent_message(&completed),
            Some((Some("m1"), "Hi"))
        );
        assert_eq!(
            completed_agent_message(&json!({"item": {"type": "reasoning", "text": "x"}})),
            None
        );
    }

    #[test]
    fn turn_errors() {
        assert_eq!(
            turn_error_message(&json!({"turn": {"status": "completed"}})),
            None
        );
        assert_eq!(
            turn_error_message(
                &json!({"turn": {"status": "failed", "error": {"message": "quota"}}})
            )
            .as_deref(),
            Some("quota")
        );
        assert_eq!(
            turn_error_message(&json!({"turn": {"status": "failed"}})).as_deref(),
            Some("The turn failed before Codex returned an answer.")
        );
        let nested = json!({"error": {"message": "{\"error\":{\"message\":\"Rate limited\"}}"}});
        assert_eq!(error_message(&nested), "Rate limited");
        assert_eq!(error_message(&json!({})), "Unknown app-server error.");
    }

    #[test]
    fn dynamic_tools_error_detection() {
        assert!(is_dynamic_tools_unsupported_error(
            "unknown field `dynamicTools`"
        ));
        assert!(is_dynamic_tools_unsupported_error("Capability unsupported"));
        assert!(!is_dynamic_tools_unsupported_error("thread limit reached"));
    }

    #[test]
    fn session_summary_is_stripped() {
        assert_eq!(strip_session_summary("Answer"), "Answer");
        assert_eq!(
            strip_session_summary("Answer\n\n---\n**Session Summary**\nstuff"),
            "Answer"
        );
        assert_eq!(strip_session_summary("A\nB\n**Session Summary** x"), "A\nB");
    }

    #[test]
    fn latest_message_reads_last_turn_only() {
        let read = json!({"thread": {"turns": [
            {"items": [{"type": "agentMessage", "text": "old"}]},
            {"items": [
                {"type": "agentMessage", "text": "first"},
                {"type": "dynamicToolCall"},
                {"type": "agentMessage", "text": "  "}
            ]}
        ]}});
        assert_eq!(latest_agent_message(&read).as_deref(), Some("first"));
        assert_eq!(
            latest_agent_message(&json!({"thread": {"turns": [{"items": []}]}})),
            None
        );
    }
}
