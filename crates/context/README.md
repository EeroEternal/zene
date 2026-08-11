# zene-context

Semantic context engine for LLM coding agents: token estimation, compaction, memory flush, prefire, and inference-gateway epoch assembly.

Part of the composable [Zene](https://github.com/ParaTensor/zene) agent stack. Usable without `zene-core`.

## Features

- `ContextEngine` — estimate → compact → assemble → epoch
- `ContextSession` trait for any runtime's session store
- `ContextHooks` for todos / background task reminders
- Optional cargo features: `memory`, `gateway`, `prefire` (all on by default)

## Minimal integration

```rust
use zene_context::{
    ContextDeps, ContextEngine, ContextEvent, ContextHooks, ContextSession,
    NoContextHooks, CompactionConfig,
};
use zene_llm::{ChatClient, ChatRequest};

struct MySession { /* impl ContextSession */ }

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut engine = ContextEngine::new(128_000);
    let client = ChatClient::from_config(&config).await?;
    let compaction = CompactionConfig::default();
    let hooks = NoContextHooks;

    loop {
        let mut session: MySession = /* ... */;
        let mut deps = ContextDeps {
            session: &mut session,
            compaction_config: &compaction,
            model: "gpt-4o",
            workdir: std::path::Path::new("."),
            client: &client,
            hooks: Some(&hooks),
            system_prompt: "You are a helpful assistant.",
            estimator: &Default::default(),
            #[cfg(feature = "prefire")]
            prefire_client_factory: None,
        };
        let prepared = engine.prepare_step(&mut deps, &[]).await?;
        for event in &prepared.events {
            if let ContextEvent::Checkpoint { reason } = event {
                eprintln!("checkpoint: {reason}");
            }
        }
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
