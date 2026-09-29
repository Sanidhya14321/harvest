import { afterAll, afterEach, describe, expect, it, vi } from "bun:test";
import * as fs from "node:fs/promises";
import * as os from "node:os";
import * as path from "node:path";
import { ModelRegistry } from "../../src/config/model-registry";
import { Settings } from "../../src/config/settings";
import type { AgentSession } from "../../src/session/agent-session";
import { openLiveAgentSession } from "../../src/session/live-session-factory";
import type { CreateAgentSessionResult } from "../../src/sdk";
import { createInMemoryAuthStorage } from "../helpers/agent-session-setup";
import type { AuthStorage } from "../../src/session/auth-storage";
import type { Model } from "@harvest/pi-ai";

/**
 * Cold-open factory contracts: every live tab gets a cloned settings object
 * (session overrides never leak sideways), a dedicated session manager
 * (opened from disk for reopens), the inherited model, a unique agent id,
 * and fresh event buses — while credentials and model routing stay shared.
 * The session creator is injected so these tests never run real discovery.
 */
describe("openLiveAgentSession", () => {
	let authStorage: AuthStorage;
	let modelRegistry: ModelRegistry;
	const roots: string[] = [];

	afterAll(() => {
		authStorage.close();
	});

	afterEach(async () => {
		vi.restoreAllMocks();
		await Promise.all(roots.splice(0).map(root => fs.rm(root, { recursive: true, force: true })));
	});

	function harness() {
		authStorage = createInMemoryAuthStorage();
		authStorage.setRuntimeApiKey("mock", "test-key");
		modelRegistry = new ModelRegistry(authStorage);
	}

	function fakeSession(): AgentSession {
		return {} as unknown as AgentSession;
	}

	it("opens a fresh tab with cloned settings and shared routing", async () => {
		harness();
		const root = await fs.mkdtemp(path.join(os.tmpdir(), "harvest-live-open-"));
		roots.push(root);
		const parentSettings = Settings.isolated({ "compaction.enabled": false });
		const model = { provider: "mock", id: "mock-model" } as unknown as Model;
		const seen: Record<string, unknown>[] = [];
		const created = fakeSession();
		const createSession = async (options: Record<string, unknown>) => {
			seen.push(options);
			return { session: created } as unknown as CreateAgentSessionResult;
		};

		const session = await openLiveAgentSession({
			cwd: root,
			sessionDir: root,
			settings: parentSettings,
			authStorage,
			modelRegistry,
			model,
			createSession,
		});

		expect(session).toBe(created);
		expect(seen).toHaveLength(1);
		const options = seen[0]!;
		expect(options.model).toBe(model);
		expect(options.authStorage).toBe(authStorage);
		expect(options.modelRegistry).toBe(modelRegistry);
		expect(options.hasUI).toBe(true);
		expect(options.rebindModelAfterDiscovery).toBe(true);
		const childSettings = options.settings as Settings;
		expect(childSettings).not.toBe(parentSettings);
		expect(childSettings.get("compaction.enabled")).toBe(false);
		const manager = options.sessionManager as {
			getSessionId: () => string;
			getSessionFile: () => string | undefined;
		};
		expect(typeof manager.getSessionId()).toBe("string");
		const freshFile = manager.getSessionFile();
		expect(freshFile?.startsWith(root)).toBe(true);
		expect(options.agentId).toBe(`tab:${manager.getSessionId()}`);
		expect(options.eventBus).toBeDefined();
	});

	it("cold-opens an existing file with its recorded session", async () => {
		harness();
		const root = await fs.mkdtemp(path.join(os.tmpdir(), "harvest-live-open-"));
		roots.push(root);
		const sessionPath = path.join(root, "2026-01-01T00-00-00_old.jsonl");
		await Bun.write(sessionPath, "");
		const parentSettings = Settings.isolated({ "compaction.enabled": false });
		const model = { provider: "mock", id: "mock-model" } as unknown as Model;
		const seen: Record<string, unknown>[] = [];
		const created = fakeSession();

		const session = await openLiveAgentSession({
			cwd: root,
			sessionDir: root,
			sessionPath,
			settings: parentSettings,
			authStorage,
			modelRegistry,
			model,
			agentId: "tab:custom",
			createSession: async options => {
				seen.push(options);
				return { session: created } as unknown as CreateAgentSessionResult;
			},
		});

		expect(session).toBe(created);
		const options = seen[0]!;
		const manager = options.sessionManager as { getSessionFile: () => string | undefined };
		expect(manager.getSessionFile()).toBe(sessionPath);
		expect(options.agentId).toBe("tab:custom");
	});
});
