import { describe, expect, it, vi } from "bun:test";
import { checkCompletionWithLaya } from "../src/core/harvest/laya-completion";
import type { LayaClient } from "../src/core/harvest/laya-client";
import { Settings } from "../src/config/settings";

/**
 * P1-10: the completion check asks only the needed question, carries task
 * context, and honors turn cancellation instead of waiting out the timeout.
 */
describe("Laya completion check", () => {
	function enabledSettings() {
		return Settings.isolated({ "laya.enabled": true });
	}

	function mockClient(answer: Record<string, unknown>) {
		const decide = vi.fn(
			async (
				_state: unknown,
				_questions: Record<string, unknown>,
			): Promise<{ success: boolean; fallback: boolean; latencyMs: number; data: Record<string, unknown> }> => ({
				success: true,
				fallback: false,
				latencyMs: 5,
				data: answer,
			}),
		);
		return { client: { decide } as unknown as LayaClient, decide };
	}

	it("sends only the requested question and leaves the unasked dimension open", async () => {
		const { client, decide } = mockClient({ unexpected_stop: { type: "noul", noul: 0.9, confidence: 0.8 } });
		const result = await checkCompletionWithLaya(
			{ assistantText: "I will now do three more things" },
			{ client, settings: enabledSettings(), checks: ["unexpected_stop"] },
		);

		expect(decide).toHaveBeenCalledTimes(1);
		expect(Object.keys(decide.mock.calls[0]?.[1] ?? {})).toEqual(["unexpected_stop"]);
		expect(result.fallback).toBe(false);
		expect(result.isPrematureStop).toBe(true);
		expect(result.isSuccess).toBe(true);
	});

	it("carries task context in the scored state", async () => {
		const { client, decide } = mockClient({ unexpected_stop: { type: "noul", noul: 0.1, confidence: 0.9 } });
		await checkCompletionWithLaya(
			{ assistantText: "done", taskContext: "migrate the billing tables" },
			{ client, settings: enabledSettings(), checks: ["unexpected_stop"] },
		);

		const state = decide.mock.calls[0]?.[0] as unknown as Record<string, string>;
		expect(state.task_context).toBe("migrate the billing tables");
	});

	it("settles immediately on an aborted turn without touching the sidecar", async () => {
		const { client, decide } = mockClient({});
		const controller = new AbortController();
		controller.abort();
		const result = await checkCompletionWithLaya(
			{ assistantText: "done" },
			{ client, settings: enabledSettings(), signal: controller.signal },
		);

		expect(decide).not.toHaveBeenCalled();
		expect(result.fallback).toBe(true);
		expect(result.fallbackReason).toBe("operation_cancelled");
		expect(result.isPrematureStop).toBe(false);
	});
});
