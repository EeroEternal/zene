// Lives at the SDK root, not in `cloudflare/`, so `internal.ts` can import it
// without dragging in `node:async_hooks` (transitive via getCloudflareContext).
import type { Model } from '@mariozechner/pi-ai';

/** Pi-ai `Api` slug for the binding-backed Workers AI provider. */
export const CLOUDFLARE_AI_BINDING_API = 'cloudflare-ai-binding' as const;
export type CloudflareAIBindingApi = typeof CLOUDFLARE_AI_BINDING_API;

/** Provider name surfaced on AssistantMessage records and usage logs. */
export const CLOUDFLARE_AI_BINDING_PROVIDER = 'workers-ai' as const;

/** Reserved Flue model-string prefix that routes to the binding provider. */
export const CLOUDFLARE_MODEL_PREFIX = 'cloudflare/';

/**
 * Cost / context-window / reasoning fields are zeroed because we don't
 * maintain a static catalog of Workers AI models. Flue features that read
 * this metadata (cost display, overflow detection) degrade gracefully.
 */
export function createCloudflareAIBindingModel(
	workersAiModelId: string,
): Model<CloudflareAIBindingApi> {
	return {
		id: workersAiModelId,
		name: workersAiModelId,
		api: CLOUDFLARE_AI_BINDING_API,
		provider: CLOUDFLARE_AI_BINDING_PROVIDER,
		// Unused: the binding handles transport. Required by `Model`.
		baseUrl: '',
		reasoning: false,
		input: ['text'],
		cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
		contextWindow: 0,
		maxTokens: 0,
	};
}
