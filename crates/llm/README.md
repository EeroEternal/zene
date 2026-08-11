# zene-llm

LLM protocol layer for [Zene](https://github.com/ParaTensor/zene): messages, providers, streaming, and inference metadata.

## Features

- OpenAI-compatible and Anthropic providers
- `ChatClient` unified API (`chat`, `chat_stream`)
- `Message` / `ToolCall` / `ToolDefinition` models
- `ContextMetadata` for session linkage (`session_id`, `context_epoch`, delta delivery)
- `TokenUsage` including optional `cached_tokens`

## Example

```rust
use zene_llm::{ChatClient, ChatRequest, Message};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = ChatClient::from_config(&config).await?;
    let response = client.chat(ChatRequest {
        model: "gpt-4o".into(),
        messages: vec![Message::user("Hello")],
        tools: vec![],
        stream: false,
        context: None,
    }).await?;
    println!("{}", response.message.content.unwrap_or_default());
    Ok(())
}
```

## License

MIT
