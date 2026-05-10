# Zene Agent SDK

> **Experimental** — Zene Agent SDK is under active development.

Zene Agent SDK is the core framework for building and running AI agents in Zene Agent Cloud. It provides the agent harness, session management, tool system, and Cloudflare Worker integration.

## Overview

Zene Agent SDK is a TypeScript framework for building AI agents that run on Cloudflare Workers. It provides:

- **Agent Harness**: Manages agent lifecycle, prompt execution, and tool orchestration
- **Session Management**: Persistent sessions with history
- **Tool System**: Built-in tools (read, write, edit, bash, grep, glob) + custom tools
- **Sandbox**: Virtual sandbox (just-bash) or container sandbox
- **Streaming**: Event-based streaming for real-time responses
- **Multi-Provider**: Supports Anthropic, OpenAI, DeepSeek, and custom providers via Cloudflare AI Gateway

## Usage

### Quick Start

```typescript
// .agents/hello.ts
import type { ZeneContext } from '@zene/agent/client';

export const triggers = { webhook: true };

export default async function ({ init, payload }: ZeneContext) {
  const agent = await init({ model: 'anthropic/claude-sonnet-4-6' });
  const session = await agent.session();

  const result = await session.prompt(`Translate to ${payload.language}: "${payload.text}"`);
  return result;
}
```

### Configuration

```typescript
// zene.config.ts
import { defineConfig } from '@zene/agent/config';

export default defineConfig({
  target: 'cloudflare',
  providers: {
    deepseek: {
      apiKey: process.env.DEEPSEEK_API_KEY,
      baseUrl: 'https://api.deepseek.com',
    },
  },
});
```

### Cloudflare AI Gateway

```typescript
// Use Cloudflare AI Gateway for provider routing
export default async function ({ init }: ZeneContext) {
  const agent = await init({
    model: 'deepseek/deepseek-chat',
    gateway: {
      url: process.env.AI_GATEWAY_URL,
    },
  });
  // ...
}
```

## API

### `init(options)`

Initialize an agent instance.

```typescript
const agent = await init({
  model: 'anthropic/claude-sonnet-4-6',  // Model to use
  sandbox: await getVirtualSandbox(),       // Sandbox environment
  providers: { ... },                      // Provider configuration
  gateway: { url: '...' },               // AI Gateway URL
});
```

### `agent.session(id?)`

Get or create a session.

```typescript
const session = await agent.session('session-123');
```

### `session.prompt(text, options?)`

Send a prompt to the agent and get a response.

```typescript
const response = await session.prompt('Hello!', {
  model: 'deepseek/deepseek-chat',  // Override model per prompt
  temperature: 0.7,
  maxTokens: 4096,
});
```

### Events

Subscribe to streaming events:

```typescript
session.onEvent((event) => {
  if (event.type === 'text_delta') {
    console.log(event.text);  // Streaming text
  }
});
```

## License

Apache-2.0
