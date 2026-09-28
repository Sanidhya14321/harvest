import { afterEach, describe, expect, it, vi } from "bun:test";
import { selectSubagentWithLaya } from "../src/core/harvest/laya-subagent-selection";
import type { LayaClient } from "../src/core/harvest/laya-client";
import type { AgentDefinition } from "../src/task/types";
import { Settings } from "../src/config/settings";

/**
 * Contracts under test:
 *  - An already-aborted parent signal settles selectSubagentWithLaya
 *    immediately to a fallback without touching the sidecar.
 *  - A live parent signal is threaded through to client.decide (which already
 *    supports it on the pruning path).
 */

const TEST_AGENTS: AgentDefinition[] = [
	{ name: "scout", description: "Fast exploration agent", systemPrompt: "You are scout", source: "bundled" },
	{ name: "task", description: "General-purpose agent", systemPrompt: "You are task", source: "bundled" },
];

afterEach(() => {
	vi.restoreAllMocks();
});

describe("selectSubagentWithLaya abort propagation", () => {
	it("settles fast to a fallback when the parent signal is already aborted", async () => {
		const decide = vi.fn(() => new Promise<never>(() => {}));
		const mockClient = { decide, isHealthy: vi.fn(async () => true) } as unknown as LayaClient;
		const settings = Settings.isolated({ "laya.enabled": true, "laya.subagentSelection": true });
		const controller = new AbortController();
		controller.abort();

		const start = performance.now();
		const decision = await selectSubagentWithLaya("Find where git status is parsed", {
			client: mockClient,
			settings,
			availableAgents: TEST_AGENTS,
			defaultAgent: "task",
			signal: controller.signal,
		});
		const elapsed = performance.now() - start;

		expect(decision.decisionType).toBe("fallback");
		expect(decision.fallback).toBe(true);
		expect(decision.fallbackReason).toBe("operation_cancelled");
		expect(decision.selectedAgent).toBe("task");
		expect(decide).not.toHaveBeenCalled();
		expect(elapsed).toBeLessThan(1000);
	});

	it("threads a live parent signal through to client.decide", async () => {
		const decide = vi.fn(async (_state: unknown, _questions: unknown, _metadata: { signal?: AbortSignal }) => ({
			success: true,
			data: { subagent_choice: { answer: "scout", confidence: 0.95 } },
			fallback: false,
			latencyMs: 5,
		}));
		const mockClient = { decide, isHealthy: vi.fn(async () => true) } as unknown as LayaClient;
		const settings = Settings.isolated({ "laya.enabled": true, "laya.subagentSelection": true });
		const controller = new AbortController();

		const decision = await selectSubagentWithLaya("Find where git status is parsed", {
			client: mockClient,
			settings,
			availableAgents: TEST_AGENTS,
			defaultAgent: "task",
			signal: controller.signal,
		});

		expect(decide).toHaveBeenCalledTimes(1);
		const metadata = decide.mock.calls[0]?.[2];
		expect(metadata.signal).toBe(controller.signal);
		expect(decision.decisionType).toBe("auto_pick");
		expect(decision.selectedAgent).toBe("scout");
	});
});
