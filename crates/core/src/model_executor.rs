use std::collections::HashSet;

use zene_llm::{Message, ToolCall};

#[derive(Default)]
pub(crate) struct ToolCallBuilder {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) arguments: String,
}

pub(crate) fn apply_tool_call_delta(
    call: &mut ToolCallBuilder,
    id: Option<String>,
    name: Option<String>,
    arguments: Option<String>,
) {
    if let Some(id) = id {
        call.id = id;
    }
    if let Some(name) = name {
        call.name = name;
    }
    if let Some(arguments) = arguments {
        call.arguments.push_str(&arguments);
    }
}

pub(crate) fn assemble_message(text: String, builders: Vec<ToolCallBuilder>) -> Message {
    let calls = normalize_tool_calls(
        builders
            .into_iter()
            .filter(|call| !call.name.is_empty())
            .map(|call| ToolCall {
                id: call.id,
                name: call.name,
                arguments: call.arguments,
            })
            .collect(),
    );
    if calls.is_empty() {
        Message::assistant(text)
    } else {
        Message::assistant_with_tools((!text.is_empty()).then_some(text), calls)
    }
}

fn normalize_tool_calls(mut calls: Vec<ToolCall>) -> Vec<ToolCall> {
    let mut used_ids = HashSet::new();
    for (index, call) in calls.iter_mut().enumerate() {
        if call.id.trim().is_empty() {
            call.id = format!("call_{index}");
        }
        let base = call.id.clone();
        let mut unique = base.clone();
        let mut suffix = 0u32;
        while !used_ids.insert(unique.clone()) {
            suffix += 1;
            unique = format!("{base}_{suffix}");
        }
        call.id = unique;
    }
    calls
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assembles_streamed_text() {
        let message = assemble_message("hello world".into(), Vec::new());
        assert_eq!(message.content.as_deref(), Some("hello world"));
        assert!(message.tool_calls.is_none());
    }

    #[test]
    fn assembles_multiple_tool_deltas() {
        let mut first = ToolCallBuilder::default();
        apply_tool_call_delta(
            &mut first,
            Some("call-a".into()),
            Some("Read".into()),
            Some("{\"path\":".into()),
        );
        apply_tool_call_delta(&mut first, None, None, Some("\"a.rs\"}".into()));
        let mut second = ToolCallBuilder::default();
        apply_tool_call_delta(&mut second, None, Some("Write".into()), Some("{}".into()));
        let message = assemble_message("checking".into(), vec![first, second]);
        let calls = message.tool_calls.expect("tool calls");
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].arguments, "{\"path\":\"a.rs\"}");
        assert_eq!(calls[1].id, "call_1");
    }

    #[test]
    fn normalizes_duplicate_and_missing_ids() {
        let calls = normalize_tool_calls(vec![
            ToolCall { id: "".into(), name: "A".into(), arguments: "{}".into() },
            ToolCall { id: "".into(), name: "B".into(), arguments: "{}".into() },
            ToolCall { id: "call_0".into(), name: "C".into(), arguments: "{}".into() },
        ]);
        assert_eq!(
            calls.iter().map(|call| call.id.as_str()).collect::<Vec<_>>(),
            ["call_0", "call_1", "call_0_1"]
        );
    }
}
