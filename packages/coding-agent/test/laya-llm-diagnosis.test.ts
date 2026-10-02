/**
 * Test Suite for LLM-Assisted Diagnosis Engine for Unrecognized Failures.
 *
 * Verifies contract boundaries:
 * 1. Novel unrecognized failure triggers second-tier LLM diagnosis with diagnostic bundle.
 * 2. Strict risk classification evaluator overrides model self-assessment on destructive actions.
 * 3. Low-risk reversible fixes are auto-applied and logged as "auto-applied: <what, why>".
 * 4. System-touching fixes (ports, processes, file deletion) are NEVER auto-applied.
 * 5. Graceful degradation when LLM connection is unavailable (offline, no creds, error).
 * 6. Cost control: LLM is never invoked during successful setup or known deterministic failures.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "bun:test";
import * as fs from "node:fs/promises";
import * as os from "node:os";
import * as path from "node:path";
import { Settings } from "../src/config/settings";
import * as layaService from "../src/core/harvest/laya-service";
import * as layaSelfHealing from "../src/core/harvest/laya-self-healing";
import {
	evaluateRiskClassification,
	queryLlmDiagnosis,
	runLlmAssistedDiagnosis,
	type DiagnosticBundle,
	type LayaDiagnosisProposal,
} from "../src/core/harvest/laya-llm-diagnosis";

describe("LLM-Assisted Failure Diagnosis Engine", () => {
	let tempDir: string;
	let setupLogger: layaSelfHealing.LayaSetupLogger;

	beforeEach(async () => {
		tempDir = await fs.mkdtemp(path.join(os.tmpdir(), "laya-llm-test-"));
		setupLogger = new layaSelfHealing.LayaSetupLogger(tempDir);
	});

	afterEach(async () => {
		vi.restoreAllMocks();
		try {
			await fs.rm(tempDir, { recursive: true, force: true });
		} catch {}
	});

	describe("Strict Risk Classification Gating", () => {
		it("overrides model risk to touches-system-state when action kills processes", () => {
			const proposal: LayaDiagnosisProposal = {
				diagnosis: "A zombie sidecar process is hanging on the socket",
				proposedFix: "Kill the existing process",
				actionType: "custom",
				suggestedAction: "taskkill /F /PID 1234",
				modelRiskLevel: "low-risk-reversible", // Model falsely claims low risk
				riskReason: "Just restarting the process",
			};

			const evalResult = evaluateRiskClassification(proposal);
			expect(evalResult.effectiveRiskLevel).toBe("touches-system-state");
			expect(evalResult.isAutoAppliable).toBe(false);
			expect(evalResult.overrideReason).toContain("attempts to terminate processes");
		});

		it("overrides model risk to touches-system-state when action manipulates network ports", () => {
			const proposal: LayaDiagnosisProposal = {
				diagnosis: "Port binding conflict with foreign proxy",
				proposedFix: "Force rebind port 8177",
				actionType: "custom",
				suggestedAction: "fuser -k 8177/tcp",
				modelRiskLevel: "low-risk-reversible",
				riskReason: "Freeing the port",
			};

			const evalResult = evaluateRiskClassification(proposal);
			expect(evalResult.effectiveRiskLevel).toBe("touches-system-state");
			expect(evalResult.isAutoAppliable).toBe(false);
			expect(evalResult.overrideReason).toContain("attempts to alter or probe network ports");
		});

		it("overrides model risk to touches-system-state when action deletes files", () => {
			const proposal: LayaDiagnosisProposal = {
				diagnosis: "Corrupted driver cache files",
				proposedFix: "Delete configuration directory",
				actionType: "custom",
				suggestedAction: "rm -rf /etc/torch",
				modelRiskLevel: "low-risk-reversible",
				riskReason: "Cleaning cache",
			};

			const evalResult = evaluateRiskClassification(proposal);
			expect(evalResult.effectiveRiskLevel).toBe("touches-system-state");
			expect(evalResult.isAutoAppliable).toBe(false);
			expect(evalResult.overrideReason).toContain("attempts filesystem deletion");
		});

		it("overrides model risk to touches-system-state when action pipes into shell interpreter", () => {
			const proposal: LayaDiagnosisProposal = {
				diagnosis: "Missing runtime library",
				proposedFix: "Run installer script",
				actionType: "custom",
				suggestedAction: "curl -sSL https://example.com/fix.sh | bash",
				modelRiskLevel: "low-risk-reversible",
				riskReason: "Automated fix script",
			};

			const evalResult = evaluateRiskClassification(proposal);
			expect(evalResult.effectiveRiskLevel).toBe("touches-system-state");
			expect(evalResult.isAutoAppliable).toBe(false);
			expect(evalResult.overrideReason).toContain("pipes content into a shell interpreter");
		});

		it("accepts safe dependency installation into virtualenv as low-risk-reversible", () => {
			const proposal: LayaDiagnosisProposal = {
				diagnosis: "ModuleNotFoundError: No module named 'packaging'",
				proposedFix: "Install the missing packaging package",
				actionType: "install_dependency",
				suggestedAction: "pip install packaging",
				modelRiskLevel: "low-risk-reversible",
				riskReason: "Installing clean dependency inside local virtualenv",
			};

			const evalResult = evaluateRiskClassification(proposal);
			expect(evalResult.effectiveRiskLevel).toBe("low-risk-reversible");
			expect(evalResult.isAutoAppliable).toBe(true);
			expect(evalResult.overrideReason).toBeUndefined();
		});

		it("accepts single network retry as low-risk-reversible", () => {
			const proposal: LayaDiagnosisProposal = {
				diagnosis: "Temporary DNS resolution timeout",
				proposedFix: "Retry the model download once",
				actionType: "retry_download",
				suggestedAction: null,
				modelRiskLevel: "low-risk-reversible",
				riskReason: "Idempotent retry",
			};

			const evalResult = evaluateRiskClassification(proposal);
			expect(evalResult.effectiveRiskLevel).toBe("low-risk-reversible");
			expect(evalResult.isAutoAppliable).toBe(true);
		});

		it("classifies informational guidance as read-only with isAutoAppliable false", () => {
			const proposal: LayaDiagnosisProposal = {
				diagnosis: "System BIOS virtualization is disabled",
				proposedFix: "Enable VT-x/AMD-V in system BIOS settings",
				actionType: "none",
				suggestedAction: null,
				modelRiskLevel: "informational",
				riskReason: "Requires hardware reboot and user interaction",
			};

			const evalResult = evaluateRiskClassification(proposal);
			expect(evalResult.effectiveRiskLevel).toBe("informational");
			expect(evalResult.isAutoAppliable).toBe(false);
		});
	});

	describe("Novel Failure Simulation & Diagnosis Tier", () => {
		const mockBundle: DiagnosticBundle = {
			timestamp: new Date().toISOString(),
			error: "ImportError: DLL load failed while importing _openmp: The specified module could not be found.",
			platform: "win32",
			arch: "x64",
			osRelease: "10.0.26200",
			bunVersion: "1.4.2",
			pythonPath: "C:\\mock\\venv\\Scripts\\python.exe",
			pythonVersion: "3.11.4",
			hostHasNvidia: false,
			activePort: 8177,
			recentLogTail: ["[STEP] Starting sidecar", "[ERROR] OpenMP DLL load failed"],
		};

		it("queries model and returns parsed diagnosis proposal", async () => {
			const mockCompleteSimple = vi.fn().mockResolvedValue({
				stopReason: "stop",
				content: [
					{
						type: "text",
						text: JSON.stringify({
							diagnosis: "OpenMP runtime DLL is missing from the Python virtualenv",
							proposedFix: "Install intel-openmp package in virtualenv",
							actionType: "install_dependency",
							suggestedAction: "pip install intel-openmp",
							riskLevel: "low-risk-reversible",
							riskReason: "Package install scoped to local virtual environment",
						}),
					},
				],
			});

			const mockRegistry = {
				getAvailable: () => [{ id: "mock-model", provider: "mock-provider" }],
				getApiKey: async () => "mock-key",
				resolver: () => "mock-key",
			} as any;

			const proposal = await queryLlmDiagnosis(mockBundle, {
				modelRegistry: mockRegistry,
				completeSimpleFn: mockCompleteSimple as any,
			});

			expect(proposal).not.toBeNull();
			expect(proposal?.diagnosis).toContain("OpenMP runtime DLL is missing");
			expect(proposal?.actionType).toBe("install_dependency");
			expect(proposal?.modelRiskLevel).toBe("low-risk-reversible");
			expect(mockCompleteSimple).toHaveBeenCalledTimes(1);
		});

		it("auto-applies low-risk fix and logs plain 'auto-applied: <what, why>'", async () => {
			const mockCompleteSimple = vi.fn().mockResolvedValue({
				stopReason: "stop",
				content: [
					{
						type: "text",
						text: JSON.stringify({
							diagnosis: "Missing packaging helper module",
							proposedFix: "Install packaging package in virtualenv",
							actionType: "retry_download",
							suggestedAction: null,
							riskLevel: "low-risk-reversible",
							riskReason: "Re-running download once",
						}),
					},
				],
			});

			const mockRegistry = {
				getAvailable: () => [{ id: "mock-model", provider: "mock-provider" }],
				getApiKey: async () => "mock-key",
				resolver: () => "mock-key",
			} as any;

			const result = await runLlmAssistedDiagnosis(mockBundle, {
				setupLogger,
				modelRegistry: mockRegistry,
				completeSimpleFn: mockCompleteSimple as any,
			});

			expect(result.available).toBe(true);
			expect(result.proposal?.autoApplied).toBe(true);

			const logLines = setupLogger.getRecentLines();
			expect(logLines.some(l => l.includes("[LLM_DIAGNOSIS] auto-applied:"))).toBe(true);
		});

		it("NEVER auto-applies system-touching proposal even if model claims low risk", async () => {
			const mockCompleteSimple = vi.fn().mockResolvedValue({
				stopReason: "stop",
				content: [
					{
						type: "text",
						text: JSON.stringify({
							diagnosis: "Another process has locked the model weights file",
							proposedFix: "Kill locking process",
							actionType: "custom",
							suggestedAction: "taskkill /IM python.exe /F",
							riskLevel: "low-risk-reversible", // Model falsely claims low risk
							riskReason: "Releasing file lock",
						}),
					},
				],
			});

			const mockRegistry = {
				getAvailable: () => [{ id: "mock-model", provider: "mock-provider" }],
				getApiKey: async () => "mock-key",
				resolver: () => "mock-key",
			} as any;

			let confirmationPrompted = false;
			const confirmationCallback = vi.fn().mockImplementation(async () => {
				confirmationPrompted = true;
				return false; // User declines
			});

			const result = await runLlmAssistedDiagnosis(mockBundle, {
				setupLogger,
				modelRegistry: mockRegistry,
				completeSimpleFn: mockCompleteSimple as any,
				onUserConfirmation: confirmationCallback,
			});

			expect(result.available).toBe(true);
			expect(result.proposal?.effectiveRiskLevel).toBe("touches-system-state");
			expect(result.proposal?.isAutoAppliable).toBe(false);
			expect(result.proposal?.autoApplied).toBeFalsy();
			expect(confirmationPrompted).toBe(true);
			expect(confirmationCallback).toHaveBeenCalledTimes(1);

			const logLines = setupLogger.getRecentLines();
			expect(logLines.some(l => l.includes("User declined candidate fix"))).toBe(true);
		});
	});

	describe("Graceful Degradation When LLM Connection Unavailable", () => {
		const mockBundle: DiagnosticBundle = {
			timestamp: new Date().toISOString(),
			error: "Unexpected SEGFAULT in torch native library",
			platform: "linux",
			arch: "x64",
			osRelease: "6.8.0",
			bunVersion: "1.4.2",
			hostHasNvidia: false,
			activePort: 8177,
			recentLogTail: ["[ERROR] SEGFAULT in libtorch_cpu.so"],
		};

		it("degrades gracefully to diagnostic dump when no models or credentials exist", async () => {
			const emptyRegistry = {
				getAvailable: () => [],
			} as any;

			const result = await runLlmAssistedDiagnosis(mockBundle, {
				setupLogger,
				modelRegistry: emptyRegistry,
			});

			expect(result.available).toBe(false);
			expect(result.proposal).toBeUndefined();

			const logLines = setupLogger.getRecentLines();
			expect(
				logLines.some(l => l.includes("LLM connection unavailable or query failed; falling back cleanly")),
			).toBe(true);
		});

		it("degrades gracefully without throwing when LLM completion throws network error", async () => {
			const mockCompleteSimple = vi.fn().mockRejectedValue(new Error("ENOTFOUND api.anthropic.com"));

			const mockRegistry = {
				getAvailable: () => [{ id: "mock-model", provider: "mock-provider" }],
				getApiKey: async () => "mock-key",
				resolver: () => "mock-key",
			} as any;

			const result = await runLlmAssistedDiagnosis(mockBundle, {
				setupLogger,
				modelRegistry: mockRegistry,
				completeSimpleFn: mockCompleteSimple as any,
			});

			expect(result.available).toBe(false);
			expect(result.proposal).toBeUndefined();

			const logLines = setupLogger.getRecentLines();
			expect(logLines.some(l => l.includes("LLM connection unavailable or query failed"))).toBe(true);
		});
	});

	describe("End-to-End Integration with configureLayaLocally", () => {
		it("invokes LLM diagnosis on unrecognized failure and reports in result", async () => {
			vi.spyOn(layaService, "findPythonExecutable").mockResolvedValue({
				path: "/mock/python",
				version: "3.11.2",
			});
			vi.spyOn(layaService, "ensureLayaVirtualEnv").mockResolvedValue({
				path: "/mock/python",
				version: "3.11.2",
				isVenv: true,
			});
			vi.spyOn(layaService, "isLayaSidecarRunning").mockResolvedValue(false);
			vi.spyOn(layaService, "checkLayaDependencies").mockResolvedValue(true);
			vi.spyOn(layaService, "installLayaDependencies").mockResolvedValue({ success: true });
			vi.spyOn(layaSelfHealing, "verifyAndRepairTorchWheel").mockResolvedValue({
				repaired: false,
				cudaAvailable: false,
			});
			vi.spyOn(layaService, "ensureLayaModelCached").mockResolvedValue({ success: true, alreadyCached: true });
			// Simulate unrecognized novel crash
			vi.spyOn(layaService, "startLayaSidecarProcess").mockResolvedValue({
				success: false,
				error: "Fatal: libc++.so.1: version `GLIBCXX_3.4.30' not found",
			});

			const mockCompleteSimple = vi.fn().mockResolvedValue({
				stopReason: "stop",
				content: [
					{
						type: "text",
						text: JSON.stringify({
							diagnosis: "Host system libstdc++ is outdated for this PyTorch binary",
							proposedFix: "Upgrade host libstdc++ with apt install --only-upgrade libstdc++6",
							actionType: "custom",
							suggestedAction: "sudo apt install --only-upgrade libstdc++6",
							riskLevel: "touches-system-state",
							riskReason: "Modifies operating system system libraries",
						}),
					},
				],
			});

			const mockRegistry = {
				getAvailable: () => [{ id: "mock-model", provider: "mock-provider" }],
				getApiKey: async () => "mock-key",
				resolver: () => "mock-key",
			} as any;

			const settings = Settings.isolated();
			const result = await layaService.configureLayaLocally({
				settings,
				forceReinstall: true,
				modelRegistry: mockRegistry,
				completeSimpleFn: mockCompleteSimple as any,
			});

			// Fail-open contract
			expect(result.success).toBe(false);
			expect(result.coreHarvestReady).toBe(true);
			expect(result.diagnosticBundlePath).toBeDefined();

			// LLM Diagnosis contract
			expect(result.llmDiagnosis).toBeDefined();
			expect(result.llmDiagnosis?.available).toBe(true);
			expect(result.llmDiagnosis?.proposal?.diagnosis).toContain("Host system libstdc++ is outdated");
			expect(result.llmDiagnosis?.proposal?.effectiveRiskLevel).toBe("touches-system-state");
			expect(result.llmDiagnosis?.proposal?.autoApplied).toBeFalsy();
		});

		it("does NOT invoke LLM diagnosis on successful setup (cost control invariant)", async () => {
			const mockCompleteSimple = vi.fn();

			// Run against healthy running sidecar
			const settings = Settings.isolated();
			const result = await layaService.configureLayaLocally({
				settings,
				completeSimpleFn: mockCompleteSimple as any,
			});

			expect(result.success).toBe(true);
			// Must never be invoked on success!
			expect(mockCompleteSimple).not.toHaveBeenCalled();
			expect(result.llmDiagnosis).toBeUndefined();
		});
	});
});
