import { afterEach, describe, expect, it, vi } from "bun:test";
import {
	buildSubagentCriteria,
	classifySubagentShadow,
	isLayaSubagentAutoPickEnabled,
	isLayaSubagentShadowEnabled,
	selectSubagentWithLaya,
	SUBAGENT_SELECTION_AUDIT_LOG,
} from "../src/core/harvest/laya-subagent-selection";
import type { LayaClient } from "../src/core/harvest/laya-client";
import type { AgentDefinition } from "../src/task/types";
import { Settings } from "../src/config/settings";
import { TempDir } from "@harvest/pi-utils";

const TEST_AGENTS: AgentDefinition[] = [
	{
		name: "scout",
		description: "Fast exploration agent",
		systemPrompt: "You are scout",
		source: "bundled",
	},
	{
		name: "reviewer",
		description: "Code review specialist",
		systemPrompt: "You are reviewer",
		source: "bundled",
	},
	{
		name: "security-reviewer",
		description: "Security audit specialist",
		systemPrompt: "You are security reviewer",
		source: "bundled",
	},
	{
		name: "sonic",
		description: "Mechanical execution agent",
		systemPrompt: "You are sonic",
		source: "bundled",
	},
	{
		name: "task",
		description: "General-purpose agent",
		systemPrompt: "You are task",
		source: "bundled",
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
					description: "",
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
			const mockClient = {
				decide: vi.fn(async () => ({
					success: true,
					data: {
						subagent_choice: {
							answer: "scout",
							confidence: 0.92,
						},
					},
					fallback: false,
					latencyMs: 15,
				})),
				isHealthy: vi.fn(async () => true),
			} as unknown as LayaClient;

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
			const mockClient = {
				decide: vi.fn(async () => ({
					success: true,
					data: {
						subagent_choice: {
							answer: "reviewer",
							confidence: 0.65, // Below 0.80
						},
					},
					fallback: false,
					latencyMs: 12,
				})),
				isHealthy: vi.fn(async () => true),
			} as unknown as LayaClient;

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
			const mockClient = {
				decide: vi.fn(async () => ({
					success: true,
					data: {
						subagent_choice: {
							answer: "scout",
							confidence: 0.08,
						},
					},
					fallback: false,
					latencyMs: 15,
				})),
				isHealthy: vi.fn(async () => true),
			} as unknown as LayaClient;

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
			const mockClient = {
				decide: vi.fn(async () => ({
					success: true,
					data: {
						subagent_choice: {
							answer: "security-reviewer",
							confidence: 0.55, // Low confidence
						},
					},
					fallback: false,
					latencyMs: 14,
				})),
				isHealthy: vi.fn(async () => true),
			} as unknown as LayaClient;

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

	describe("Shadow telemetry without blocking dispatch", () => {
		it("distinguishes auto-pick from shadow settings", () => {
			expect(isLayaSubagentAutoPickEnabled(Settings.isolated({ "laya.subagentSelection": true }))).toBe(true);
			expect(isLayaSubagentAutoPickEnabled(Settings.isolated({ "laya.subagentSelection": false }))).toBe(false);
			expect(
				isLayaSubagentShadowEnabled(
					Settings.isolated({ "laya.subagentSelection": false, "laya.subagentSelectionShadow": true }),
				),
			).toBe(true);
			expect(
				isLayaSubagentShadowEnabled(
					Settings.isolated({ "laya.subagentSelection": false, "laya.subagentSelectionShadow": false }),
				),
			).toBe(false);
		});

		it("scores the assignment in the background without awaiting dispatch", async () => {
			const tempDir = TempDir.createSync("@pi-shadow-selection-");
			try {
				const release = Promise.withResolvers<void>();
				const mockClient = {
					decide: vi.fn(async () => {
						await release.promise;
						return {
							success: true,
							fallback: false,
							latencyMs: 1,
							data: { subagent_choice: { answer: "scout", confidence: 0.9 } },
						};
					}),
				} as unknown as LayaClient;

				const auditBefore = SUBAGENT_SELECTION_AUDIT_LOG.length;
				// Returns void immediately while the decision still pends:
				// dispatch never waits for shadow telemetry.
				const returned = classifySubagentShadow("Explore codebase structure", {
					client: mockClient,
					settings: Settings.isolated({ "laya.enabled": true, "laya.subagentSelection": false }),
					availableAgents: TEST_AGENTS,
					defaultAgent: "task",
					agentDir: tempDir.path(),
				});
				expect(returned).toBeUndefined();
				expect(mockClient.decide).toHaveBeenCalledTimes(1);
				expect(SUBAGENT_SELECTION_AUDIT_LOG).toHaveLength(auditBefore);

				release.resolve();
				const deadline = Date.now() + 5000;
				let recentRecord;
				for (;;) {
					recentRecord = SUBAGENT_SELECTION_AUDIT_LOG[SUBAGENT_SELECTION_AUDIT_LOG.length - 1];
					if (recentRecord && SUBAGENT_SELECTION_AUDIT_LOG.length > auditBefore) break;
					if (Date.now() > deadline) throw new Error("background shadow classification never settled");
					await Bun.sleep(5);
				}
				expect(recentRecord?.decisionType).toBe("shadow");
				expect(recentRecord?.layaPick).toBe("scout");
				expect(recentRecord?.selectedAgent).toBe("task");
				// The caller pick is a disagreement baseline, never a correctness label.
				expect(recentRecord?.callerBaseline).toBe("task");
				expect(recentRecord?.groundTruth).toBeUndefined();
			} finally {
				tempDir.removeSync();
			}
		});

		it("drops shadow classifications beyond the in-flight bound instead of queueing", async () => {
			const tempDir = TempDir.createSync("@pi-shadow-bound-");
			try {
				const release = Promise.withResolvers<void>();
				const mockClient = {
					decide: vi.fn(async () => {
						await release.promise;
						return { success: false, fallback: true, latencyMs: 1 };
					}),
				} as unknown as LayaClient;
				const options = {
					client: mockClient,
					settings: Settings.isolated({ "laya.enabled": true, "laya.subagentSelection": false }),
					availableAgents: TEST_AGENTS,
					defaultAgent: "task",
					agentDir: tempDir.path(),
				};

				classifySubagentShadow("first", options);
				classifySubagentShadow("second", options);
				classifySubagentShadow("third-over-bound", options);
				expect(mockClient.decide).toHaveBeenCalledTimes(2);

				const auditBefore = SUBAGENT_SELECTION_AUDIT_LOG.length;
				release.resolve();
				const deadline = Date.now() + 5000;
				while (SUBAGENT_SELECTION_AUDIT_LOG.length < auditBefore + 2) {
					if (Date.now() > deadline) throw new Error("background shadows never settled");
					await Bun.sleep(5);
				}
				// Let the in-flight counter teardown flush before reusing the bound.
				await Bun.sleep(0);
				// Draining the in-flight pair frees the bound for new work.
				classifySubagentShadow("fourth-after-drain", options);
				expect(mockClient.decide).toHaveBeenCalledTimes(3);
			} finally {
				tempDir.removeSync();
			}
		});
	});

	describe("Step 3: Fail-Open Fallback", () => {
		it("fails OPEN to default agent when sidecar is unreachable or errors", async () => {
			const mockClient = {
				decide: vi.fn(async () => ({
					success: false,
					fallback: true,
					fallbackReason: "fetch_failed: connection refused",
					latencyMs: 5,
				})),
				isHealthy: vi.fn(async () => false),
			} as unknown as LayaClient;

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
			const mockClient = {
				decide: vi.fn(async () => ({
					success: false,
					fallback: true,
					fallbackReason: "timeout: exceeded 300ms budget",
					latencyMs: 301,
				})),
				isHealthy: vi.fn(async () => false),
			} as unknown as LayaClient;

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
			const mockClient = {
				decide: vi.fn(),
				isHealthy: vi.fn(async () => true),
			} as unknown as LayaClient;

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

			const mockClient = {
				decide: vi.fn(),
				isHealthy: vi.fn(async () => true),
			} as unknown as LayaClient;

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
