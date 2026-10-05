//! Batch tool calls for the model. A JavaScript sandbox runner is a later change
//! that should keep this call list as the thing the script is allowed to invoke.

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::Value;
use zene_llm::ToolDefinition;

use crate::registry::{Tool, ToolContext, ToolResult};

pub const CODEMODE_TOOL_NAME: &str = "codemode";
const MAX_CALLS: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodemodeCall {
    pub name: String,
    pub arguments: String,
}

#[derive(Deserialize)]
struct CodemodeArgs {
    calls: Vec<CodemodeCallArg>,
}

#[derive(Deserialize)]
struct CodemodeCallArg {
    name: String,
    #[serde(default)]
    arguments: Value,
}

pub fn parse_codemode(arguments: &str) -> Result<Vec<CodemodeCall>> {
    let args: CodemodeArgs = serde_json::from_str(arguments).context("parse codemode args")?;
    if args.calls.is_empty() {
        anyhow::bail!("codemode requires at least one call");
    }
    if args.calls.len() > MAX_CALLS {
        anyhow::bail!("codemode accepts at most {MAX_CALLS} calls");
    }
    let mut calls = Vec::with_capacity(args.calls.len());
    for call in args.calls {
        if call.name == CODEMODE_TOOL_NAME {
            anyhow::bail!("codemode cannot call itself");
        }
        if call.name.is_empty() {
            anyhow::bail!("codemode call is missing a tool name");
        }
        let arguments = if call.arguments.is_null() {
            "{}".to_string()
        } else if let Some(text) = call.arguments.as_str() {
            text.to_string()
        } else {
            call.arguments.to_string()
        };
        calls.push(CodemodeCall {
            name: call.name,
            arguments,
        });
    }
    Ok(calls)
}

pub struct CodemodeTool;

#[async_trait::async_trait]
impl Tool for CodemodeTool {
    fn name(&self) -> &str {
        CODEMODE_TOOL_NAME
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: CODEMODE_TOOL_NAME.to_string(),
            description: "Run up to 16 registered tools from one call. Pass calls as {name, arguments}. Prefer codemode when issuing 2+ independent tool calls in one turn to save round-trips. Do not use when calls depend on each other's results. Do not call codemode from inside codemode. A script sandbox is not available in this build; pass the call list directly.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "calls": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "name": { "type": "string" },
                                "arguments": {}
                            },
                            "required": ["name"]
                        }
                    }
                },
                "required": ["calls"]
            }),
        }
    }

    async fn execute(&self, arguments: &str, _ctx: &ToolContext) -> Result<ToolResult> {
        match parse_codemode(arguments) {
            Ok(calls) => Ok(ToolResult {
                content: format!(
                    "codemode planned {} call(s); the turn executor should have expanded them",
                    calls.len()
                ),
                is_error: true,
            }),
            Err(err) => Ok(ToolResult {
                content: err.to_string(),
                is_error: true,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_object_and_string_arguments() {
        let calls = parse_codemode(
            r#"{"calls":[{"name":"Read","arguments":{"path":"a.rs"}},{"name":"Grep","arguments":"{\"pattern\":\"x\"}"}]}"#,
        )
        .unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "Read");
        assert!(calls[0].arguments.contains("a.rs"));
        assert_eq!(calls[1].arguments, r#"{"pattern":"x"}"#);
    }

    #[test]
    fn rejects_nested_codemode() {
        let err = parse_codemode(r#"{"calls":[{"name":"codemode","arguments":{}}]}"#).unwrap_err();
        assert!(err.to_string().contains("itself"));
    }
}
