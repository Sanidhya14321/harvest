import { describe, expect, it } from "bun:test";
import * as path from "node:path";
import { YAML } from "bun";
import { getAgentDir } from "@harvest/pi-utils";
import { configureCustomProvider } from "../src/config/custom-provider";

describe("configureCustomProvider", () => {
	it("saves custom provider configuration to models.yml with apiKey and default model", async () => {
		const testProviderId = "test-custom-prov";
		await configureCustomProvider(
			"https://api.example.com/v1",
			"sk-secret-key-123",
			undefined,
			undefined,
			testProviderId,
		);

		const agentDir = getAgentDir();
		const modelsPath = path.join(agentDir, "models.yml");
		const content = await Bun.file(modelsPath).text();
		const parsed = YAML.parse(content) as Record<string, any>;

		expect(parsed.providers).toBeDefined();
		expect(parsed.providers[testProviderId]).toBeDefined();
		expect(parsed.providers[testProviderId].baseUrl).toBe("https://api.example.com/v1");
		expect(parsed.providers[testProviderId].apiKey).toBe("sk-secret-key-123");
		expect(parsed.providers[testProviderId].api).toBe("openai-completions");
		expect(parsed.providers[testProviderId].discovery.type).toBe("openai-models-list");
		expect(parsed.providers[testProviderId].models[0].id).toBe("auto");

		// Clean up test entry
		delete parsed.providers[testProviderId];
		await Bun.write(modelsPath, YAML.stringify(parsed, null, 2));
	});

	it("saves custom provider configuration with auth: none when apiKey is omitted", async () => {
		const testProviderId = "test-custom-noauth";
		await configureCustomProvider("http://localhost:8080/v1/", "", undefined, undefined, testProviderId);

		const agentDir = getAgentDir();
		const modelsPath = path.join(agentDir, "models.yml");
		const content = await Bun.file(modelsPath).text();
		const parsed = YAML.parse(content) as Record<string, any>;

		expect(parsed.providers[testProviderId]).toBeDefined();
		expect(parsed.providers[testProviderId].baseUrl).toBe("http://localhost:8080/v1");
		expect(parsed.providers[testProviderId].auth).toBe("none");
		expect(parsed.providers[testProviderId].apiKey).toBeUndefined();

		// Clean up test entry
		delete parsed.providers[testProviderId];
		await Bun.write(modelsPath, YAML.stringify(parsed, null, 2));
	});

	it("normalizes baseUrl by appending /v1 when bare host is provided and saves to authStorage", async () => {
		const testProviderId = "test-custom-normalize";
		const mockAuthStorage = {
			savedKeys: new Map<string, string>(),
			setApiKey: async (p: string, k: string) => {
				mockAuthStorage.savedKeys.set(p, k);
			},
		};
		const mockModelRegistry = {
			authStorage: mockAuthStorage,
			refresh: async () => {},
		};

		await configureCustomProvider(
			"http://localhost:11434",
			"my-key",
			mockModelRegistry as any,
			undefined,
			testProviderId,
		);

		const agentDir = getAgentDir();
		const modelsPath = path.join(agentDir, "models.yml");
		const content = await Bun.file(modelsPath).text();
		const parsed = YAML.parse(content) as Record<string, any>;

		expect(parsed.providers[testProviderId]).toBeDefined();
		expect(parsed.providers[testProviderId].baseUrl).toBe("http://localhost:11434/v1");
		expect(parsed.providers[testProviderId].apiKey).toBe("my-key");
		expect(mockAuthStorage.savedKeys.get(testProviderId)).toBe("my-key");

		delete parsed.providers[testProviderId];
		await Bun.write(modelsPath, YAML.stringify(parsed, null, 2));
	});
});
