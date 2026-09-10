import * as path from "node:path";
import { YAML } from "bun";
import { getAgentDir, logger } from "@harvest/pi-utils";
import type { ModelRegistry } from "./model-registry";
import type { Settings } from "./settings";

export async function configureCustomProvider(
	baseUrl: string,
	apiKey?: string,
	modelRegistry?: ModelRegistry,
	settings?: Settings,
	providerId = "custom",
): Promise<void> {
	const agentDir = getAgentDir();
	const modelsPath = path.join(agentDir, "models.yml");
	let config: Record<string, unknown> = {};
	try {
		const content = await Bun.file(modelsPath).text();
		config = (YAML.parse(content) as Record<string, unknown>) || {};
	} catch {
		config = {};
	}
	if (!config || typeof config !== "object") config = {};
	const providers = (config.providers && typeof config.providers === "object" ? config.providers : {}) as Record<
		string,
		unknown
	>;
	config.providers = providers;

	let cleanBaseUrl = baseUrl.trim().replace(/\/+$/, "");
	try {
		const parsed = new URL(cleanBaseUrl);
		if (parsed.pathname === "" || parsed.pathname === "/") {
			cleanBaseUrl = `${cleanBaseUrl}/v1`;
		}
	} catch {
		// Ignore URL parsing errors and keep input as-is
	}
	const cleanApiKey = apiKey?.trim();

	const existing = providers[providerId] as { models?: unknown[] } | undefined;
	providers[providerId] = {
		baseUrl: cleanBaseUrl,
		...(cleanApiKey ? { apiKey: cleanApiKey } : { auth: "none" }),
		api: "openai-completions",
		discovery: {
			type: "openai-models-list",
		},
		models: existing?.models?.length
			? existing.models
			: [
					{
						id: "auto",
						name: "Auto (Router)",
						reasoning: false,
						input: ["text", "image"],
						contextWindow: 128000,
						maxTokens: 16384,
					},
				],
	};

	const yamlStr = YAML.stringify(config, null, 2);
	await Bun.write(modelsPath, yamlStr);

	if (modelRegistry?.authStorage && cleanApiKey) {
		try {
			await modelRegistry.authStorage.set(providerId, { type: "api_key", key: cleanApiKey });
		} catch (err) {
			logger.warn("Failed to save custom provider API key to authStorage", { err: String(err) });
		}
	}

	if (modelRegistry) {
		try {
			await modelRegistry.refresh("online-if-uncached");
		} catch (err) {
			logger.warn("Failed to refresh model registry after configuring custom provider", { err: String(err) });
		}
	}

	if (settings) {
		try {
			settings.setModelRole("default", `${providerId}/auto`);
			await settings.flush();
		} catch (err) {
			logger.warn("Failed to update default model setting", { err: String(err) });
		}
	}
}
