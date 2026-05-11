/**
 * Agent handler - handles agent prompts with SSE streaming via Flue / pi-ai
 */
import type { Context } from 'hono';
import { createFlueContext, type FlueContext } from '@zene/agent/client';
import { resolveModel as resolvePiModel } from '@zene/agent/internal';

interface Env {
  DB: D1Database;
  AI_GATEWAY_URL?: string;
}

// In-memory provider config (module-level)
let providerConfig: Record<string, any> = {};

export function setProviderConfig(config: Record<string, any>): void {
  providerConfig = { ...providerConfig, ...config };
}

export function getProviderConfig(): Record<string, any> {
  return { ...providerConfig };
}

/**
 * Handle POST /api/agent/:agentName/prompt
 * Expects JSON body: { sessionId: string, prompt: string, model?: string }
 * Returns SSE stream
 */
export async function handleAgentPrompt(c: Context<{ Bindings: Env }>) {
  const { agentName } = c.req.param();
  const body = await c.req.json<{
    sessionId?: string;
    prompt: string;
    model?: string;
  }>();

  const { sessionId = 'default', prompt, model } = body;

  if (!prompt) {
    return c.json({ error: 'prompt is required' }, 400);
  }

  const gatewayBaseUrl = c.env.AI_GATEWAY_URL ?? providerConfig.gatewayUrl ?? '';
  const modelStr = model ?? 'deepseek/deepseek-v4-flash';
  const slash = modelStr.indexOf('/');
  const provider = slash === -1 ? modelStr : modelStr.slice(0, slash);

  // Build provider config: override baseUrl when gateway is configured
  const providers: Record<string, any> = {
    ...providerConfig.providers,
  };
  if (gatewayBaseUrl && provider !== 'cloudflare') {
    providers[provider] = {
      ...(providers[provider] ?? {}),
      baseUrl: `${gatewayBaseUrl}/${provider}`,
    };
  }

  // Create Flue context
  const ctx: FlueContext = createFlueContext({
    id: agentName,
    payload: {},
    env: {
      AI_GATEWAY_URL: c.env.AI_GATEWAY_URL,
      ...providerConfig,
    },
    agentConfig: {
      roles: {},
      skills: {},
      model: modelStr,
      systemPrompt: 'You are a helpful AI assistant.',
      thinkingLevel: 'medium',
      resolveModel: (modelConfig: string | undefined, p?: any) => {
        if (typeof modelConfig !== 'string') return undefined;
        return resolvePiModel(modelConfig, p);
      },
    },
    createDefaultEnv: async () => {
      return {
        cwd: '/tmp',
        exec: async (cmd: string) => ({ stdout: '', stderr: '', exitCode: 0 }),
        readFile: async (path: string) => '',
        writeFile: async (path: string, content: string) => {},
        stat: async (path: string) => ({ isDirectory: false }),
        readdir: async (path: string) => [],
      };
    },
    defaultStore: {
      save: async () => {},
      load: async () => null,
      delete: async () => {},
    },
  });

  // Initialize agent
  const agent = await ctx.init({
    model: modelStr,
    providers,
  });

  // Get or create session
  const session = await agent.session(sessionId);

  // Set up SSE stream
  const stream = new ReadableStream({
    async start(controller) {
      const encoder = new TextEncoder();

      // Subscribe to agent events
      ctx.setEventCallback((event) => {
        const data = `data: ${JSON.stringify(event)}\n\n`;
        controller.enqueue(encoder.encode(data));
      });

      try {
        // Send prompt
        await session.prompt(prompt);

        // End stream
        controller.enqueue(encoder.encode('data: [DONE]\n\n'));
        controller.close();
      } catch (error) {
        const errorData = `data: ${JSON.stringify({
          type: 'error',
          error: error instanceof Error ? error.message : String(error),
        })}\n\n`;
        controller.enqueue(encoder.encode(errorData));
        controller.enqueue(encoder.encode('data: [DONE]\n\n'));
        controller.close();
      }
    },
  });

  return new Response(stream, {
    headers: {
      'Content-Type': 'text/event-stream',
      'Cache-Control': 'no-cache',
      'Connection': 'keep-alive',
    },
  });
}
