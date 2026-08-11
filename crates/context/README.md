# zene-context

Semantic context engine for LLM coding agents: token estimation, compaction, memory flush, prefire, and inference-gateway epoch assembly.

Part of the composable [Zene](https://github.com/ParaTensor/zene) agent stack. Usable without `zene-core`.

## Features

- `ContextEngine` — estimate → compact → assemble → epoch
- `ContextSession` trait for any runtime's session store
- `ContextEventHandler` trait for runtime IO (gateway, memory flush, segment store)
- `ContextHooks` for todos / background task reminders
- Optional cargo features: `memory`, `gateway`, `prefire` (all on by default)

## Minimal integration

```rust
use zene_context::{
    ContextDeps, ContextEngine, ContextEvent, ContextEventHandler, ContextHooks,
    EventOutcome, NoContextHooks, NoopContextEventHandler, CompactionConfig,
};
use zene_llm::{ChatClient, ChatRequest};

struct MySession { /* impl ContextSession + persist_checkpoint */ }

struct MyHandler;

#[async_trait::async_trait]
impl ContextEventHandler for MyHandler {
    async fn handle(&mut self, event: &ContextEvent) -> anyhow::Result<EventOutcome> {
        match event {
            ContextEvent::MemoryFlush { conversation } => {
                // run sidecar LLM + persist to your store
                let _ = conversation;
                Ok(EventOutcome::MemoryFlush(zene_context::FlushResult::NothingToStore))
            }
            ContextEvent::PublishPrefix { session_id, epoch, messages } => {
                // notify inference gateway
                let _ = (session_id, epoch, messages);
                Ok(EventOutcome::Void)
            }
            _ => Ok(EventOutcome::Void),
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut engine = ContextEngine::new(128_000);
    let client = ChatClient::from_config(&config).await?;
    let compaction = CompactionConfig::default();
    let hooks = NoContextHooks;
    let mut handler = NoopContextEventHandler;

    loop {
        let mut session: MySession = /* ... */;
        let mut deps = ContextDeps {
            session: &mut session,
            compaction_config: &compaction,
            model: "gpt-4o",
            client: &client,
            hooks: Some(&hooks),
            system_prompt: "You are a helpful assistant.",
            estimator: &Default::default(),
            handler: &mut handler,
            #[cfg(feature = "prefire")]
            prefire_client_factory: None,
        };
        let prepared = engine.prepare_step(&mut deps, &[]).await?;
        // prepared.events mirrors side effects already handled by handler
        let resp = client.chat(ChatRequest {
            model: "gpt-4o".into(),
            messages: prepared.step.messages,
            context: Some(prepared.step.metadata),
            tools: vec![],
            stream: false,
        }).await?;
        // append response, record usage, repeat
        break;
    }
    Ok(())
}
```

## Lightweight dependency set

```toml
zene-context = { version = "0.1", default-features = false }
```

## License

MIT
