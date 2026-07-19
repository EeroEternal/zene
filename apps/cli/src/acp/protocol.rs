use serde_json::{json, Value};

/// JSON-RPC 2.0 standard / reserved error codes used by the ACP server.
#[allow(dead_code)]
pub mod error_codes {
    pub const PARSE_ERROR: i64 = -32700;
    pub const INVALID_REQUEST: i64 = -32600;
    pub const METHOD_NOT_FOUND: i64 = -32601;
    pub const INVALID_PARAMS: i64 = -32602;
    pub const INTERNAL_ERROR: i64 = -32603;
    /// Application-level error (session busy, agent failure, etc.).
    pub const APPLICATION_ERROR: i64 = -32000;
}

#[derive(Debug, Clone)]
pub enum RpcId {
    Number(u64),
    String(String),
    Null,
}

impl RpcId {
    pub fn from_value(v: &Value) -> Self {
        match v {
            Value::Number(n) => n
                .as_u64()
                .map(RpcId::Number)
                .unwrap_or(RpcId::Null),
            Value::String(s) => RpcId::String(s.clone()),
            _ => RpcId::Null,
        }
    }

    pub fn to_value(&self) -> Value {
        match self {
            RpcId::Number(n) => json!(n),
            RpcId::String(s) => json!(s),
            RpcId::Null => Value::Null,
        }
    }
}

pub fn is_request(msg: &Value) -> bool {
    msg.get("method").and_then(Value::as_str).is_some() && msg.get("id").is_some()
}

pub fn is_notification(msg: &Value) -> bool {
    msg.get("method").and_then(Value::as_str).is_some() && msg.get("id").is_none()
}

pub fn is_response(msg: &Value) -> bool {
    msg.get("id").is_some()
        && (msg.get("result").is_some() || msg.get("error").is_some())
        && msg.get("method").is_none()
}

pub fn ok_response(id: RpcId, result: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id.to_value(),
        "result": result,
    })
}

pub fn err_response(id: RpcId, code: i64, message: impl Into<String>) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id.to_value(),
        "error": {
            "code": code,
            "message": message.into(),
        }
    })
}

/// Extract plain text (and embedded context text) from ACP prompt content blocks.
pub fn prompt_text_from_params(params: &Value) -> String {
    let mut parts = Vec::new();
    if let Some(blocks) = params.get("prompt").and_then(Value::as_array) {
        for block in blocks {
            match block.get("type").and_then(Value::as_str) {
                Some("text") => {
                    if let Some(text) = block.get("text").and_then(Value::as_str) {
                        parts.push(text.to_string());
                    }
                }
                Some("resource") => {
                    // Embedded resource: prefer inline text content.
                    if let Some(text) = block
                        .pointer("/resource/text")
                        .or_else(|| block.pointer("/text"))
                        .and_then(Value::as_str)
                    {
                        let uri = block
                            .pointer("/resource/uri")
                            .or_else(|| block.get("uri"))
                            .and_then(Value::as_str)
                            .unwrap_or("resource");
                        parts.push(format!("<embedded resource uri=\"{uri}\">\n{text}\n</embedded resource>"));
                    } else if let Some(uri) = block
                        .pointer("/resource/uri")
                        .or_else(|| block.get("uri"))
                        .and_then(Value::as_str)
                    {
                        parts.push(format!("<embedded resource uri=\"{uri}\" />"));
                    }
                }
                Some("resource_link") => {
                    let uri = block.get("uri").and_then(Value::as_str).unwrap_or("");
                    let name = block.get("name").and_then(Value::as_str).unwrap_or(uri);
                    let description = block
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    if description.is_empty() {
                        parts.push(format!("<resource_link name=\"{name}\" uri=\"{uri}\" />"));
                    } else {
                        parts.push(format!(
                            "<resource_link name=\"{name}\" uri=\"{uri}\">{description}</resource_link>"
                        ));
                    }
                }
                _ => {}
            }
        }
    }
    if parts.is_empty() {
        if let Some(text) = params.get("text").and_then(Value::as_str) {
            return text.to_string();
        }
    }
    parts.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_text_blocks() {
        let params = json!({
            "sessionId": "s1",
            "prompt": [
                { "type": "text", "text": "hello" },
                { "type": "text", "text": "world" },
                { "type": "image", "data": "..." }
            ]
        });
        assert_eq!(prompt_text_from_params(&params), "hello\nworld");
    }

    #[test]
    fn extracts_embedded_resource_and_link() {
        let params = json!({
            "prompt": [
                { "type": "text", "text": "look at this" },
                {
                    "type": "resource",
                    "resource": { "uri": "file:///a.rs", "text": "fn main() {}" }
                },
                {
                    "type": "resource_link",
                    "uri": "file:///b.rs",
                    "name": "b.rs",
                    "description": "helper"
                }
            ]
        });
        let text = prompt_text_from_params(&params);
        assert!(text.contains("look at this"));
        assert!(text.contains("fn main() {}"));
        assert!(text.contains("file:///a.rs"));
        assert!(text.contains("helper"));
        assert!(text.contains("b.rs"));
    }

    #[test]
    fn classifies_jsonrpc_shapes() {
        assert!(is_request(&json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}})));
        assert!(is_notification(&json!({"jsonrpc":"2.0","method":"session/cancel","params":{}})));
        assert!(is_response(&json!({"jsonrpc":"2.0","id":1,"result":{}})));
    }
}
