/**
 * Internal runtime helpers consumed by the generated server entry point.
 *
 * This subpath is NOT part of the public API. It exists solely so the build
 * plugins (Node, Cloudflare) can emit stable bare-specifier imports that
 * resolve through normal package-exports resolution at both build time and
 * runtime, for both workspace-linked and published-npm installs.
 *
 * User agent code should never import from here.
 */
import { getModel, type Api, type KnownProvider, type Model } from '@mariozechner/pi-ai';
import {
	CLOUDFLARE_MODEL_PREFIX,
	createCloudflareAIBindingModel,
} from './cloudflare-model.ts';
import type { FlueModelDefinition } from './config.ts';
import type { ModelConfig, ProviderSettings, ProvidersConfig } from './types.ts';

export { createFlueContext } from './client.ts';
export type { FlueContextConfig, FlueContextInternal } from './client.ts';
export { InMemorySessionStore } from './session.ts';
export { bashFactoryToSessionEnv } from './sandbox.ts';

// Error framework. Re-exported here for the build-plugin templates. Trimmed
// to only the names the templates actually import — anything thrown
// transitively by the helpers (e.g. UnsupportedMediaTypeError thrown inside
// parseJsonBody) is bundled via static imports inside error-utils.ts and
// doesn't need to appear on this surface. If a future template needs more,
// add it here at that time.
export { parseJsonBody, toHttpResponse, toSseData, validateAgentRequest } from './error-utils.ts';
export {
	AgentNotFoundError,
	InvalidRequestError,
	MethodNotAllowedError,
	RouteNotFoundError,
} from './errors.ts';

/**
 * Resolve a `provider/model-id` string into a pi-ai `Model` object.
 * Lives here (rather than in the generated entry point) so that user
 * projects don't have to declare `@mariozechner/pi-ai` as a direct
 * dependency — wrangler's bundler resolves bare specifiers from the entry
 * file's location, which on pnpm-isolated installs doesn't see Flue's
 * transitive deps. Centralizing the resolver here keeps `_entry.ts`
 * dependency-free apart from `@zene/agent/*`.
 *
 * Resolution order (highest priority first):
 *
 *   1. User-defined `models` from `flue.config.ts`. Keyed by bare provider
 *      name (the part of the model string before the first `/`).
 *      Last-write-wins on collision with built-ins — same semantics as
 *      pi-ai's `registerApiProvider`.
 *   2. The reserved `cloudflare/` prefix (Workers AI binding).
 *   3. pi-ai's static catalog via `getModel`.
 *
 * `userModels` is undefined for legacy callers and for `flue run --target node`
 * server processes that haven't been re-bundled yet; both fall through to the
 * built-in branches without behavior change.
 */
export function resolveModel(
	model: ModelConfig | undefined,
	providers?: ProvidersConfig,
	userModels?: Record<string, FlueModelDefinition>,
): Model<Api> | undefined {
	if (model === false || model === undefined) return undefined;

	const modelString = model;

	const slash = modelString.indexOf('/');
	if (slash === -1) {
		throw new Error(
			`[zene] Invalid model "${modelString}". ` +
				`Use the "provider/model-id" format (e.g. "anthropic/claude-haiku-4-5").`,
		);
	}
	const provider = modelString.slice(0, slash);
	const modelId = modelString.slice(slash + 1);

	// 1. User-defined models from flue.config.ts. Consulted first so users
	//    can shadow built-ins (e.g. register their own "cloudflare" or
	//    "anthropic" routing) — matches pi-ai's last-write-wins behavior.
	const userDef = userModels?.[provider];
	if (userDef) {
		if (!modelId) {
			throw new Error(
				`[zene] Invalid model "${modelString}". ` +
					`The "${provider}/" prefix is registered in flue.config.ts, but no model id ` +
					`was given. Use "${provider}/<model-id>".`,
			);
		}
		// Resolve the override key once, here. The user can pin a different
		// `provider` on the definition (e.g. for shared overrides across
		// multiple prefixes); otherwise it falls back to the map key.
		// `init({ providers: { ollama: { baseUrl } } })` keys off this.
		const resolvedProvider = userDef.provider ?? provider;
		const built = buildUserModel(userDef, resolvedProvider, modelId);
		return applyProviderSettings(built, providers?.[resolvedProvider]);
	}

	// 2. Reserved `cloudflare/` prefix. Routes through the Workers AI binding;
	//    the API handler is only registered on the Cloudflare target, so
	//    node-target use fails at dispatch time with pi-ai's "no API provider
	//    registered" error.
	if (modelString.startsWith(CLOUDFLARE_MODEL_PREFIX)) {
		const workersAiModelId = modelString.slice(CLOUDFLARE_MODEL_PREFIX.length);
		if (!workersAiModelId) {
			throw new Error(
				`[zene] Invalid model "${modelString}". ` +
					`Use "cloudflare/<workers-ai-model-id>" (e.g. "cloudflare/@cf/moonshotai/kimi-k2.6").`,
			);
		}
		// `providers.cloudflare` settings are not applied: the binding owns
		// transport and ignores baseUrl/headers/apiKey. For gateway-based
		// observability, use pi-ai's `cloudflare-ai-gateway` provider.
		return createCloudflareAIBindingModel(workersAiModelId);
	}

	// 3. pi-ai catalog. `getModel` is overloaded on literal provider/modelId;
	//    we cast through runtime strings and rely on the null-return check
	//    below for unknowns.
	const resolved = getModel(provider as KnownProvider, modelId as never);
	if (!resolved) {
		throw new Error(
			`[zene] Unknown model "${modelString}". ` +
				`Provider "${provider}" / model id "${modelId}" ` +
				`is not registered with @mariozechner/pi-ai.`,
		);
	}
	return applyProviderSettings(resolved, providers?.[provider]);
}

/**
 * Construct a pi-ai `Model` literal from a user-supplied `FlueModelDefinition`,
 * the resolved provider name (caller chose between `def.provider` and the
 * map key), and the suffix of the model string (everything after the first
 * `/`).
 *
 * Cost / context-window fields are zeroed because no static catalog exists
 * for user-defined providers; Flue features that read those (cost display,
 * overflow detection) degrade gracefully, exactly like the `cloudflare/`
 * branch.
 */
function buildUserModel(
	def: FlueModelDefinition,
	provider: string,
	modelId: string,
): Model<Api> {
	switch (def.kind) {
		case 'openai-completions': {
			return {
				id: modelId,
				name: modelId,
				api: 'openai-completions',
				provider,
				baseUrl: def.baseUrl,
				reasoning: false,
				input: ['text'],
				cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
				contextWindow: 0,
				maxTokens: 0,
				headers: def.headers,
			};
		}
		default: {
			// Exhaustive check. Adding a new `kind` to the union without a
			// matching case here is a type error.
			const _exhaustive: never = def.kind;
			throw new Error(`[zene] Unknown user model kind: ${String(_exhaustive)}`);
		}
	}
}

function applyProviderSettings<TApi extends Api>(
	model: Model<TApi>,
	providerSettings: ProviderSettings | undefined,
): Model<TApi> {
	if (!providerSettings) return model;

	const hasBaseUrl = providerSettings.baseUrl !== undefined;
	const hasHeaders = providerSettings.headers !== undefined;
	if (!hasBaseUrl && !hasHeaders) return model;

	return {
		...model,
		baseUrl: providerSettings.baseUrl ?? model.baseUrl,
		headers: hasHeaders ? { ...(model.headers ?? {}), ...providerSettings.headers } : model.headers,
	};
}
