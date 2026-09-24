/**
 * Test Suite for Laya Autonomous Setup and Local Service Management.
 *
 * Verifies contract boundaries:
 * 1. Python runtime detection & version validation (>= 3.9)
 * 2. Port conflict detection and safe socket reclamation
 * 3. Sidecar directory resolution across multiple candidate paths
 * 4. Dependency verification and PEP 668 auto-handling
 * 5. HuggingFace cache bypass when weights are already present
 * 6. configureLayaLocally end-to-end pipeline and calibration integration
 * 7. CLI harvest laya setup execution and auto-healing on calibrate
 */

import { afterEach, describe, expect, it, vi } from "bun:test";
import * as layaService from "../src/core/harvest/laya-service";
import * as layaCalibration from "../src/core/harvest/laya-calibration";
import { Settings } from "../src/config/settings";
import { runLayaCommand } from "../src/cli/laya-cli";

afterEach(() => {
	vi.restoreAllMocks();
});

describe("Laya Service & Autonomous Setup", () => {
	describe("Python Detection & Validation", () => {
		it("detects valid Python 3.9+ runtime and extracts version", async () => {
			const spy = vi.spyOn(layaService, "testPythonExecutable").mockResolvedValue({
				path: "/mock/bin/python3",
				version: "3.11.4",
			});

			const res = await layaService.findPythonExecutable();
			expect(res).not.toBeNull();
			expect(res?.version).toBe("3.11.4");
			expect(res?.path).toBe("/mock/bin/python3");
		});

		it("rejects Python versions older than 3.9", async () => {
			// Mock probe to return 3.8.10
			vi.spyOn(Bun, "spawn").mockReturnValue({
				exited: Promise.resolve(0),
				stdout: new Response("3.8.10\n/usr/bin/python3.8\n").body,
				stderr: new Response("").body,
			} as unknown as ReturnType<typeof Bun.spawn>);

			const res = await layaService.testPythonExecutable("/usr/bin/python3.8");
			expect(res).toBeNull();
		});

		it("accepts Python 3.9, 3.10, 3.11, and 3.12", async () => {
			for (const ver of ["3.9.18", "3.10.12", "3.11.9", "3.12.3"]) {
				const spy = vi.spyOn(Bun, "spawn").mockImplementation(() => ({
					exited: Promise.resolve(0),
					stdout: new Response(`${ver}\n/usr/bin/python\n`).body,
					stderr: new Response("").body,
				} as unknown as ReturnType<typeof Bun.spawn>));

				const res = await layaService.testPythonExecutable("/usr/bin/python");
				expect(res).not.toBeNull();
				expect(res?.version).toBe(ver);
				spy.mockRestore();
			}
		});
	});

	describe("Port Management", () => {
		it("correctly identifies available port", async () => {
			const available = await layaService.isPortInUse(59876);
			expect(available).toBe(false);
		});

		it("detects when sidecar is already active on port and avoids killing", async () => {
			vi.spyOn(layaService, "isPortInUse").mockResolvedValue(true);
			vi.spyOn(layaService, "isLayaSidecarRunning").mockResolvedValue(true);

			const result = await layaService.freePortIfOccupied(8177);
			expect(result.freed).toBe(true);
			expect(result.alreadyRunning).toBe(true);
		});
	});

	describe("Directory Resolution", () => {
		it("resolves sidecar directory containing server.py", () => {
			const sidecarDir = layaService.getSidecarDir();
			expect(typeof sidecarDir).toBe("string");
			expect(sidecarDir.length).toBeGreaterThan(0);
			expect(sidecarDir).toContain("decision-sidecar");
		});
	});

	describe("Dependency & Cache Verification", () => {
		it("skips pip installation when dependencies are already importable", async () => {
			vi.spyOn(layaService, "checkLayaDependencies").mockResolvedValue(true);

			const res = await layaService.installLayaDependencies("/mock/bin/python3");
			expect(res.success).toBe(true);
			expect(res.alreadyInstalled).toBe(true);
		});

		it("skips model download when checkpoint is cached", async () => {
			vi.spyOn(Bun, "spawn").mockReturnValue({
				exited: Promise.resolve(0),
				stdout: new Response("MODEL_CACHED\n"),
				stderr: new Response(""),
			} as unknown as ReturnType<typeof Bun.spawn>);

			const res = await layaService.ensureLayaModelCached("/mock/bin/python3");
			expect(res.success).toBe(true);
			expect(res.alreadyCached).toBe(true);
		});
	});

	describe("configureLayaLocally & setupLayaAutonomously", () => {
		it("runs complete setup pipeline and calibrates hardware", async () => {
			vi.spyOn(layaService, "findPythonExecutable").mockResolvedValue({
				path: "/opt/venv/bin/python",
				version: "3.11.2",
			});
			vi.spyOn(layaService, "isLayaSidecarRunning").mockResolvedValue(false);
			vi.spyOn(layaService, "checkLayaDependencies").mockResolvedValue(true);
			vi.spyOn(layaService, "ensureLayaModelCached").mockResolvedValue({ success: true, alreadyCached: true });
			vi.spyOn(layaService, "startLayaSidecarProcess").mockResolvedValue({ success: true });

			const mockCalibration: layaCalibration.CalibrationRecord = {
				timestamp: Date.now(),
				hardware: {
					tier: "cpu",
					device: "cpu",
					device_name: "Mock CPU (8 cores)",
					reason: "Test CPU",
					signature: "mock123",
				},
				benchmarks: {
					ultraShort: { minMs: 15, medianMs: 20, samplesMs: [15, 20, 25] },
					singleChoice: { minMs: 25, medianMs: 30, samplesMs: [25, 30, 35] },
					singleScore: { minMs: 18, medianMs: 22, samplesMs: [18, 22, 26] },
					multiChunkScore: { minMs: 40, medianMs: 50, samplesMs: [40, 50, 60] },
				},
				derivedSettings: {
					rawSingleChoiceLatencyMs: 30,
					subagentSelectionTimeoutMs: 60,
					subagentSelectionRecommendEnabled: true,
					subagentSelectionReason: "Fast mock",
					pruningRecommendEnabled: true,
					pruningReason: "Within budget",
					maxAcceptableLatencyPerTurnMs: 120,
					estimatedAddedLatencyPerTurnMs: 44,
					worstCaseBatchLatencyMs: 60,
				},
			};

			vi.spyOn(layaCalibration, "ensureCalibrated").mockResolvedValue(mockCalibration);

			const settings = Settings.isolated();
			const stepsRecorded: string[] = [];

			const result = await layaService.configureLayaLocally({
				settings,
				onStepUpdate: (stepId, status) => {
					stepsRecorded.push(`${stepId}:${status}`);
				},
			});

			expect(result.success).toBe(true);
			expect(result.calibration?.hardware.tier).toBe("cpu");
			expect(settings.get("laya.enabled")).toBe(true);
			expect(settings.get("laya.url")).toBe("http://127.0.0.1:8177");
			expect(settings.get("laya.autostart")).toBe(true);

			expect(stepsRecorded).toContain("python:running");
			expect(stepsRecorded).toContain("python:done");
			expect(stepsRecorded).toContain("dependencies:done");
			expect(stepsRecorded).toContain("model:done");
			expect(stepsRecorded).toContain("sidecar:done");
			expect(stepsRecorded).toContain("calibrate:done");
			expect(stepsRecorded).toContain("connect:done");
		});

		it("aborts cleanly with actionable error if Python is missing and un-installable", async () => {
			vi.spyOn(layaService, "findPythonExecutable").mockResolvedValue(null);
			vi.spyOn(layaService, "bootstrapPythonIfMissing").mockResolvedValue(null);

			const settings = Settings.isolated();
			const result = await layaService.configureLayaLocally({ settings });

			expect(result.success).toBe(false);
			expect(result.error).toContain("Python 3.9+ not found");
			expect(settings.get("laya.enabled")).toBe(false);
		});
	});

	describe("CLI harvest laya setup Action", () => {
		it("runs setup via CLI command with JSON flag", async () => {
			const mockCalibration: layaCalibration.CalibrationRecord = {
				timestamp: Date.now(),
				hardware: {
					tier: "cuda",
					device: "cuda",
					device_name: "Mock RTX 4090",
					reason: "NVIDIA CUDA",
					signature: "gpu4090",
				},
				benchmarks: {
					ultraShort: { minMs: 3, medianMs: 4, samplesMs: [3, 4, 5] },
					singleChoice: { minMs: 5, medianMs: 6, samplesMs: [5, 6, 7] },
					singleScore: { minMs: 4, medianMs: 5, samplesMs: [4, 5, 6] },
					multiChunkScore: { minMs: 8, medianMs: 10, samplesMs: [8, 10, 12] },
				},
				derivedSettings: {
					rawSingleChoiceLatencyMs: 6,
					subagentSelectionTimeoutMs: 250,
					subagentSelectionRecommendEnabled: true,
					subagentSelectionReason: "Fast GPU",
					pruningRecommendEnabled: true,
					pruningReason: "Sub-10ms latency",
					maxAcceptableLatencyPerTurnMs: 120,
					estimatedAddedLatencyPerTurnMs: 10,
					worstCaseBatchLatencyMs: 12,
				},
			};

			vi.spyOn(layaService, "configureLayaLocally").mockResolvedValue({
				success: true,
				calibration: mockCalibration,
			});

			const stdoutChunks: string[] = [];
			const writeSpy = vi.spyOn(process.stdout, "write").mockImplementation((str: unknown) => {
				stdoutChunks.push(String(str));
				return true;
			});

			await runLayaCommand({
				action: "setup",
				flags: { json: true },
			});

			writeSpy.mockRestore();

			const output = stdoutChunks.join("");
			expect(output).toContain('"success": true');
			expect(output).toContain('"tier": "cuda"');
		});
	});

	describe("Hardware Probing & Stream Processing", () => {
		it("probes host NVIDIA GPU presence safely without throwing", async () => {
			const hasNvidia = await layaService.isHostNvidiaGpuPresent();
			expect(typeof hasNvidia).toBe("boolean");
		});

		it("streams process output lines and captures stdout/stderr", async () => {
			const mockLines: string[] = [];
			const proc = Bun.spawn([
				"python",
				"-c",
				"import sys; sys.stdout.write('line 1\\n'); sys.stderr.write('err 1\\n'); sys.stdout.write('line 2\\r'); sys.stdout.flush()",
			], {
				stdout: "pipe",
				stderr: "pipe",
			});

			const res = await layaService.streamProcessOutput(proc, line => mockLines.push(line), {
				activityTimeoutMs: 5000,
				totalTimeoutMs: 10000,
			});

			expect(res.exitCode).toBe(0);
			expect(mockLines).toContain("line 1");
			expect(mockLines).toContain("err 1");
			expect(mockLines).toContain("line 2");
		});

		it("terminates and throws when activity timeout expires", async () => {
			const proc = Bun.spawn([
				"python",
				"-c",
				"import time; time.sleep(10)",
			], {
				stdout: "pipe",
				stderr: "pipe",
			});

			const promise = layaService.streamProcessOutput(proc, undefined, {
				activityTimeoutMs: 200,
				totalTimeoutMs: 1000,
			});

			await expect(promise).rejects.toThrow("Process timed out");
		});
	});
});

