import { afterEach, describe, expect, it, vi } from "bun:test";
import {
	buildSubagentCriteria,
	selectSubagentWithLaya,
	SUBAGENT_SELECTION_AUDIT_LOG,
} from "../src/core/harvest/laya-subagent-selection";
import type { LayaClient } from "../src/core/harvest/laya-client";
import type { AgentDefinition } from "../src/task/types";
import { Settings } from "../src/config/settings";

const TEST_AGENTS: AgentDefinition[] = [
	{
		name: "scout",
		description: "Fast exploration agent",
		systemPrompt: "You are scout",
		source: "bundle",
	},
	{
		name: "reviewer",
		description: "Code review specialist",
		systemPrompt: "You are reviewer",
		source: "bundle",
	},
	{
		name: "security-reviewer",
		description: "Security audit specialist",
		systemPrompt: "You are security reviewer",
		source: "bundle",
	},
	{
		name: "sonic",
		description: "Mechanical execution agent",
		systemPrompt: "You are sonic",
		source: "bundle",
	},
	{
		name: "task",
		description: "General-purpose agent",
		systemPrompt: "You are task",
		source: "bundle",
	},
];

afterEach(() => {
	vi.restoreAllMocks();
});

describe("Laya Subagent Selection (Phase 2)", () => {
	describe("Step 0 & 1: Subagent Roster and Mutually Distinct Criteria", () => {
		it("enumerates mutually distinct criteria for all 5 confirmed Harvest subagents", () => {
			const criteria = buildSubagentCriteria(TEST_AGENTS);
			const names = Object.keys(criteria);

			expect(names).toEqual(["scout", "reviewer", "security-reviewer", "task", "sonic"]);
			expect(names.length).toBe(5);

			// Criteria must be distinct and non-empty
			const criteriaValues = Object.values(criteria);
			const uniqueValues = new Set(criteriaValues);
			expect(uniqueValues.size).toBe(5);

			// Must accurately reflect the confirmed purposes
			expect(criteria.scout).toContain("Codebase navigation and discovery");
			expect(criteria.reviewer).toContain("Code review and pull request inspection");
			expect(criteria["security-reviewer"]).toContain("Security and vulnerability assessment");
			expect(criteria.sonic).toContain("Mechanical and repetitive tasks");
			expect(criteria.task).toContain("Implementation and active software engineering");
		});

		it("supports custom subagents with custom descriptions or generated fallbacks", () => {
			const customAgents: AgentDefinition[] = [
				{
					name: "database-migrator",
					description: "Executes and verifies SQL schema migrations",
					systemPrompt: "You migrate DB",
					source: "project",
				},
				{
					name: "unannotated-agent",
					systemPrompt: "You are unannotated",
					source: "project",
				},
			];

			const criteria = buildSubagentCriteria(customAgents);
			expect(criteria["database-migrator"]).toBe("Executes and verifies SQL schema migrations");
			expect(criteria["unannotated-agent"]).toContain("Specialized agent 'unannotated-agent'");
		});
	});

	describe("Step 2: Confidence-Gated Auto-Pick vs Escalation", () => {
		it("auto-picks subagent directly when confidence >= threshold (0.80)", async () => {
			const mockClient: LayaClient = {
				decide: vi.fn(async () => ({
					data: {
						subagent_choice: {
							answer: "scout",
							confidence: 0.92,
						},
					},
					fallback: false,
					latencyMs: 15,
				})),
				batchDecide: vi.fn(),
				isHealthy: vi.fn(async () => true),
				getMetrics: vi.fn(),
			};

			const activeSettings = Settings.isolated({
				"laya.enabled": true,
				"laya.subagentSelection": true,
			});

			const decision = await selectSubagentWithLaya("Find where git status is parsed in the codebase", {
				client: mockClient,
				settings: activeSettings,
				availableAgents: TEST_AGENTS,
				defaultAgent: "task",
				confidenceThreshold: 0.8,
			});

			expect(decision.selectedAgent).toBe("scout");
			expect(decision.layaPick).toBe("scout");
			expect(decision.confidence).toBe(0.92);
			expect(decision.decisionType).toBe("auto_pick");
			expect(decision.fallback).toBe(false);

			// Check audit log
			const recentRecord = SUBAGENT_SELECTION_AUDIT_LOG[SUBAGENT_SELECTION_AUDIT_LOG.length - 1];
			expect(recentRecord?.selectedAgent).toBe("scout");
			expect(recentRecord?.decisionType).toBe("auto_pick");
			expect(recentRecord?.confidence).toBe(0.92);
		});

		it("escalates to default/caller agent when confidence is below threshold (< 0.80)", async () => {
			const mockClient: LayaClient = {
				decide: vi.fn(async () => ({
					data: {
						subagent_choice: {
							answer: "reviewer",
							confidence: 0.65, // Below 0.80
						},
					},
					fallback: false,
					latencyMs: 12,
				})),
				batchDecide: vi.fn(),
				isHealthy: vi.fn(async () => true),
				getMetrics: vi.fn(),
			};

			const activeSettings = Settings.isolated({
				"laya.enabled": true,
				"laya.subagentSelection": true,
			});

			const decision = await selectSubagentWithLaya("Maybe check this code or maybe rewrite it", {
				client: mockClient,
				settings: activeSettings,
				availableAgents: TEST_AGENTS,
				defaultAgent: "task",
				confidenceThreshold: 0.8,
			});

			// Escalated to defaultAgent
			expect(decision.selectedAgent).toBe("task");
			expect(decision.layaPick).toBe("reviewer");
			expect(decision.confidence).toBe(0.65);
			expect(decision.decisionType).toBe("escalation");
			expect(decision.fallback).toBe(false);

			// Check audit log logs escalation distinctly
			const recentRecord = SUBAGENT_SELECTION_AUDIT_LOG[SUBAGENT_SELECTION_AUDIT_LOG.length - 1];
			expect(recentRecord?.selectedAgent).toBe("task");
			expect(recentRecord?.layaPick).toBe("reviewer");
			expect(recentRecord?.decisionType).toBe("escalation");
		});

		it("records decision in shadow mode without altering actual subagent dispatch", async () => {
			const mockClient: LayaClient = {
				decide: vi.fn(async () => ({
					data: {
						subagent_choice: {
							answer: "scout",
							confidence: 0.08,
						},
					},
					fallback: false,
					latencyMs: 15,
				})),
				batchDecide: vi.fn(),
				isHealthy: vi.fn(async () => true),
				getMetrics: vi.fn(),
			};

			// Active selection disabled, shadow mode enabled
			const shadowSettings = Settings.isolated({
				"laya.enabled": true,
				"laya.subagentSelection": false,
				"laya.subagentSelectionShadow": true,
			});

			const decision = await selectSubagentWithLaya("Explore codebase structure", {
				client: mockClient,
				settings: shadowSettings,
				availableAgents: TEST_AGENTS,
				defaultAgent: "task",
			});

			expect(decision.selectedAgent).toBe("task"); // Dispatched agent NEVER altered in shadow mode!
			expect(decision.layaPick).toBe("scout");
			expect(decision.decisionType).toBe("shadow");
			expect(decision.fallback).toBe(false);

			const recentRecord = SUBAGENT_SELECTION_AUDIT_LOG[SUBAGENT_SELECTION_AUDIT_LOG.length - 1];
			expect(recentRecord?.selectedAgent).toBe("task");
			expect(recentRecord?.layaPick).toBe("scout");
			expect(recentRecord?.decisionType).toBe("shadow");
		});

		it("allows forced-auto mode to bypass confidence threshold for benchmark comparison", async () => {
			const mockClient: LayaClient = {
				decide: vi.fn(async () => ({
					data: {
						subagent_choice: {
							answer: "security-reviewer",
							confidence: 0.55, // Low confidence
						},
					},
					fallback: false,
					latencyMs: 14,
				})),
				batchDecide: vi.fn(),
				isHealthy: vi.fn(async () => true),
				getMetrics: vi.fn(),
			};

			const decision = await selectSubagentWithLaya("Check for possible path traversal in file loader", {
				client: mockClient,
				availableAgents: TEST_AGENTS,
				defaultAgent: "task",
				confidenceThreshold: 0.8,
				forcedAuto: true,
			});

			// Forced auto adopts pick regardless of low confidence
			expect(decision.selectedAgent).toBe("security-reviewer");
			expect(decision.decisionType).toBe("auto_pick");
		});
	});

	describe("Step 3: Fail-Open Fallback", () => {
		it("fails OPEN to default agent when sidecar is unreachable or errors", async () => {
			const mockClient: LayaClient = {
				decide: vi.fn(async () => ({
					fallback: true,
					fallbackReason: "fetch_failed: connection refused",
					latencyMs: 5,
				})),
				batchDecide: vi.fn(),
				isHealthy: vi.fn(async () => false),
				getMetrics: vi.fn(),
			};

			const decision = await selectSubagentWithLaya("Investigate memory leak", {
				client: mockClient,
				availableAgents: TEST_AGENTS,
				defaultAgent: "task",
			});

			expect(decision.selectedAgent).toBe("task");
			expect(decision.decisionType).toBe("fallback");
			expect(decision.fallback).toBe(true);
			expect(decision.fallbackReason).toBe("fetch_failed: connection refused");

			// Check audit log logs fallback distinctly from escalation
			const recentRecord = SUBAGENT_SELECTION_AUDIT_LOG[SUBAGENT_SELECTION_AUDIT_LOG.length - 1];
			expect(recentRecord?.selectedAgent).toBe("task");
			expect(recentRecord?.decisionType).toBe("fallback");
		});

		it("fails OPEN to default agent when sidecar times out (~300ms budget)", async () => {
			const mockClient: LayaClient = {
				decide: vi.fn(async () => ({
					fallback: true,
					fallbackReason: "timeout: exceeded 300ms budget",
					latencyMs: 301,
				})),
				batchDecide: vi.fn(),
				isHealthy: vi.fn(async () => false),
				getMetrics: vi.fn(),
			};

			const decision = await selectSubagentWithLaya("Audit dependencies", {
				client: mockClient,
				availableAgents: TEST_AGENTS,
				defaultAgent: "task",
				timeoutMs: 300,
			});

			expect(decision.selectedAgent).toBe("task");
			expect(decision.decisionType).toBe("fallback");
			expect(decision.fallback).toBe(true);
			expect(decision.fallbackReason).toContain("timeout");
		});
	});

	describe("Settings and Roster Boundary Bypasses", () => {
		it("immediately returns the only agent without calling sidecar when roster has <= 1 agent", async () => {
			const singleAgent = [TEST_AGENTS[0]!];
			const mockClient: LayaClient = {
				decide: vi.fn(),
				batchDecide: vi.fn(),
				isHealthy: vi.fn(async () => true),
				getMetrics: vi.fn(),
			};

			const decision = await selectSubagentWithLaya("Do something", {
				client: mockClient,
				availableAgents: singleAgent,
			});

			expect(decision.selectedAgent).toBe("scout");
			expect(decision.latencyMs).toBe(0);
			expect(mockClient.decide).not.toHaveBeenCalled();
		});

		it("falls back immediately when laya.subagentSelection and shadow are disabled in settings", async () => {
			const settings = Settings.isolated({
				"laya.enabled": true,
				"laya.subagentSelection": false,
				"laya.subagentSelectionShadow": false,
			});

			const mockClient: LayaClient = {
				decide: vi.fn(),
				batchDecide: vi.fn(),
				isHealthy: vi.fn(async () => true),
				getMetrics: vi.fn(),
			};

			const decision = await selectSubagentWithLaya("Do something", {
				client: mockClient,
				settings,
				availableAgents: TEST_AGENTS,
				defaultAgent: "task",
			});

			expect(decision.selectedAgent).toBe("task");
			expect(decision.decisionType).toBe("fallback");
			expect(decision.fallbackReason).toBe("laya_subagent_selection_disabled_by_settings");
			expect(mockClient.decide).not.toHaveBeenCalled();
		});
	});

	describe("Integration with resolveEffectiveSubagentPolicy", () => {
		it("delegates to Laya when agent is not specified in request", async () => {
			const { resolveEffectiveSubagentPolicy } = await import("../src/task/structured-subagent");
			const layaModule = await import("../src/core/harvest/laya-subagent-selection");

			const spy = vi.spyOn(layaModule, "selectSubagentWithLaya").mockResolvedValue({
				selectedAgent: "scout",
				layaPick: "scout",
				confidence: 0.95,
				threshold: 0.8,
				decisionType: "auto_pick",
				fallback: false,
				latencyMs: 10,
			});

			const isolatedSettings = Settings.isolated({
				"laya.enabled": true,
				"laya.subagentSelection": true,
			});
			vi.spyOn(isolatedSettings, "reloadFromDisk").mockResolvedValue(undefined);

			const testSession = {
				cwd: process.cwd(),
				hasUI: false,
				settings: isolatedSettings,
				getSessionFile: () => null,
				getSessionSpawns: () => "*",
				getPlanModeState: () => undefined,
			};

			const policy = await resolveEffectiveSubagentPolicy({
				session: testSession as any,
				invocationKind: "task",
				assignment: "Find all usages of search_web in the codebase",
			});

			expect(spy).toHaveBeenCalled();
			expect(policy.agentName).toBe("scout");
		});

		it("preserves explicit agent without Laya override when caller specifies an agent", async () => {
			const { resolveEffectiveSubagentPolicy } = await import("../src/task/structured-subagent");
			const layaModule = await import("../src/core/harvest/laya-subagent-selection");

			const spy = vi.spyOn(layaModule, "selectSubagentWithLaya");

			const isolatedSettings = Settings.isolated({
				"laya.enabled": true,
				"laya.subagentSelection": true,
			});
			vi.spyOn(isolatedSettings, "reloadFromDisk").mockResolvedValue(undefined);

			const testSession = {
				cwd: process.cwd(),
				hasUI: false,
				settings: isolatedSettings,
				getSessionFile: () => null,
				getSessionSpawns: () => "*",
				getPlanModeState: () => undefined,
			};

			const policy = await resolveEffectiveSubagentPolicy({
				session: testSession as any,
				invocationKind: "task",
				assignment: "Find all usages of search_web in the codebase",
				agent: "reviewer",
			});

			// Should not invoke Laya selection because caller explicitly chose "reviewer"
			expect(spy).not.toHaveBeenCalled();
			expect(policy.agentName).toBe("reviewer");
		});
	});

	describe("Live Sidecar Integration", () => {
		it("communicates with live Laya daemon if running on port 8177", async () => {
			const { getLayaClient } = await import("../src/core/harvest/laya-client");
			const client = getLayaClient();
			const healthy = await client.isHealthy();
			if (!healthy) {
				console.log("Skipping live daemon test: daemon not reachable on port 8177");
				return;
			}

			const decision = await selectSubagentWithLaya(
				"Review this pull request diff for bugs, regressions, and edge cases before merging.",
				{
					availableAgents: TEST_AGENTS,
					defaultAgent: "task",
					forcedAuto: true,
					timeoutMs: 40000,
				},
			);

			expect(decision.fallback).toBe(false);
			expect(decision.layaPick).toBe("reviewer");
			expect(decision.selectedAgent).toBe("reviewer");
			expect(decision.decisionType).toBe("auto_pick");
		}, 50000);
	});
});
