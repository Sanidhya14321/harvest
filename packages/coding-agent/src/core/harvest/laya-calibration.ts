/**
 * Hardware-Aware Self-Calibration Engine for Laya.
 *
 * Implements one-time local calibration of the Laya decision sidecar on whatever
 * hardware it is running on (CPU, NVIDIA GPU, or Apple Silicon), replacing hardcoded
 * timeouts and feature flags with empirically derived settings.
 */

import * as path from "node:path";
import { getAgentDir, isEnoent, logger } from "@harvest/pi-utils";
import type { LayaClient, LayaQuestionDefinition } from "./laya-client";
import type { SettingPath } from "../../config/settings";
import {
	CALIBRATION_PRUNING_CRITERIA,
	CALIBRATION_SANITY_CHECK_INSTRUCTIONS,
	SUBAGENT_CRITERIA,
	SUBAGENT_SELECTION_INSTRUCTIONS,
	renderCalibrationRelevanceInstructions,
} from "./laya-prompt-assets";

/** Settings surface needed to tell an explicit override from a schema default. */
export interface ExplicitSettingSource {
	get(key: SettingPath): unknown;
	isConfigured?: (key: SettingPath) => boolean;
}

/**
 * Explicitly configured value, or undefined when the key is unset. Plain
 * `get()` cannot make this distinction: schema defaults (e.g.
 * `laya.pruning: true`, `laya.subagentSelectionTimeoutMs: 300`) would
 * otherwise mask hardware-derived calibration. Sources without
 * `isConfigured` (unit-test stubs) fall back to `get()`.
 */
export function getExplicitSetting<T>(settings: ExplicitSettingSource | undefined, key: SettingPath): T | undefined {
	if (!settings) return undefined;
	try {
		if (typeof settings.isConfigured === "function" && !settings.isConfigured(key)) return undefined;
		return settings.get(key) as T;
	} catch {
		return undefined;
	}
}

export interface HardwareInfo {
	tier: "cuda" | "apple_silicon_mlx" | "apple_silicon_mps" | "cpu" | string;
	device: string;
	device_name: string;
	reason: string;
	signature: string;
	details?: Record<string, unknown>;
}

export interface BenchmarkMeasurement {
	minMs: number;
	medianMs: number;
	samplesMs: number[];
}

export interface DerivedLayaSettings {
	/** Measured raw single-choice latency (ms) */
	rawSingleChoiceLatencyMs: number;
	/** Safe timeout for subagent selection (roughly 2x measured single choice latency) */
	subagentSelectionTimeoutMs: number;
	/** Whether subagent selection is recommended to be auto-enabled on this hardware */
	subagentSelectionRecommendEnabled: boolean;
	/** Human-readable explanation for subagent selection recommendation */
	subagentSelectionReason: string;
	/** Whether context pruning is recommended to be enabled given turn latency budget */
	pruningRecommendEnabled: boolean;
	/** Human-readable explanation for pruning recommendation */
	pruningReason: string;
	/** Configured or default maximum acceptable added latency per turn (ms) */
	maxAcceptableLatencyPerTurnMs: number;
	/** Realistic estimated added latency per turn for scoring 1-2 aged-out chunks (ms) */
	estimatedAddedLatencyPerTurnMs: number;
	/** Worst-case batch scoring latency for full candidate pool (B=2, L=1024) (ms) */
	worstCaseBatchLatencyMs: number;
	/** Preserved from shadow-mode review; calibration NEVER derives or overwrites this */
	subagentSelectionConfidenceThreshold?: number;
}

export interface CalibrationRecord {
	timestamp: number;
	hardware: HardwareInfo;
	benchmarks: {
		/** Ultra-short (~40 tokens) sanity baseline */
		ultraShort: BenchmarkMeasurement;
		/** Single choice (~350 tokens) - Phase 2 subagent selection shape */
		singleChoice: BenchmarkMeasurement;
		/** Single score (~150 tokens) - Phase 1 short candidate shape */
		singleScore: BenchmarkMeasurement;
		/** Batched score, same-length (B=2, L=1024) - Phase 1 typical candidate pool shape */
		batchedScore: BenchmarkMeasurement;
	};
	derivedSettings: DerivedLayaSettings;
}

const CALIBRATION_FILENAME = "laya-calibration.json";
export const DEFAULT_MAX_ACCEPTABLE_TURN_LATENCY_MS = 150;
const SANITY_TIMEOUT_CEILING_MS = 15000;
const MINIMUM_TIMEOUT_FLOOR_MS = 300;

/**
 * 4 Representative benchmark payloads matching real Harvest call sites.
 *
 * Question wording is built from the same versioned prompt assets the
 * production call sites send (`laya-prompt-assets`), so calibration measures
 * the deployed questions instead of drifting inline copies.
 */
export const BENCHMARK_PAYLOADS = {
	ultraShort: {
		state: "Task: Check status of running worker processes and report IDs.",
		questions: {
			status_check: {
				type: "noul" as const,
				instructions: CALIBRATION_SANITY_CHECK_INSTRUCTIONS,
			},
		},
	},
	singleChoice: {
		state: {
			task_assignment:
				"Audit all URL and file loading handlers in the codebase for potential path traversal (CWE-22) vulnerabilities and return evidence.",
		},
		questions: {
			subagent_choice: {
				type: "choice" as const,
				instructions: SUBAGENT_SELECTION_INSTRUCTIONS,
				criteria: { ...SUBAGENT_CRITERIA },
			},
		},
	},
	singleScore: {
		state:
			"Goal: Resolve TypeScript compilation errors in connection pool.\nTrace:\nCOMPILER_TRACE: error TS2322: Type 'string' is not assignable to type 'number'. Property 'port' requires integer.",
		questions: {
			score_relevance: {
				type: "score" as const,
				instructions: renderCalibrationRelevanceInstructions("chunk"),
				criteria: [...CALIBRATION_PRUNING_CRITERIA],
			},
		},
	},
	batchedScore: {
		state: {
			chunk_1:
				"Current Task/Goal:\nResolve TypeScript compilation errors in connection pool.\n\nCandidate Chunk (Tool 'bash' result):\n" +
				"COMPILER_TRACE: error TS2322: Type 'string' is not assignable to type 'number'.\n".repeat(45).slice(0, 3200),
			chunk_2:
				"Current Task/Goal:\nResolve TypeScript compilation errors in connection pool.\n\nCandidate Chunk (Tool 'read_file' result):\n" +
				"export interface PoolConfig { host: string; port: number; maxConnections: number; }\n".repeat(40).slice(0, 3200),
		},
		questions: {
			chunk_1: {
				type: "score" as const,
				instructions: renderCalibrationRelevanceInstructions("Tool 'bash' result"),
				criteria: [...CALIBRATION_PRUNING_CRITERIA],
			},
			chunk_2: {
				type: "score" as const,
				instructions: renderCalibrationRelevanceInstructions("Tool 'read_file' result"),
				criteria: [...CALIBRATION_PRUNING_CRITERIA],
			},
		},
	},
};

function calculateMedian(values: number[]): number {
	if (values.length === 0) return 0;
	const sorted = [...values].sort((a, b) => a - b);
	const mid = Math.floor(sorted.length / 2);
	return sorted.length % 2 !== 0 ? sorted[mid] : (sorted[mid - 1] + sorted[mid]) / 2;
}

/**
 * Measure a single benchmark shape against the running Laya sidecar.
 * Executes 1 warmup run (discarded) + `iterations` measurement runs.
 */
export async function measureBenchmarkShape(
	client: LayaClient,
	payload: { state: string | Record<string, unknown> | unknown[]; questions: Record<string, LayaQuestionDefinition> },
	iterations = 3,
): Promise<BenchmarkMeasurement> {
	// Warmup run (discarded to avoid counting one-time JIT/cold-path overhead)
	await client.decide(payload.state, payload.questions, {
		callSite: "calibration_warmup",
		timeoutMs: 180000,
	});

	const samples: number[] = [];
	for (let i = 0; i < iterations; i++) {
		const res = await client.decide(payload.state, payload.questions, {
			callSite: `calibration_run_${i + 1}`,
			timeoutMs: 180000,
		});
		if (res.success && res.latencyMs > 0) {
			samples.push(res.latencyMs);
		} else {
			throw new Error(`Calibration benchmark step failed: ${res.fallbackReason ?? "unknown error"}`);
		}
	}

	const minMs = Math.round(Math.min(...samples) * 10) / 10;
	const medianMs = Math.round(calculateMedian(samples) * 10) / 10;
	return { minMs, medianMs, samplesMs: samples.map(s => Math.round(s * 10) / 10) };
}

/**
 * Derive operational settings from raw benchmark measurements.
 * Follows Step 3 logic: derives thresholds strictly from measurements, not hardware labels.
 */
export function deriveSettingsFromBenchmarks(
	benchmarks: CalibrationRecord["benchmarks"],
	maxAcceptableLatencyPerTurnMs: number = DEFAULT_MAX_ACCEPTABLE_TURN_LATENCY_MS,
): DerivedLayaSettings {
	const choiceMedian = benchmarks.singleChoice.medianMs;
	const singleScoreMedian = benchmarks.singleScore.medianMs;
	const batchedMedian = benchmarks.batchedScore.medianMs;

	// Subagent selection timeout: roughly 2x measured latency with safety floor and ceiling
	const rawTimeout = Math.round(choiceMedian * 2.0);
	const subagentSelectionTimeoutMs = Math.min(
		Math.max(rawTimeout, MINIMUM_TIMEOUT_FLOOR_MS),
		SANITY_TIMEOUT_CEILING_MS,
	);

	// Recommendation on subagent selection:
	// If single choice itself takes > 3,500ms, using Laya on every subagent handoff causes noticeable drag
	let subagentSelectionRecommendEnabled = true;
	let subagentSelectionReason = "";
	if (choiceMedian > 3500) {
		subagentSelectionRecommendEnabled = false;
		subagentSelectionReason = `Single choice decision latency (${choiceMedian}ms) exceeds 3.5s; recommended disabled to prevent interactive turn delay.`;
	} else if (choiceMedian <= 600) {
		subagentSelectionRecommendEnabled = true;
		subagentSelectionReason = `Fast choice latency (${choiceMedian}ms); auto-enabled with ${subagentSelectionTimeoutMs}ms timeout budget.`;
	} else {
		subagentSelectionRecommendEnabled = true;
		subagentSelectionReason = `Moderate choice latency (${choiceMedian}ms); auto-enabled with ${subagentSelectionTimeoutMs}ms safety timeout.`;
	}

	// Pruning enablement:
	// Under decision-locking, turns score only newly-aged-out chunks (typically 1-2 chunks, not B=2/L=1024).
	// Realistic per-turn cost is modeled as ~1.5x singleScore latency (~1-2 chunks of ~150-300 tokens).
	// Batched score (B=2, L=1024) remains stored as the worst-case upper bound.
	const estimatedAddedLatencyPerTurnMs = Math.round(singleScoreMedian * 1.5 * 10) / 10;
	const worstCaseBatchLatencyMs = batchedMedian;
	const pruningRecommendEnabled = estimatedAddedLatencyPerTurnMs <= maxAcceptableLatencyPerTurnMs;
	let pruningReason = "";
	if (pruningRecommendEnabled) {
		pruningReason = `Realistic per-turn scoring latency (${estimatedAddedLatencyPerTurnMs}ms) is within acceptable turn budget of ${maxAcceptableLatencyPerTurnMs}ms (worst-case full batch: ${worstCaseBatchLatencyMs}ms).`;
	} else {
		pruningReason = `Realistic per-turn scoring latency (${estimatedAddedLatencyPerTurnMs}ms) exceeds acceptable added turn budget of ${maxAcceptableLatencyPerTurnMs}ms (worst-case full batch: ${worstCaseBatchLatencyMs}ms). Auto-disabled to avoid slowing down interactive turns.`;
	}

	return {
		rawSingleChoiceLatencyMs: choiceMedian,
		subagentSelectionTimeoutMs,
		subagentSelectionRecommendEnabled,
		subagentSelectionReason,
		pruningRecommendEnabled,
		pruningReason,
		maxAcceptableLatencyPerTurnMs,
		estimatedAddedLatencyPerTurnMs,
		worstCaseBatchLatencyMs,
	};
}

/**
 * Execute full calibration suite against running Laya client.
 */
export async function runCalibrationBenchmark(
	client: LayaClient,
	hardware: HardwareInfo,
	options: { maxAcceptableLatencyPerTurnMs?: number; iterations?: number } = {},
): Promise<CalibrationRecord> {
	const iterations = options.iterations ?? 3;
	const maxAcceptableLatency = options.maxAcceptableLatencyPerTurnMs ?? DEFAULT_MAX_ACCEPTABLE_TURN_LATENCY_MS;

	logger.info("Starting Laya local self-calibration benchmark", {
		tier: hardware.tier,
		device: hardware.device_name,
		signature: hardware.signature,
	});

	// Shape 1: Ultra-short (~40 tokens)
	const ultraShort = await measureBenchmarkShape(client, BENCHMARK_PAYLOADS.ultraShort, iterations);
	// Shape 2: Single choice (~350 tokens)
	const singleChoice = await measureBenchmarkShape(client, BENCHMARK_PAYLOADS.singleChoice, iterations);
	// Shape 3: Single score (~150 tokens)
	const singleScore = await measureBenchmarkShape(client, BENCHMARK_PAYLOADS.singleScore, iterations);
	// Shape 4: Batched score (B=2, L=1024)
	const batchedScore = await measureBenchmarkShape(client, BENCHMARK_PAYLOADS.batchedScore, iterations);

	const benchmarks = { ultraShort, singleChoice, singleScore, batchedScore };
	const derivedSettings = deriveSettingsFromBenchmarks(benchmarks, maxAcceptableLatency);

	const record: CalibrationRecord = {
		timestamp: Date.now(),
		hardware,
		benchmarks,
		derivedSettings,
	};

	logger.info("Completed Laya local self-calibration benchmark", {
		ultraShortMedianMs: ultraShort.medianMs,
		singleChoiceMedianMs: singleChoice.medianMs,
		batchedScoreMedianMs: batchedScore.medianMs,
		derivedTimeoutMs: derivedSettings.subagentSelectionTimeoutMs,
		pruningEnabled: derivedSettings.pruningRecommendEnabled,
	});

	return record;
}

/**
 * Get path to calibration cache file.
 */
export function getCalibrationFilePath(agentDir: string = getAgentDir()): string {
	return path.join(agentDir, CALIBRATION_FILENAME);
}

let _memoryCachedCalibration: { agentDir: string; record: CalibrationRecord } | null = null;

export function getMemoryCachedCalibration(): CalibrationRecord | null {
	return _memoryCachedCalibration?.record ?? null;
}

export function setMemoryCachedCalibration(cal: CalibrationRecord | null, agentDir: string = getAgentDir()): void {
	_memoryCachedCalibration = cal ? { agentDir, record: cal } : null;
}

/**
 * Get calibrated timeout in ms synchronously if available, otherwise return fallback.
 */
export function getDerivedTimeoutMsSync(fallback = 300): number {
	return _memoryCachedCalibration?.record?.derivedSettings?.subagentSelectionTimeoutMs ?? fallback;
}

/**
 * Get calibrated pruning recommendation synchronously if available, otherwise return fallback.
 */
export function getDerivedPruningEnabledSync(fallback = true): boolean {
	return _memoryCachedCalibration?.record?.derivedSettings?.pruningRecommendEnabled ?? fallback;
}

/**
 * Load cached calibration record from disk.
 */
export async function loadCalibration(agentDir: string = getAgentDir()): Promise<CalibrationRecord | null> {
	if (_memoryCachedCalibration && _memoryCachedCalibration.agentDir === agentDir) {
		return _memoryCachedCalibration.record;
	}
	const filePath = getCalibrationFilePath(agentDir);
	try {
		const content = await Bun.file(filePath).text();
		const parsed = JSON.parse(content) as CalibrationRecord;
		_memoryCachedCalibration = { agentDir, record: parsed };
		return parsed;
	} catch (err) {
		if (isEnoent(err)) return null;
		logger.warn("Failed to read Laya calibration file", { filePath, err: String(err) });
		return null;
	}
}

/**
 * Save calibration record to disk.
 */
export async function saveCalibration(
	record: CalibrationRecord,
	agentDir: string = getAgentDir(),
): Promise<void> {
	_memoryCachedCalibration = { agentDir, record };
	const filePath = getCalibrationFilePath(agentDir);
	await Bun.write(filePath, JSON.stringify(record, null, 2) + "\n");
	logger.info("Saved Laya calibration record", { filePath, signature: record.hardware.signature });
}

/**
 * Check if the active calibration matches the detected hardware signature.
 */
export function isCalibrationValidForHardware(
	calibration: CalibrationRecord | null,
	currentHardware: HardwareInfo,
): boolean {
	if (!calibration || !calibration.hardware || !calibration.derivedSettings) {
		return false;
	}
	return calibration.hardware.signature === currentHardware.signature;
}

/**
 * Ensure calibrated: loads cached calibration if valid for current hardware;
 * otherwise runs calibration against sidecar and persists to disk.
 */
export async function ensureCalibrated(
	client: LayaClient,
	options: {
		agentDir?: string;
		force?: boolean;
		maxAcceptableLatencyPerTurnMs?: number;
		iterations?: number;
	} = {},
): Promise<CalibrationRecord> {
	const agentDir = options.agentDir ?? getAgentDir();
	const hardware = await client.getHardwareInfo();
	if (!hardware) {
		throw new Error("Unable to retrieve hardware info from Laya sidecar. Is the sidecar running?");
	}

	const cached = await loadCalibration(agentDir);
	if (!options.force && isCalibrationValidForHardware(cached, hardware)) {
		return cached!;
	}

	// Hardware signature changed or no cache exists: run benchmark
	const record = await runCalibrationBenchmark(client, hardware, {
		maxAcceptableLatencyPerTurnMs: options.maxAcceptableLatencyPerTurnMs,
		iterations: options.iterations,
	});

	// Preserve confidence threshold if one was set by shadow-mode review
	if (cached?.derivedSettings?.subagentSelectionConfidenceThreshold !== undefined) {
		record.derivedSettings.subagentSelectionConfidenceThreshold =
			cached.derivedSettings.subagentSelectionConfidenceThreshold;
	}

	await saveCalibration(record, agentDir);
	return record;
}
