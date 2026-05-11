/**
 * Configuration handlers - get/set LLM provider config
 *
 * In production, use D1 or KV for persistent storage.
 * This uses in-memory storage for simplicity.
 */
import type { Context } from 'hono';
import { getProviderConfig, setProviderConfig } from './agent-handler';

/**
 * GET /api/config/provider
 * Get current provider configuration
 */
export async function handleGetConfig(c: Context) {
  const config = getProviderConfig();
  return c.json({ config });
}

/**
 * POST /api/config/provider
 * Set provider configuration
 *
 * Expects JSON body:
 * {
 *   provider: 'deepseek' | 'openai' | 'anthropic' | 'custom',
 *   apiKey?: string,
 *   baseUrl?: string,
 *   gatewayUrl?: string,
 *   defaultModel?: string
 * }
 */
export async function handleSetConfig(c: Context) {
  const body = await c.req.json<{
    provider?: string;
    apiKey?: string;
    baseUrl?: string;
    gatewayUrl?: string;
    defaultModel?: string;
  }>();

  const { provider, apiKey, baseUrl, gatewayUrl, defaultModel } = body;

  // Validate
  if (!provider) {
    return c.json({ error: 'provider is required' }, 400);
  }

  // Update config
  const currentConfig = getProviderConfig();
  const newConfig = {
    ...currentConfig,
    providers: {
      ...(currentConfig.providers ?? {}),
      [provider]: {
        ...(currentConfig.providers?.[provider] ?? {}),
        ...(apiKey !== undefined && { apiKey }),
        ...(baseUrl !== undefined && { baseUrl }),
      },
    },
    ...(gatewayUrl !== undefined && { gatewayUrl }),
    ...(defaultModel !== undefined && { defaultModel }),
  };

  setProviderConfig(newConfig);

  return c.json({
    success: true,
    config: newConfig,
  });
}
