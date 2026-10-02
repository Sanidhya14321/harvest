import { describe, expect, it, vi } from "bun:test";
import { checkToolCallGating } from "../src/core/harvest/laya-gating";
import type { LayaClient } from "../src/core/harvest/laya-client";
import { Settings } from "../src/config/settings";

/**
 * P1-9: gating eligibility follows the tool's structured approval tier,
 * not just wire names — an MCP-style write tool whose name is not listed
 * is still classified, while read-tier tools bypass without a sidecar call.
 */
describe("Laya tool gating tier eligibility", () => {
	function enabledSettings() {
		return Settings.isolated({ "laya.enabled": true });
	}

	function scoredClient() {
		const decide = vi.fn(async () => ({
			success: true,
			fallback: false,
			latencyMs: 5,
			data: { irreversibility: { type: "noul", noul: 0.1, confidence: 0.95 } },
		}));
		return { client: { decide } as unknown as LayaClient, decide };
	}

	it("classifies an unlisted write-tier tool instead of bypassing it", async () => {
		const { client, decide } = scoredClient();
		const result = await checkToolCallGating(
			"mcp__remote_shell_exec",
			{ command: "echo hi" },
			{ client, settings: enabledSettings(), toolTier: "write" },
		);

		expect(decide).toHaveBeenCalledTimes(1);
		expect(result.isHighRiskTool).toBe(true);
		expect(result.requireApproval).toBe(false);
		expect(result.fallback).toBe(false);
	});

	it("bypasses a read-tier tool without calling the sidecar", async () => {
		const { client, decide } = scoredClient();
		const result = await checkToolCallGating(
			"read",
			{ path: "src/index.ts" },
			{ client, settings: enabledSettings(), toolTier: "read" },
		);

		expect(decide).not.toHaveBeenCalled();
		expect(result.isHighRiskTool).toBe(false);
		expect(result.requireApproval).toBe(false);
		expect(result.reason).toBe("read_tier_bypass");
	});

	it("keeps the name list for callers without a tier", async () => {
		const { client, decide } = scoredClient();
		const listed = await checkToolCallGating("bash", { command: "echo hi" }, { client, settings: enabledSettings() });
		expect(decide).toHaveBeenCalledTimes(1);
		expect(listed.isHighRiskTool).toBe(true);

		const { client: client2, decide: decide2 } = scoredClient();
		const unlisted = await checkToolCallGating(
			"glob",
			{ pattern: "**/*.ts" },
			{ client: client2, settings: enabledSettings() },
		);
		expect(decide2).not.toHaveBeenCalled();
		expect(unlisted.reason).toBe("tool_not_in_high_risk_list");
	});

	it("requires human review without inference when a destructive command tail exceeds the evidence limit", async () => {
		const { client, decide } = scoredClient();
		const result = await checkToolCallGating(
			"bash",
			{ command: `${"echo harmless; ".repeat(200)}rm -rf /` },
			{
				client,
				settings: enabledSettings(),
				toolTier: "exec",
			},
		);
		expect(result.requireApproval).toBe(true);
		expect(result.reason).toBe("fallback_incomplete_gating_evidence");
		expect(decide).not.toHaveBeenCalled();
	});

	it("classifies nested permission changes and array entries beyond the old ten-item truncation", async () => {
		const decide = vi.fn(async (state: Record<string, unknown>) => {
			const text = JSON.stringify(state);
			return {
				success: true,
				fallback: false,
				latencyMs: 1,
				data: {
					irreversibility: { type: "noul", noul: text.includes("public-write") ? 0.9 : 0.1, confidence: 0.95 },
				},
			};
		});
		const result = await checkToolCallGating(
			"permissions",
			{
				changes: [
					...Array.from({ length: 10 }, () => ({ access: "read" })),
					{ nested: { access: "public-write" } },
				],
			},
			{ client: { decide } as unknown as LayaClient, settings: enabledSettings(), toolTier: "write" },
		);
		expect(result.requireApproval).toBe(true);
		expect(result.reason).toBe("laya_classified_irreversible");
	});

	it("settles cyclic and excessively wide evidence with approval rather than serialization errors or inference", async () => {
		const cyclic: Record<string, unknown> = {};
		cyclic.self = cyclic;
		for (const args of [cyclic, { targets: Array.from({ length: 300 }, () => "target") }]) {
			const { client, decide } = scoredClient();
			const result = await checkToolCallGating("write", args, { client, settings: enabledSettings() });
			expect(result.requireApproval).toBe(true);
			expect(result.fallback).toBe(true);
			expect(decide).not.toHaveBeenCalled();
		}
	});
});
