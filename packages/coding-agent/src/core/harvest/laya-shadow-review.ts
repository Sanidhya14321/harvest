/**
 * Laya Shadow-Mode Trace Review, Disagreement Sampling, and Threshold Re-Derivation.
 *
 * Implements tooling to:
 * 1. Filter shadow-mode traces for disagreements (`layaPick !== selectedAgent`).
 * 2. Surface caller outcome signals (completed cleanly vs failed/aborted) as context.
 * 3. Store human review labels ("laya_correct", "caller_correct", "ambiguous") to ~/.harvest/laya-subagent-labels.json.
 * 4. Sweep thresholds against empirical labeled disagreement traces.
 * 5. Refuse recalibration if fewer than MINIMUM_LABELED_DISAGREEMENTS_FLOOR (30) exist.
 */

import * as fs from "node:fs/promises";
import * as path from "node:path";
import { getAgentDir, isEnoent, logger } from "@harvest/pi-utils";
import type { SubagentSelectionAuditRecord } from "./laya-subagent-selection";

export const MINIMUM_LABELED_DISAGREEMENTS_FLOOR = 30;

export type HumanReviewLabel = "laya_correct" | "caller_correct" | "ambiguous";

export interface LayaLabeledTrace {
	readonly traceId: string;
	readonly timestamp: number;
	readonly assignment: string;
	readonly layaPick: string;
	readonly confidence: number;
	readonly callerPick: string;
	readonly callerOutcomeClean?: boolean;
	readonly callerOutcomeError?: string;
	readonly label: HumanReviewLabel;
	readonly reviewedAt: number;
	readonly notes?: string;
}

export interface LayaReviewStore {
	readonly version: number;
	readonly updatedAt: number;
	readonly labels: Record<string, LayaLabeledTrace>;
}

export function getReviewLabelsPath(agentDir?: string): string {
	const dir = agentDir ?? getAgentDir();
	return path.join(dir, "laya-subagent-labels.json");
}

/**
 * Load persisted human review labels from disk.
 */
export async function loadReviewLabels(agentDir?: string): Promise<Map<string, LayaLabeledTrace>> {
	const filePath = getReviewLabelsPath(agentDir);
	try {
		const text = await fs.readFile(filePath, "utf-8");
		const store = JSON.parse(text) as LayaReviewStore;
		const map = new Map<string, LayaLabeledTrace>();
		if (store && store.labels && typeof store.labels === "object") {
			for (const [id, labelObj] of Object.entries(store.labels)) {
				map.set(id, labelObj);
			}
		}
		return map;
	} catch (err) {
		if (isEnoent(err)) return new Map();
		logger.warn("Failed to load Laya review labels from disk", { filePath, error: String(err) });
		return new Map();
	}
}

/**
 * Save or update a single human review label.
 */
export async function saveReviewLabel(label: LayaLabeledTrace, agentDir?: string): Promise<void> {
	await saveReviewLabels([label], agentDir);
}

/**
 * Save or update multiple human review labels atomically.
 */
export async function saveReviewLabels(labels: readonly LayaLabeledTrace[], agentDir?: string): Promise<void> {
	const filePath = getReviewLabelsPath(agentDir);
	const existing = await loadReviewLabels(agentDir);

	for (const label of labels) {
		existing.set(label.traceId, label);
	}

	const store: LayaReviewStore = {
		version: 1,
		updatedAt: Date.now(),
		labels: Object.fromEntries(existing.entries()),
	};

	try {
		await fs.mkdir(path.dirname(filePath), { recursive: true });
		await fs.writeFile(filePath, JSON.stringify(store, null, 2), "utf-8");
	} catch (err) {
		logger.error("Failed to save Laya review labels", { filePath, error: String(err) });
		throw err;
	}
}

export interface ShadowTraceStats {
	readonly totalShadowTraces: number;
	readonly recentShadowTraces: number; // <= maxAgeDays
	readonly totalDisagreements: number;
	readonly recentDisagreements: number;
	readonly labeledCount: number;
	readonly pendingDisagreements: number;
}

/**
 * Calculate summary statistics for shadow-mode traces and disagreements.
 */
export function getShadowTraceStats(
	records: readonly SubagentSelectionAuditRecord[],
	labels: Map<string, LayaLabeledTrace>,
	maxAgeDays = 30,
): ShadowTraceStats {
	const now = Date.now();
	const maxAgeMs = maxAgeDays * 24 * 60 * 60 * 1000;

	let totalShadowTraces = 0;
	let recentShadowTraces = 0;
	let totalDisagreements = 0;
	let recentDisagreements = 0;
	let labeledCount = 0;

	for (const record of records) {
		if (record.decisionType !== "shadow") continue;
		totalShadowTraces++;

		const isRecent = now - record.timestamp <= maxAgeMs;
		if (isRecent) recentShadowTraces++;

		const hasDisagreement = Boolean(record.layaPick && record.layaPick !== record.selectedAgent);
		if (hasDisagreement) {
			totalDisagreements++;
			if (isRecent) recentDisagreements++;
			if (labels.has(record.id)) {
				labeledCount++;
			}
		}
	}

	return {
		totalShadowTraces,
		recentShadowTraces,
		totalDisagreements,
		recentDisagreements,
		labeledCount,
		pendingDisagreements: Math.max(0, totalDisagreements - labeledCount),
	};
}

export interface FilterDisagreementOptions {
	readonly maxAgeDays?: number;
	readonly unlabeledOnly?: boolean;
	readonly limit?: number;
}

/**
 * Extract disagreement traces where Laya's pick differed from the caller's actual pick.
 */
export function filterDisagreements(
	records: readonly SubagentSelectionAuditRecord[],
	labels: Map<string, LayaLabeledTrace>,
	options: FilterDisagreementOptions = {},
): SubagentSelectionAuditRecord[] {
	const now = Date.now();
	const maxAgeMs = options.maxAgeDays ? options.maxAgeDays * 24 * 60 * 60 * 1000 : Infinity;

	const disagreements: SubagentSelectionAuditRecord[] = [];

	for (const record of records) {
		if (record.decisionType !== "shadow") continue;
		if (!record.layaPick || record.layaPick === record.selectedAgent) continue;
		if (now - record.timestamp > maxAgeMs) continue;
		if (options.unlabeledOnly && labels.has(record.id)) continue;

		disagreements.push(record);
		if (options.limit && disagreements.length >= options.limit) break;
	}

	return disagreements;
}

export interface ThresholdSweepPoint {
	readonly threshold: number;
	readonly totalEvaluated: number;
	readonly autoPicks: number;
	readonly correctAutoPicks: number;
	readonly wrongAutoPicks: number;
	readonly escalations: number;
	readonly correctEscalations: number;
	readonly wrongEscalations: number;
	readonly errorRate: number; // wrongAutoPicks / autoPicks
	readonly precision: number; // correctAutoPicks / autoPicks
	readonly yield: number; // autoPicks / totalEvaluated
}

export const DEFAULT_SWEEP_THRESHOLDS = [
	0.001, 0.002, 0.005, 0.008, 0.01, 0.012, 0.015, 0.02, 0.025, 0.03, 0.04, 0.05, 0.075, 0.1, 0.15, 0.2, 0.3,
	0.5, 0.8,
];

/**
 * Sweep candidate confidence thresholds across labeled disagreement traces.
 * Note: "ambiguous" labels are excluded from accuracy computation.
 */
export function sweepThresholds(
	labels: readonly LayaLabeledTrace[],
	candidateThresholds: readonly number[] = DEFAULT_SWEEP_THRESHOLDS,
): ThresholdSweepPoint[] {
	// Exclude ambiguous traces from accuracy/precision derivation
	const decisiveLabels = labels.filter(l => l.label === "laya_correct" || l.label === "caller_correct");
	const totalEvaluated = decisiveLabels.length;

	if (totalEvaluated === 0) return [];

	const sortedCandidates = [...candidateThresholds].sort((a, b) => a - b);

	return sortedCandidates.map(threshold => {
		let autoPicks = 0;
		let correctAutoPicks = 0;
		let wrongAutoPicks = 0;
		let escalations = 0;
		let correctEscalations = 0;
		let wrongEscalations = 0;

		for (const item of decisiveLabels) {
			if (item.confidence >= threshold) {
				autoPicks++;
				if (item.label === "laya_correct") {
					correctAutoPicks++;
				} else {
					wrongAutoPicks++;
				}
			} else {
				escalations++;
				if (item.label === "caller_correct") {
					correctEscalations++;
				} else {
					wrongEscalations++;
				}
			}
		}

		const errorRate = autoPicks > 0 ? wrongAutoPicks / autoPicks : 0.0;
		const precision = autoPicks > 0 ? correctAutoPicks / autoPicks : 1.0;
		const yieldRate = totalEvaluated > 0 ? autoPicks / totalEvaluated : 0.0;

		return {
			threshold,
			totalEvaluated,
			autoPicks,
			correctAutoPicks,
			wrongAutoPicks,
			escalations,
			correctEscalations,
			wrongEscalations,
			errorRate,
			precision,
			yield: yieldRate,
		};
	});
}

export interface DerivationResult {
	readonly recommendedThreshold: number;
	readonly points: ThresholdSweepPoint[];
	readonly selectedPoint: ThresholdSweepPoint;
	readonly sampleCount: number;
}

/**
 * Re-derive optimal confidence threshold from empirical human review labels.
 *
 * Enforces MINIMUM_LABELED_DISAGREEMENTS_FLOOR (30 decisive labels).
 * Throws if the floor is not met.
 */
export function deriveOptimalThreshold(
	labels: readonly LayaLabeledTrace[],
	options: {
		candidateThresholds?: readonly number[];
		maxAcceptableErrorRate?: number;
		minSampleFloor?: number;
	} = {},
): DerivationResult {
	const floor = options.minSampleFloor ?? MINIMUM_LABELED_DISAGREEMENTS_FLOOR;
	const decisiveLabels = labels.filter(l => l.label === "laya_correct" || l.label === "caller_correct");

	if (decisiveLabels.length < floor) {
		throw new Error(
			`Insufficient labeled disagreement data: found ${decisiveLabels.length} decisive labels, ` +
				`but recalibration requires a minimum floor of ${floor} labeled samples.`,
		);
	}

	const candidates = options.candidateThresholds ?? DEFAULT_SWEEP_THRESHOLDS;
	const points = sweepThresholds(decisiveLabels, candidates);

	if (points.length === 0) {
		throw new Error("No threshold points evaluated.");
	}

	const maxError = options.maxAcceptableErrorRate ?? 0.0; // Target 0% wrong auto-picks

	// Filter points meeting the error rate constraint (default 0 wrong auto-picks)
	const zeroErrorPoints = points.filter(p => p.errorRate <= maxError && p.autoPicks > 0);

	let bestPoint: ThresholdSweepPoint;

	if (zeroErrorPoints.length > 0) {
		// Pick point with highest yield among zero-error points; break ties with lower threshold
		bestPoint = zeroErrorPoints.reduce((best, cur) => {
			if (cur.yield > best.yield) return cur;
			if (cur.yield === best.yield && cur.threshold < best.threshold) return cur;
			return best;
		}, zeroErrorPoints[0]!);
	} else {
		// If zero-error is not achievable at any threshold, pick point with lowest error rate and highest precision
		bestPoint = points.reduce((best, cur) => {
			if (cur.errorRate < best.errorRate) return cur;
			if (cur.errorRate === best.errorRate && cur.precision > best.precision) return cur;
			if (cur.errorRate === best.errorRate && cur.yield > best.yield) return cur;
			return best;
		}, points[0]!);
	}

	return {
		recommendedThreshold: bestPoint.threshold,
		points,
		selectedPoint: bestPoint,
		sampleCount: decisiveLabels.length,
	};
}
