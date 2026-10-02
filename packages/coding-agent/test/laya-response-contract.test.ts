import { afterEach, describe, expect, it } from "bun:test";
import { LayaClient, type LayaQuestionDefinition } from "../src/core/harvest/laya-client";
import { checkToolCallGating } from "../src/core/harvest/laya-gating";
import { Settings } from "../src/config/settings";

describe("Laya HTTP decision contract", () => {
	let server: ReturnType<typeof Bun.serve> | undefined;
	afterEach(() => server?.stop(true));
	function clientFor(answers: unknown, extra: Record<string, unknown> = {}) {
		server = Bun.serve({
			hostname: "127.0.0.1",
			port: 0,
			fetch: () => Response.json({ answers, model: "test", latency_ms: 1, ...extra }),
		});
		return new LayaClient({ baseUrl: server.url.toString().replace(/\/$/, ""), authToken: "test" });
	}
	const noul = { risk: { type: "noul" as const, instructions: "Evaluate risk" } };
	it.each([
		["missing answer", {}],
		["null answer", { risk: null }],
		["wrong type", { risk: { type: "choice", noul: 0, confidence: 1 } }],
		["string confidence", { risk: { type: "noul", noul: 0, confidence: "1" } }],
		["out of range noul", { risk: { type: "noul", noul: -1, confidence: 1 } }],
		["invalid distribution", { risk: { type: "noul", noul: 0, confidence: 1, probabilities: { yes: 2 } } }],
	])("falls back for %s without exposing malformed answers", async (_name, answers) => {
		const result = await clientFor(answers).decide("English state", noul);
		expect(result.success).toBe(false);
		expect(result.fallbackReason).toBe("invalid_decision_response");
		expect(result.data).toBeUndefined();
	});
	it("rejects a choice outside the requested options", async () => {
		const questions: Record<string, LayaQuestionDefinition> = {
			route: { type: "choice", instructions: "Route", criteria: { cheap: "Simple", capable: "Complex" } },
		};
		const result = await clientFor({ route: { type: "choice", choice: "unknown", confidence: 1 } }).decide(
			"Task",
			questions,
		);
		expect(result.fallbackReason).toBe("invalid_decision_response");
	});
	it("rejects a score outside the request's ordinal range", async () => {
		const result = await clientFor({ relevance: { type: "score", score: 2, confidence: 1 } }).decide("Task", {
			relevance: { type: "score", instructions: "Evaluate", criteria: ["irrelevant", "relevant"] },
		});
		expect(result.fallbackReason).toBe("invalid_decision_response");
	});
	it("preserves the non-English fallback when the service intentionally omits answers", async () => {
		const result = await clientFor({}, { non_english: true }).decide("English request", noul);
		expect(result.fallbackReason).toBe("non_english_state");
	});
	it("requires approval for malformed confidence delivered by an HTTP sidecar", async () => {
		const result = await checkToolCallGating(
			"write",
			{ path: "a.ts", content: "x" },
			{
				client: clientFor({ irreversibility: { type: "noul", noul: 0, confidence: 8 } }),
				settings: Settings.isolated({ "laya.enabled": true }),
			},
		);
		expect(result.requireApproval).toBe(true);
		expect(result.reason).toBe("fallback_invalid_decision_response");
	});
});
