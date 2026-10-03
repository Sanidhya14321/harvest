import { afterEach, beforeEach, describe, expect, it } from "bun:test";
import { LayaClient } from "../src/core/harvest/laya-client";
import {
	getDecisionStats,
	isBreakerOpen,
	recordDecisionOutcome,
	resetDecisionBreaker,
} from "../src/core/harvest/laya-circuit";

/**
 * P2-1: after repeated consecutive sidecar failures, optional decision
 * points fail fast without another HTTP round trip; required gating always
 * attempts. Statistics stay queryable per call site.
 */
describe("Laya decision circuit breaker", () => {
	let server: ReturnType<typeof Bun.serve> | undefined;
	let hits = 0;

	beforeEach(() => {
		resetDecisionBreaker();
		hits = 0;
	});

	afterEach(() => {
		try {
			server?.stop(true);
		} catch {}
		server = undefined;
		resetDecisionBreaker();
	});

	function hangingClient(): LayaClient {
		server = Bun.serve({
			hostname: "127.0.0.1",
			port: 0,
			fetch: () => {
				hits++;
				return new Promise<Response>(() => {});
			},
		});
		return new LayaClient({ baseUrl: server.url.toString().replace(/\/$/, ""), timeoutMs: 50, authToken: "test" });
	}

	it("fails fast for optional points after consecutive timeouts without more HTTP", async () => {
		const client = hangingClient();
		for (let i = 0; i < 5; i++) {
			const result = await client.decide(
				"state",
				{ q: { type: "noul", instructions: "ok?" } },
				{
					callSite: "context_pruning",
				},
			);
			expect(result.fallback).toBe(true);
		}
		expect(hits).toBe(5);
		expect(isBreakerOpen("context_pruning")).toBe(true);

		const start = performance.now();
		const skipped = await client.decide(
			"state",
			{ q: { type: "noul", instructions: "ok?" } },
			{
				callSite: "context_pruning",
			},
		);
		expect(skipped.fallbackReason).toBe("breaker_open_sidecar_overloaded");
		expect(hits).toBe(5);
		expect(performance.now() - start).toBeLessThan(50);
	});

	it("never skips required tool gating, however often it fails", async () => {
		const client = hangingClient();
		for (let i = 0; i < 6; i++) {
			const result = await client.decide(
				"rm -rf /",
				{ q: { type: "noul", instructions: "ok?" } },
				{
					callSite: "tool_gating",
				},
			);
			expect(result.fallback).toBe(true);
		}
		expect(hits).toBe(6);
		expect(isBreakerOpen("tool_gating")).toBe(false);
	});

	it("resets the consecutive count on success", () => {
		for (let i = 0; i < 4; i++) {
			recordDecisionOutcome("brain_retrieval", {
				success: false,
				fallbackReason: "timeout_exceeded_50ms",
				latencyMs: 50,
			});
		}
		expect(isBreakerOpen("brain_retrieval")).toBe(false);
		recordDecisionOutcome("brain_retrieval", { success: true, latencyMs: 10 });
		for (let i = 0; i < 4; i++) {
			recordDecisionOutcome("brain_retrieval", {
				success: false,
				fallbackReason: "timeout_exceeded_50ms",
				latencyMs: 50,
			});
		}
		expect(isBreakerOpen("brain_retrieval")).toBe(false);
		const stats = getDecisionStats("brain_retrieval");
		expect(stats.calls).toBe(9);
		expect(stats.fallbacks).toBe(8);
		expect(stats.timeouts).toBe(8);
		expect(stats.consecutiveFailures).toBe(4);
	});

	it("reports latency percentiles over a bounded window", () => {
		for (let i = 1; i <= 10; i++) {
			recordDecisionOutcome("completion_check", { success: true, latencyMs: i * 10 });
		}
		const stats = getDecisionStats("completion_check");
		expect(stats.calls).toBe(10);
		expect(stats.p50Ms).toBe(60);
		expect(stats.p95Ms).toBe(100);
	});

	it("ignores health-neutral fallbacks for breaker counting", () => {
		for (let i = 0; i < 10; i++) {
			recordDecisionOutcome("model_routing", {
				success: false,
				fallbackReason: "operation_cancelled",
				latencyMs: 1,
			});
		}
		expect(isBreakerOpen("model_routing")).toBe(false);
		expect(getDecisionStats("model_routing").consecutiveFailures).toBe(0);
	});
});
