import { afterEach, beforeEach, describe, expect, it } from "bun:test";
import * as fs from "node:fs/promises";
import * as os from "node:os";
import * as path from "node:path";
import {
	loadAuditRecords,
	recordAuditLog,
	recordSubagentOutcome,
	type SubagentSelectionAuditRecord,
} from "../src/core/harvest/laya-subagent-selection";
import {
	deriveOptimalThreshold,
	filterDisagreements,
	getShadowTraceStats,
	loadReviewLabels,
	saveReviewLabel,
	saveReviewLabels,
	type LayaLabeledTrace,
} from "../src/core/harvest/laya-shadow-review";
import { runLayaCommand } from "../src/cli/laya-cli";
import { loadCalibration, saveCalibration, type CalibrationRecord } from "../src/core/harvest/laya-calibration";

describe("Laya Shadow-Mode Review & Recalibration", () => {
	let tmpDir: string;

	beforeEach(async () => {
		process.exitCode = 0;
		tmpDir = await fs.mkdtemp(path.join(os.tmpdir(), "harvest-laya-review-test-"));
	});

	afterEach(async () => {
		process.exitCode = 0;
		try {
			await fs.rm(tmpDir, { recursive: true, force: true });
		} catch {}
	});

	describe("Audit & Outcome Persistence", () => {
		it("records decision and attaches downstream outcome upon completion", async () => {
			const traceId = "test_trace_001";
			const decisionRecord: SubagentSelectionAuditRecord = {
				id: traceId,
				timestamp: Date.now() - 5000,
				sessionId: "sess_1",
				assignment: "Find all usages of grep in the codebase",
				availableAgents: ["scout", "reviewer", "task"],
				layaPick: "scout",
				confidence: 0.025,
				selectedAgent: "task",
				decisionType: "shadow",
				latencyMs: 120,
			};

			// Record initial decision to disk
			recordAuditLog(decisionRecord, tmpDir);

			// Small sleep to ensure fs append completes
			await Bun.sleep(20);

			// Now record downstream outcome
			await recordSubagentOutcome(
				traceId,
				{
					completedCleanly: true,
					exitCode: 0,
					durationMs: 450,
					timestamp: Date.now(),
				},
				tmpDir,
			);

			// Load audit records from disk
			const loaded = await loadAuditRecords(tmpDir);
			expect(loaded.length).toBe(1);
			expect(loaded[0]?.id).toBe(traceId);
			expect(loaded[0]?.assignment).toBe("Find all usages of grep in the codebase");
			expect(loaded[0]?.layaPick).toBe("scout");
			expect(loaded[0]?.selectedAgent).toBe("task");
			expect(loaded[0]?.outcome).toBeDefined();
			expect(loaded[0]?.outcome?.completedCleanly).toBe(true);
			expect(loaded[0]?.outcome?.exitCode).toBe(0);
		});

		it("handles multiple decisions and failed execution outcomes correctly", async () => {
			const trace1 = "trace_success";
			const trace2 = "trace_failure";

			recordAuditLog(
				{
					id: trace1,
					timestamp: Date.now() - 10000,
					sessionId: "sess_1",
					assignment: "Search for files",
					availableAgents: ["scout", "task"],
					layaPick: "scout",
					confidence: 0.03,
					selectedAgent: "task",
					decisionType: "shadow",
					latencyMs: 100,
				},
				tmpDir,
			);

			recordAuditLog(
				{
					id: trace2,
					timestamp: Date.now() - 5000,
					sessionId: "sess_1",
					assignment: "Run complex refactor",
					availableAgents: ["scout", "task"],
					layaPick: "task",
					confidence: 0.04,
					selectedAgent: "task",
					decisionType: "shadow",
					latencyMs: 100,
				},
				tmpDir,
			);

			await Bun.sleep(20);

			await recordSubagentOutcome(
				trace1,
				{
					completedCleanly: true,
					exitCode: 0,
					durationMs: 300,
					timestamp: Date.now(),
				},
				tmpDir,
			);

			await recordSubagentOutcome(
				trace2,
				{
					completedCleanly: false,
					exitCode: 1,
					error: "Command execution failed: syntax error",
					durationMs: 800,
					timestamp: Date.now(),
				},
				tmpDir,
			);

			const loaded = await loadAuditRecords(tmpDir);
			expect(loaded.length).toBe(2);

			const rec1 = loaded.find(r => r.id === trace1);
			expect(rec1?.outcome?.completedCleanly).toBe(true);

			const rec2 = loaded.find(r => r.id === trace2);
			expect(rec2?.outcome?.completedCleanly).toBe(false);
			expect(rec2?.outcome?.error).toContain("syntax error");
		});
	});

	describe("Disagreement Filtering and Statistics", () => {
		it("calculates accurate shadow trace stats and isolates disagreements", async () => {
			const now = Date.now();
			const dayMs = 24 * 60 * 60 * 1000;

			const records: SubagentSelectionAuditRecord[] = [
				// Disagreement, recent (1 day ago)
				{
					id: "d1",
					timestamp: now - dayMs,
					sessionId: "s1",
					assignment: "Review diff",
					availableAgents: ["scout", "reviewer", "task"],
					layaPick: "reviewer",
					confidence: 0.02,
					selectedAgent: "task",
					decisionType: "shadow",
					latencyMs: 100,
				},
				// Agreement, recent (2 days ago)
				{
					id: "a1",
					timestamp: now - 2 * dayMs,
					sessionId: "s1",
					assignment: "Write code",
					availableAgents: ["scout", "reviewer", "task"],
					layaPick: "task",
					confidence: 0.05,
					selectedAgent: "task",
					decisionType: "shadow",
					latencyMs: 100,
				},
				// Disagreement, older than 30 days (40 days ago)
				{
					id: "d2_old",
					timestamp: now - 40 * dayMs,
					sessionId: "s1",
					assignment: "Find symbol",
					availableAgents: ["scout", "reviewer", "task"],
					layaPick: "scout",
					confidence: 0.015,
					selectedAgent: "task",
					decisionType: "shadow",
					latencyMs: 100,
				},
			];

			const labels = new Map<string, LayaLabeledTrace>();
			// Label d1
			labels.set("d1", {
				traceId: "d1",
				timestamp: now - dayMs,
				assignment: "Review diff",
				layaPick: "reviewer",
				confidence: 0.02,
				callerPick: "task",
				label: "laya_correct",
				reviewedAt: now,
			});

			const stats = getShadowTraceStats(records, labels, 30);
			expect(stats.totalShadowTraces).toBe(3);
			expect(stats.recentShadowTraces).toBe(2);
			expect(stats.totalDisagreements).toBe(2); // d1 and d2_old
			expect(stats.recentDisagreements).toBe(1); // d1
			expect(stats.labeledCount).toBe(1);
			expect(stats.pendingDisagreements).toBe(1); // d2_old pending

			const pending = filterDisagreements(records, labels, { unlabeledOnly: true });
			expect(pending.length).toBe(1);
			expect(pending[0]?.id).toBe("d2_old");
		});
	});

	describe("Review Label Persistence", () => {
		it("persists review labels to disk and restores them without duplication", async () => {
			const label1: LayaLabeledTrace = {
				traceId: "trace_A",
				timestamp: Date.now() - 10000,
				assignment: "Check for security holes in file loader",
				layaPick: "security-reviewer",
				confidence: 0.028,
				callerPick: "task",
				callerOutcomeClean: true,
				label: "laya_correct",
				reviewedAt: Date.now(),
			};

			const label2: LayaLabeledTrace = {
				traceId: "trace_B",
				timestamp: Date.now() - 5000,
				assignment: "Fix typo in comment",
				layaPick: "reviewer",
				confidence: 0.008,
				callerPick: "task",
				callerOutcomeClean: true,
				label: "caller_correct",
				reviewedAt: Date.now(),
			};

			await saveReviewLabels([label1, label2], tmpDir);

			const loaded = await loadReviewLabels(tmpDir);
			expect(loaded.size).toBe(2);
			expect(loaded.get("trace_A")?.label).toBe("laya_correct");
			expect(loaded.get("trace_B")?.label).toBe("caller_correct");

			// Update label1 to ambiguous
			const updatedLabel1: LayaLabeledTrace = {
				...label1,
				label: "ambiguous",
				reviewedAt: Date.now(),
			};
			await saveReviewLabel(updatedLabel1, tmpDir);

			const reloaded = await loadReviewLabels(tmpDir);
			expect(reloaded.size).toBe(2);
			expect(reloaded.get("trace_A")?.label).toBe("ambiguous");
		});
	});

	describe("Recalibration Floor Gating & Threshold Derivation", () => {
		it("strictly refuses recalibration when fewer than 30 decisive labels exist", () => {
			const smallDataset: LayaLabeledTrace[] = [];
			for (let i = 0; i < 29; i++) {
				smallDataset.push({
					traceId: `trace_${i}`,
					timestamp: Date.now() - i * 1000,
					assignment: `Task assignment ${i}`,
					layaPick: i % 2 === 0 ? "scout" : "reviewer",
					confidence: 0.01 + (i % 5) * 0.005,
					callerPick: "task",
					label: i % 3 === 0 ? "caller_correct" : "laya_correct",
					reviewedAt: Date.now(),
				});
			}

			// 29 labels < 30 floor
			expect(() => deriveOptimalThreshold(smallDataset)).toThrow(
				/Insufficient labeled disagreement data: found 29 decisive labels, but recalibration requires a minimum floor of 30/,
			);
		});

		it("excludes ambiguous labels from the 30-sample floor requirement", () => {
			const datasetWithAmbiguous: LayaLabeledTrace[] = [];
			// 25 decisive + 10 ambiguous = 35 total labels, but only 25 decisive
			for (let i = 0; i < 25; i++) {
				datasetWithAmbiguous.push({
					traceId: `decisive_${i}`,
					timestamp: Date.now(),
					assignment: `Assignment ${i}`,
					layaPick: "scout",
					confidence: 0.02,
					callerPick: "task",
					label: "laya_correct",
					reviewedAt: Date.now(),
				});
			}
			for (let i = 0; i < 10; i++) {
				datasetWithAmbiguous.push({
					traceId: `ambiguous_${i}`,
					timestamp: Date.now(),
					assignment: `Ambiguous ${i}`,
					layaPick: "scout",
					confidence: 0.02,
					callerPick: "task",
					label: "ambiguous",
					reviewedAt: Date.now(),
				});
			}

			expect(() => deriveOptimalThreshold(datasetWithAmbiguous)).toThrow(
				/found 25 decisive labels, but recalibration requires a minimum floor of 30/,
			);
		});

		it("accurately derives optimal threshold and metrics on a dataset of >= 30 labels", () => {
			const dataset: LayaLabeledTrace[] = [];
			// Synthesize 40 labeled traces:
			// - 10 wrong picks (caller_correct) clustered with low confidence (0.002 to 0.007)
			// - 30 correct picks (laya_correct) clustered with higher confidence (0.010 to 0.040)
			for (let i = 0; i < 10; i++) {
				dataset.push({
					traceId: `wrong_${i}`,
					timestamp: Date.now(),
					assignment: `Wrong task ${i}`,
					layaPick: "reviewer",
					confidence: 0.002 + i * 0.0005, // 0.002 to 0.0065
					callerPick: "task",
					label: "caller_correct", // Laya was wrong
					reviewedAt: Date.now(),
				});
			}

			for (let i = 0; i < 30; i++) {
				dataset.push({
					traceId: `correct_${i}`,
					timestamp: Date.now(),
					assignment: `Correct task ${i}`,
					layaPick: "scout",
					confidence: 0.01 + i * 0.001, // 0.010 to 0.039
					callerPick: "task",
					label: "laya_correct", // Laya was right
					reviewedAt: Date.now(),
				});
			}

			const result = deriveOptimalThreshold(dataset);

			// At threshold >= 0.010, wrong auto-picks should be 0 (100% precision)
			expect(result.recommendedThreshold).toBeGreaterThanOrEqual(0.008);
			expect(result.selectedPoint.wrongAutoPicks).toBe(0);
			expect(result.selectedPoint.precision).toBe(1.0);
			expect(result.selectedPoint.autoPicks).toBe(30);
			expect(result.selectedPoint.yield).toBe(30 / 40); // 75% yield
		});
	});

	describe("CLI Commands Integration (review-shadow and recalibrate-subagent)", () => {
		it("recalibrate-subagent exits with code 1 and warns when insufficient labels exist", async () => {
			process.exitCode = 0;
			await runLayaCommand({
				action: "recalibrate-subagent",
				flags: { agentDir: tmpDir },
			});

			expect(process.exitCode).toBe(1);
			process.exitCode = 0;
		});

		it("recalibrate-subagent respects dry-run and only updates calibration when --yes is provided", async () => {
			// Populate tmpDir with a baseline calibration file
			const initialCal: CalibrationRecord = {
				version: 1,
				timestamp: Date.now(),
				hardware: {
					tier: "cpu",
					device_name: "Test CPU",
					signature: "test-cpu-sig",
					reason: "Test",
				},
				benchmarks: {
					ultraShort: { minMs: 10, medianMs: 15, runsMs: [10, 15, 20] },
					singleChoice: { minMs: 50, medianMs: 60, runsMs: [50, 60, 70] },
					singleScore: { minMs: 30, medianMs: 35, runsMs: [30, 35, 40] },
					batchedScore: { minMs: 80, medianMs: 90, runsMs: [80, 90, 100] },
				},
				derivedSettings: {
					subagentSelectionRecommendEnabled: true,
					subagentSelectionTimeoutMs: 300,
					subagentSelectionConfidenceThreshold: 0.05, // initial threshold
					subagentSelectionReason: "Test",
					pruningRecommendEnabled: true,
					estimatedAddedLatencyPerTurnMs: 100,
					maxAcceptableLatencyPerTurnMs: 1500,
					pruningReason: "Test",
				},
			};
			await saveCalibration(initialCal, tmpDir);

			// Populate 35 labeled traces
			const labels: LayaLabeledTrace[] = [];
			for (let i = 0; i < 35; i++) {
				labels.push({
					traceId: `cli_trace_${i}`,
					timestamp: Date.now(),
					assignment: `CLI trace assignment ${i}`,
					layaPick: "scout",
					confidence: 0.015 + i * 0.001,
					callerPick: "task",
					label: "laya_correct",
					reviewedAt: Date.now(),
				});
			}
			await saveReviewLabels(labels, tmpDir);

			// 1. Dry run (without --yes)
			process.exitCode = 0;
			await runLayaCommand({
				action: "recalibrate-subagent",
				flags: { agentDir: tmpDir, yes: false },
			});
			expect(process.exitCode).toBe(0);

			// Verify calibration file is untouched
			let cal = await loadCalibration(tmpDir);
			expect(cal?.derivedSettings.subagentSelectionConfidenceThreshold).toBe(0.05);

			// 2. Confirmed run (with --yes)
			await runLayaCommand({
				action: "recalibrate-subagent",
				flags: { agentDir: tmpDir, yes: true },
			});

			cal = await loadCalibration(tmpDir);
			// Derived optimal threshold should now be written
			expect(cal?.derivedSettings.subagentSelectionConfidenceThreshold).not.toBe(0.05);
			expect(cal?.derivedSettings.subagentSelectionConfidenceThreshold).toBeLessThanOrEqual(0.015);
		});

		it("review-shadow supports programmatic labeling via --label flag", async () => {
			const traceId = "flag_trace_123";
			recordAuditLog(
				{
					id: traceId,
					timestamp: Date.now(),
					sessionId: "s1",
					assignment: "Verify token counts",
					availableAgents: ["scout", "task"],
					layaPick: "scout",
					confidence: 0.018,
					selectedAgent: "task",
					decisionType: "shadow",
					latencyMs: 50,
				},
				tmpDir,
			);
			await Bun.sleep(20);

			// Programmatic label via CLI flag
			await runLayaCommand({
				action: "review-shadow",
				flags: {
					agentDir: tmpDir,
					label: `${traceId}:laya_correct`,
				},
			});

			const labels = await loadReviewLabels(tmpDir);
			expect(labels.has(traceId)).toBe(true);
			expect(labels.get(traceId)?.label).toBe("laya_correct");
		});
	});
});
