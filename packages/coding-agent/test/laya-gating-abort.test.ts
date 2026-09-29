import { afterEach, describe, expect, it, vi } from "bun:test";
import { checkToolCallGating } from "../src/core/harvest/laya-gating";
import { LayaClient, type LayaClient as LayaClientType } from "../src/core/harvest/laya-client";
import { Settings } from "../src/config/settings";

/**
 * Contracts under test:
 * - An already-aborted turn settles tool gating fail-CLOSED immediately
 *   without touching the sidecar.
 * - A live turn signal is threaded through to client.decide.
 * - Aborting mid-flight settles a real decide call to the
 *   operation_cancelled fallback instead of riding out the sidecar timeout.
 */

afterEach(() => {
	vi.restoreAllMocks();
});

describe("checkToolCallGating abort propagation", () => {
	it("settles fail-CLOSED fast without a sidecar call when already aborted", async () => {
		const decide = vi.fn(() => new Promise<never>(() => {}));
		const mockClient = { decide } as unknown as LayaClientType;
		const settings = Settings.isolated({ "laya.enabled": true });
		const controller = new AbortController();
		controller.abort();

		const start = performance.now();
		const decision = await checkToolCallGating(
			"bash",
			{ command: "rm -rf /tmp/x" },
			{
				client: mockClient,
				settings,
				signal: controller.signal,
			},
		);
		const elapsed = performance.now() - start;

		expect(decision.isHighRiskTool).toBe(true);
		expect(decision.requireApproval).toBe(true);
		expect(decision.fallback).toBe(true);
		expect(decision.reason).toBe("fallback_operation_cancelled");
		expect(decide).not.toHaveBeenCalled();
		expect(elapsed).toBeLessThan(1000);
	});

	it("threads a live turn signal through to client.decide", async () => {
		const decide = vi.fn(async (_state: unknown, _questions: unknown, _metadata: { signal?: AbortSignal }) => ({
			success: true,
			fallback: false,
			data: { irreversibility: { type: "noul", noul: 0.1, confidence: 0.95 } },
			latencyMs: 5,
		}));
		const mockClient = { decide } as unknown as LayaClientType;
		const settings = Settings.isolated({ "laya.enabled": true });
		const controller = new AbortController();

		const decision = await checkToolCallGating(
			"write",
			{ path: "/tmp/x" },
			{
				client: mockClient,
				settings,
				signal: controller.signal,
			},
		);

		expect(decide).toHaveBeenCalledTimes(1);
		const metadata = decide.mock.calls[0]?.[2] as { signal?: AbortSignal };
		expect(metadata.signal).toBe(controller.signal);
		expect(decision.requireApproval).toBe(false);
		expect(decision.fallback).toBe(false);
	});
});

describe("LayaClient decide abort propagation", () => {
	let server: ReturnType<typeof Bun.serve> | undefined;

	afterEach(() => {
		server?.stop(true);
		server = undefined;
	});

	it("settles a hung sidecar call to operation_cancelled when the turn aborts", async () => {
		let hits = 0;
		server = Bun.serve({
			hostname: "127.0.0.1",
			port: 0,
			async fetch() {
				hits++;
				await Bun.sleep(30_000);
				return Response.json({ answers: {}, latency_ms: 1, model: "test" });
			},
		});
		const client = new LayaClient({
			baseUrl: server.url.toString().replace(/\/$/, ""),
			timeoutMs: 30_000,
		});
		const questions = { irreversibility: { type: "noul" as const, instructions: "is this irreversible?" } };
		const controller = new AbortController();

		const start = performance.now();
		const pending = client.decide("write file", questions, { callSite: "test", signal: controller.signal });
		await Bun.sleep(50);
		controller.abort();
		const result = await pending;
		const elapsed = performance.now() - start;

		expect(hits).toBe(1);
		expect(result.fallback).toBe(true);
		expect(result.fallbackReason).toBe("operation_cancelled");
		expect(elapsed).toBeLessThan(5000);
	});
});
