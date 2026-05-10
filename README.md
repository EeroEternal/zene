# Zene Agent Cloud

A web application that provides an OpenAI Codex-style chat interface for users to interact with multiple Flue Agents running on Cloudflare Workers.

## Features

- Chat-style UI (OpenAI Codex Web style)
- Multiple agent sessions
- Real-time streaming responses (typewriter effect)
- Session management (create, switch, delete, history)
- Dark/light theme toggle
- Markdown rendering with code highlighting
- Configurable LLM provider connections via Cloudflare AI Gateway

## Architecture

```
zene/
├── apps/
│   ├── web/              # Next.js web UI
│   └── worker/           # Cloudflare Worker (agent runtime)
├── packages/
│   └── agent/            # Agent SDK (from flue, modified)
├── package.json
└── pnpm-workspace.yaml
```

## Getting Started

### Prerequisites

- Node.js >= 22.18.0
- pnpm >= 10.0.0
- Cloudflare account (for worker deployment)

### Installation

```bash
pnpm install
```

### Development

```bash
# Start web UI
pnpm dev

# Start worker (Cloudflare Worker)
pnpm worker:dev
```

## License

Apache-2.0
