/**
 * Agent handler - handles agent prompts with SSE streaming
 *
 * Uses direct fetch to Cloudflare AI Gateway compat endpoint,
 * bypassing pi-ai OpenAI SDK streaming issues in Workers.
 */
import type { Context } from 'hono';

interface Env {
  DB: D1Database;
  AI_GATEWAY_URL?: string;
  AI_GATEWAY_TOKEN?: string;
}

// In-memory provider config (module-level)
let providerConfig: Record<string, any> = {};

export function setProviderConfig(config: Record<string, any>): void {
  providerConfig = { ...providerConfig, ...config };
}

export function getProviderConfig(): Record<string, any> {
  return { ...providerConfig };
}

// In-memory session store (replace with D1 in production)
const sessionMessages: Record<string, Array<{ role: string; content: string }>> = {};

function getSessionHistory(sessionId: string): Array<{ role: string; content: string }> {
  return sessionMessages[sessionId] || [];
}

function addMessage(sessionId: string, role: string, content: string): void {
  if (!sessionMessages[sessionId]) {
    sessionMessages[sessionId] = [];
  }
  sessionMessages[sessionId].push({ role, content });
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

  const gatewayBaseUrl = c.env.AI_GATEWAY_URL ?? providerConfig.gatewayUrl ?? '';
  const gatewayToken = c.env.AI_GATEWAY_TOKEN ?? providerConfig.gatewayToken ?? '';
  const modelStr = model ?? 'deepseek/deepseek-chat';

  if (!gatewayBaseUrl) {
    return c.json({ error: 'AI_GATEWAY_URL not configured' }, 500);
  }

  // Add user message to history
  addMessage(sessionId, 'user', prompt);

  // Build messages from history
  const messages = getSessionHistory(sessionId).map((m) => ({
    role: m.role,
    content: m.content,
  }));

  // Set up SSE stream
  const stream = new ReadableStream({
    async start(controller) {
      const encoder = new TextEncoder();
      let assistantContent = '';

      try {
        const res = await fetch(`${gatewayBaseUrl}/chat/completions`, {
          method: 'POST',
          headers: {
            'Content-Type': 'application/json',
            ...(gatewayToken ? { 'cf-aig-authorization': `Bearer ${gatewayToken}` } : {}),
          },
          body: JSON.stringify({
            model: modelStr,
            messages,
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
                assistantContent += delta.content;
                controller.enqueue(encoder.encode(`data: ${JSON.stringify({ type: 'text_delta', text: delta.content })}\n\n`));
              }
            } catch {
              // Ignore malformed chunks
            }
          }
        }

        // Save assistant message to history
        if (assistantContent) {
          addMessage(sessionId, 'assistant', assistantContent);
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
