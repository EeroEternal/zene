/**
 * Agent handler - handles agent prompts with SSE streaming
 */
import type { Context } from 'hono';

interface Env {
  AI: any;
  DB: D1Database;
  AI_GATEWAY_URL?: string;
  DEEPSEEK_API_KEY?: string;
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
  const body = await c.req.json<{
    sessionId?: string;
    prompt: string;
    model?: string;
  }>();

  const { sessionId = 'default', prompt, model } = body;

  if (!prompt) {
    return c.json({ error: 'prompt is required' }, 400);
  }

  // Resolve model and provider
  const modelStr = model ?? 'deepseek/deepseek-v4-flash';
  const slash = modelStr.indexOf('/');
  const provider = slash === -1 ? modelStr : modelStr.slice(0, slash);
  const modelId = slash === -1 ? modelStr : modelStr.slice(slash + 1);

  // Resolve API key and base URL
  let apiKey = '';
  let baseUrl = '';

  const gatewayBaseUrl = c.env.AI_GATEWAY_URL ?? providerConfig.gatewayUrl ?? '';

  if (provider === 'deepseek') {
    apiKey = c.env.DEEPSEEK_API_KEY ?? providerConfig.providers?.deepseek?.apiKey ?? '';
    baseUrl = gatewayBaseUrl
      ? `${gatewayBaseUrl}/${provider}`
      : 'https://api.deepseek.com';
  } else if (provider === 'anthropic') {
    apiKey = providerConfig.providers?.anthropic?.apiKey ?? '';
    baseUrl = gatewayBaseUrl
      ? `${gatewayBaseUrl}/${provider}`
      : 'https://api.anthropic.com';
  } else if (provider === 'openai') {
    apiKey = providerConfig.providers?.openai?.apiKey ?? '';
    baseUrl = gatewayBaseUrl
      ? `${gatewayBaseUrl}/${provider}`
      : 'https://api.openai.com';
  } else {
    return c.json({ error: `Unsupported provider: ${provider}` }, 400);
  }

  if (!apiKey) {
    return c.json({ error: `No API key configured for provider: ${provider}` }, 400);
  }

  // Set up SSE stream
  const stream = new ReadableStream({
    async start(controller) {
      const encoder = new TextEncoder();

      try {
        const res = await fetch(`${baseUrl}/v1/chat/completions`, {
          method: 'POST',
          headers: {
            'Content-Type': 'application/json',
            'Authorization': `Bearer ${apiKey}`,
          },
          body: JSON.stringify({
            model: modelId,
            messages: [{ role: 'user', content: prompt }],
            stream: true,
          }),
        });

        if (!res.ok) {
          const text = await res.text().catch(() => 'Unknown error');
          controller.enqueue(encoder.encode(`data: ${JSON.stringify({ type: 'error', error: text })}\n\n`));
          controller.enqueue(encoder.encode('data: [DONE]\n\n'));
          controller.close();
          return;
        }

        if (!res.body) {
          controller.enqueue(encoder.encode(`data: ${JSON.stringify({ type: 'error', error: 'Empty response body' })}\n\n`));
          controller.enqueue(encoder.encode('data: [DONE]\n\n'));
          controller.close();
          return;
        }

        const reader = res.body.getReader();
        const decoder = new TextDecoder();
        let buffer = '';

        while (true) {
          const { done, value } = await reader.read();
          if (done) break;

          buffer += decoder.decode(value, { stream: true });
          const lines = buffer.split('\n');
          buffer = lines.pop() || '';

          for (const line of lines) {
            const trimmed = line.trim();
            if (!trimmed.startsWith('data: ')) continue;
            const data = trimmed.slice(6);
            if (data === '[DONE]') continue;

            try {
              const chunk = JSON.parse(data);
              const choice = chunk.choices?.[0];
              const delta = choice?.delta;
              if (delta?.content) {
                controller.enqueue(encoder.encode(`data: ${JSON.stringify({ type: 'text_delta', text: delta.content })}\n\n`));
              }
            } catch {
              // Ignore malformed chunks
            }
          }
        }

        controller.enqueue(encoder.encode('data: [DONE]\n\n'));
        controller.close();
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        controller.enqueue(encoder.encode(`data: ${JSON.stringify({ type: 'error', error: message })}\n\n`));
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
