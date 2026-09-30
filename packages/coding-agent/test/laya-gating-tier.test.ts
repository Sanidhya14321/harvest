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
});
