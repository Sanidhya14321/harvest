import { describe, expect, it } from "bun:test";
import * as os from "node:os";
import * as path from "node:path";
import * as fs from "node:fs/promises";
import {
	deriveSettingsFromBenchmarks,
	isCalibrationValidForHardware,
	loadCalibration,
	saveCalibration,
	type BenchmarkMeasurement,
	type CalibrationRecord,
	type HardwareInfo,
} from "../src/core/harvest/laya-calibration";

describe("Laya Hardware Detection & Self-Calibration", () => {
	const sampleHardware: HardwareInfo = {
		tier: "cpu",
		device: "cpu",
		device_name: "Intel Core i7-8650U (8 cores)",
		reason: "CPU execution active: 8 cores (MKL=True, oneDNN=True)",
		signature: "cpu-intel-8650u-sig",
		details: { logical_cores: 8 },
	};

	const makeMeasurement = (medianMs: number, minMs?: number): BenchmarkMeasurement => ({
		medianMs,
		minMs: minMs ?? medianMs * 0.9,
		samplesMs: [medianMs * 0.9, medianMs, medianMs * 1.1],
	});

	describe("deriveSettingsFromBenchmarks", () => {
		it("derives sub-300ms safe floor and enables pruning for fast GPU hardware", () => {
			const fastGpuBenchmarks = {
				ultraShort: makeMeasurement(12.5),
				singleChoice: makeMeasurement(45.0),
				singleScore: makeMeasurement(28.0),
				batchedScore: makeMeasurement(110.0), // 110ms added turn latency
			};

			const derived = deriveSettingsFromBenchmarks(fastGpuBenchmarks, 200);

			// 45ms * 2 = 90ms, but floor is 300ms to absorb OS scheduling jitter
			expect(derived.subagentSelectionTimeoutMs).toBe(300);
			expect(derived.subagentSelectionRecommendEnabled).toBe(true);
			// 110ms <= 200ms max acceptable turn latency budget
			expect(derived.pruningRecommendEnabled).toBe(true);
			expect(derived.estimatedAddedLatencyPerTurnMs).toBe(110.0);
		});

		it("derives safe timeout multiplier and automatically disables pruning on slow CPU hardware", () => {
			const slowCpuBenchmarks = {
				ultraShort: makeMeasurement(950.0),
				singleChoice: makeMeasurement(2600.0),
				singleScore: makeMeasurement(1400.0),
				batchedScore: makeMeasurement(22500.0), // 22.5s added turn latency
			};

			const derived = deriveSettingsFromBenchmarks(slowCpuBenchmarks, 200);

			// 2600ms * 2 = 5200ms timeout budget for subagent selection
			expect(derived.subagentSelectionTimeoutMs).toBe(5200);
			expect(derived.subagentSelectionRecommendEnabled).toBe(true);
			// 22500ms > 200ms -> pruning MUST be automatically disabled by default
			expect(derived.pruningRecommendEnabled).toBe(false);
			expect(derived.pruningReason).toContain("exceeds acceptable added turn budget of 200ms");
		});

		it("flags subagent selection as not recommended when single choice latency exceeds 3.5s", () => {
			const overloadedCpuBenchmarks = {
				ultraShort: makeMeasurement(1500.0),
				singleChoice: makeMeasurement(4200.0), // > 3.5s
				singleScore: makeMeasurement(2800.0),
				batchedScore: makeMeasurement(38000.0),
			};

			const derived = deriveSettingsFromBenchmarks(overloadedCpuBenchmarks, 200);

			expect(derived.subagentSelectionRecommendEnabled).toBe(false);
			expect(derived.subagentSelectionReason).toContain("exceeds 3.5s");
			expect(derived.subagentSelectionTimeoutMs).toBe(8400);
		});
	});

	describe("isCalibrationValidForHardware", () => {
		const validRecord: CalibrationRecord = {
			timestamp: Date.now(),
			hardware: sampleHardware,
			benchmarks: {
				ultraShort: makeMeasurement(20),
				singleChoice: makeMeasurement(50),
				singleScore: makeMeasurement(30),
				batchedScore: makeMeasurement(100),
			},
			derivedSettings: {
				subagentSelectionTimeoutMs: 300,
				subagentSelectionRecommendEnabled: true,
				subagentSelectionReason: "Fast",
				pruningRecommendEnabled: true,
				pruningReason: "Within budget",
				maxAcceptableLatencyPerTurnMs: 200,
				estimatedAddedLatencyPerTurnMs: 100,
			},
		};

		it("returns true when calibration signature matches current detected hardware", () => {
			expect(isCalibrationValidForHardware(validRecord, sampleHardware)).toBe(true);
		});

		it("returns false when hardware signature changes (triggering auto-recalibration)", () => {
			const upgradedGpuHardware: HardwareInfo = {
				...sampleHardware,
				tier: "cuda",
				device: "cuda",
				device_name: "NVIDIA GeForce RTX 4090",
				signature: "cuda-rtx-4090-new-sig",
			};

			expect(isCalibrationValidForHardware(validRecord, upgradedGpuHardware)).toBe(false);
		});

		it("returns false when calibration record is null or missing fields", () => {
			expect(isCalibrationValidForHardware(null, sampleHardware)).toBe(false);
			expect(isCalibrationValidForHardware({} as any, sampleHardware)).toBe(false);
		});
	});

	describe("Persistence: loadCalibration & saveCalibration", () => {
		it("persists calibration record to disk and restores identical data", async () => {
			const tmpDir = await fs.mkdtemp(path.join(os.tmpdir(), "harvest-cal-test-"));
			try {
				const record: CalibrationRecord = {
					timestamp: 1720000000000,
					hardware: sampleHardware,
					benchmarks: {
						ultraShort: makeMeasurement(15),
						singleChoice: makeMeasurement(65),
						singleScore: makeMeasurement(35),
						batchedScore: makeMeasurement(140),
					},
					derivedSettings: {
						subagentSelectionTimeoutMs: 300,
						subagentSelectionRecommendEnabled: true,
						subagentSelectionReason: "Fast choice latency",
						pruningRecommendEnabled: true,
						pruningReason: "Batched scoring within budget",
						maxAcceptableLatencyPerTurnMs: 200,
						estimatedAddedLatencyPerTurnMs: 140,
					},
				};

				await saveCalibration(record, tmpDir);

				// Bypass in-memory cache by reading from fresh tmpDir
				const restored = await loadCalibration(tmpDir);
				expect(restored).not.toBeNull();
				expect(restored?.hardware.signature).toBe(sampleHardware.signature);
				expect(restored?.benchmarks.singleChoice.medianMs).toBe(65);
				expect(restored?.derivedSettings.subagentSelectionTimeoutMs).toBe(300);
				expect(restored?.derivedSettings.pruningRecommendEnabled).toBe(true);
			} finally {
				await fs.rm(tmpDir, { recursive: true, force: true });
			}
		});

		it("returns null gracefully when calibration file does not exist", async () => {
			const nonExistentDir = path.join(os.tmpdir(), `non-existent-${Date.now()}`);
			const loaded = await loadCalibration(nonExistentDir);
			expect(loaded).toBeNull();
		});
	});
});
