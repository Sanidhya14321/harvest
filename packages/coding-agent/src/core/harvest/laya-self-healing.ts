/**
 * Bounded Self-Healing and Diagnostic Framework for Harvest + Laya.
 *
 * Implements a strict, finite table of known failure modes:
 * 1. Wrong PyTorch wheel installed (CPU wheel on GPU host or vice-versa)
 * 2. Windows HuggingFace cache symlink restriction (WinError 1314)
 * 3. Port 8177 occupied by an unrecognized process (preserves foreign process, offers alternate)
 * 4. Model download failure / incomplete checkpoint (purges corrupt snapshot, retries once)
 * 5. Router accidental invocation (enforces single-checkpoint contract)
 * 6. Insufficient disk space (<2.0 GB free) before download starts
 * 7. Stale / mismatched calibration file (triggers automatic recalibration)
 * 8. Python missing on host (provides specific OS-tailored instructions)
 *
 * An unrecognized failure NEVER triggers an open-ended guess; instead it emits
 * a comprehensive diagnostic bundle to ~/.harvest/agent/logs/laya-setup.log and halts.
 */

import * as fs from "node:fs/promises";
import * as net from "node:net";
import * as os from "node:os";
import * as path from "node:path";
import { getAgentDir, logger } from "@harvest/pi-utils";

export interface FailureModeDefinition {
	readonly id: string;
	readonly signature: string;
	readonly description: string;
	readonly detection: string;
	readonly remediation: string;
	readonly logging: string;
}

export const KNOWN_FAILURE_MODES: readonly FailureModeDefinition[] = [
	{
		id: "1",
		signature: "WRONG_TORCH_WHEEL",
		description: "Wrong PyTorch wheel installed (CPU wheel on host with active NVIDIA GPU)",
		detection: "Host NVIDIA driver detected (nvcuda.dll / libcuda.so / nvidia-smi) while torch.cuda.is_available() reports False",
		remediation: "Reinstall PyTorch using CUDA 12.4 index (https://download.pytorch.org/whl/cu124)",
		logging: "Logs detected issue, before/after torch.cuda.is_available() state to laya-setup.log",
	},
	{
		id: "2",
		signature: "WINDOWS_HF_SYMLINK_RESTRICTION",
		description: "Windows HuggingFace cache symlink restriction (WinError 1314)",
		detection: "Error matching WinError 1314 or 'A required privilege is not held by the client' during model download/cache",
		remediation: "Set HF_HUB_DISABLE_SYMLINKS=1 and HF_HUB_DISABLE_SYMLINKS_WARNING=1 with _safe_symlink file-copy fallback",
		logging: "Logs symlink restriction detection and environment variable override to laya-setup.log",
	},
	{
		id: "3",
		signature: "PORT_CONFLICT",
		description: "Port 8177 already in use on sidecar startup",
		detection: "TCP bind failure or EADDRINUSE on port 8177",
		remediation: "Probe /health: reuse existing Harvest sidecar if healthy; preserve foreign process and allocate alternate port (8178-8185)",
		logging: "Logs whether port was reused or redirected to alternate port without killing foreign processes to laya-setup.log",
	},
	{
		id: "4",
		signature: "CORRUPTED_CHECKPOINT",
		description: "Model download failure / incomplete checkpoint",
		detection: "Cached model.safetensors < 800MB or corrupt header on safe_open probe",
		remediation: "Purge corrupt snapshot from HuggingFace cache and retry download once (halt on second failure)",
		logging: "Logs corrupted checkpoint signature, purge event, and retry outcome to laya-setup.log",
	},
	{
		id: "5",
		signature: "ROUTER_ACCIDENTAL_INVOCATION",
		description: "Router accidentally invoked, triggering secondary checkpoint download",
		detection: "Structural audit of sidecar script checking for laya.Router or secondary checkpoint in cache",
		remediation: "Enforce single-checkpoint contract (laya.load('convaiinnovations/laya-typed-decisions'))",
		logging: "Logs architecture validation outcome to laya-setup.log",
	},
	{
		id: "6",
		signature: "INSUFFICIENT_DISK_SPACE",
		description: "Insufficient disk space for model weights",
		detection: "Pre-flight statfs check verifies < 2,000 MB available on target cache volume before download starts",
		remediation: "Halt download before starting, preventing partial write corruption",
		logging: "Logs available vs required MB to laya-setup.log",
	},
	{
		id: "7",
		signature: "STALE_CALIBRATION_SIGNATURE",
		description: "Stale / mismatched calibration file from previous hardware or checkpoint",
		detection: "Hardware signature in laya-calibration.json does not match detected hardware signature",
		remediation: "Trigger automatic recalibration to derive fresh latency thresholds",
		logging: "Logs signature mismatch and recalibration event to laya-setup.log",
	},
	{
		id: "8",
		signature: "MISSING_PYTHON",
		description: "Python 3.9+ runtime not present on host system",
		detection: "testPythonExecutable fails across PATH and well-known installation directories",
		remediation: "Autonomous bootstrap via system package manager (winget/brew) or clear OS-specific commands",
		logging: "Logs missing runtime and OS-tailored resolution instructions to laya-setup.log",
	},
];

export interface HealingActionRecord {
	readonly timestamp: string;
	readonly signature: string;
	readonly description: string;
	readonly detectedIssue: string;
	readonly actionTaken: string;
	readonly result: "resolved" | "unresolved" | "escalated" | "prevented";
	readonly details?: Record<string, unknown>;
}

export interface DiagnosticBundle {
	readonly timestamp: string;
	readonly error: string;
	readonly platform: string;
	readonly arch: string;
	readonly osRelease: string;
	readonly bunVersion: string;
	readonly pythonPath?: string;
	readonly pythonVersion?: string;
	readonly torchVersion?: string;
	readonly torchCuda?: string | null;
	readonly hostHasNvidia: boolean;
	readonly diskSpaceMbAvailable?: number;
	readonly activePort: number;
	readonly recentLogTail: readonly string[];
}

/**
 * Dedicated logger for Laya setup and self-healing actions.
 * Writes human-readable, append-only logs to `~/.harvest/agent/logs/laya-setup.log`.
 */
export class LayaSetupLogger {
	readonly logFilePath: string;
	#recentLines: string[] = [];

	constructor(agentDir: string = getAgentDir()) {
		this.logFilePath = path.join(agentDir, "logs", "laya-setup.log");
	}

	async log(message: string, details?: Record<string, unknown>): Promise<void> {
		const timestamp = new Date().toISOString();
		const line = details
			? `[${timestamp}] ${message} | ${JSON.stringify(details)}`
			: `[${timestamp}] ${message}`;

		this.#recentLines.push(line);
		if (this.#recentLines.length > 50) {
			this.#recentLines.shift();
		}

		logger.debug(`[LayaSetup] ${message}`, details);

		try {
			await fs.mkdir(path.dirname(this.logFilePath), { recursive: true });
			await fs.appendFile(this.logFilePath, `${line}\n`, "utf8");
		} catch {
			// Non-fatal logging error
		}
	}

	async recordHealing(action: HealingActionRecord): Promise<void> {
		await this.log(`[SELF-HEALING] [${action.signature}] ${action.description}`, {
			detected: action.detectedIssue,
			actionTaken: action.actionTaken,
			result: action.result,
			...action.details,
		});
	}

	getRecentLines(): readonly string[] {
		return [...this.#recentLines];
	}
}

/**
 * Check available disk space on the volume hosting targetDir.
 * Returns available space in Megabytes.
 */
export async function getAvailableDiskSpaceMb(targetDir: string): Promise<number> {
	try {
		await fs.mkdir(targetDir, { recursive: true });
		const stat = await fs.statfs(targetDir);
		const bytesFree = Number(stat.bavail) * Number(stat.bsize);
		return Math.round(bytesFree / (1024 * 1024));
	} catch {
		// Unknown free space must not be treated as evidence that a download fits.
		return -1;
	}
}

/**
 * Failure Mode 6: Pre-flight disk space check.
 * Requires at least 2,000 MB free for the 842MB weights + unpacked files.
 */
export async function checkAvailableDiskSpace(
	targetDir: string,
	requiredMb = 2000,
	setupLogger?: LayaSetupLogger,
): Promise<{ ok: boolean; availableMb: number; requiredMb: number; error?: string }> {
	const availableMb = await getAvailableDiskSpaceMb(targetDir);
	if (availableMb < 0) {
		return {
			ok: false,
			availableMb,
			requiredMb,
			error: `Could not determine free disk space in ${targetDir}; model download was not started.`,
		};
	}
	if (availableMb < requiredMb) {
		const error = `Insufficient disk space: ${availableMb}MB available in ${targetDir}, but at least ${requiredMb}MB is required for model weights.`;
		await setupLogger?.recordHealing({
			timestamp: new Date().toISOString(),
			signature: "INSUFFICIENT_DISK_SPACE",
			description: "Pre-flight disk space check failed",
			detectedIssue: `Free space ${availableMb}MB is below required ${requiredMb}MB`,
			actionTaken: "Halted download before partial writes corrupted cache",
			result: "prevented",
			details: { availableMb, requiredMb, targetDir },
		});
		return { ok: false, availableMb, requiredMb, error };
	}
	return { ok: true, availableMb, requiredMb };
}

/**
 * Failure Mode 8: Missing Python instructions generator.
 */
export function getMissingPythonInstructions(): { os: string; command: string; instructions: string } {
	if (process.platform === "win32") {
		return {
			os: "Windows",
			command: "winget install Python.Python.3.11 --scope user",
			instructions: "Run 'winget install Python.Python.3.11 --scope user' in PowerShell, or download Python 3.11 from https://www.python.org/downloads/ (check 'Add python.exe to PATH').",
		};
	}
	if (process.platform === "darwin") {
		return {
			os: "macOS",
			command: "brew install python@3.11",
			instructions: "Run 'brew install python@3.11' via Homebrew, or install Python 3.11+ from https://www.python.org/downloads/.",
		};
	}
	return {
		os: "Linux",
		command: "sudo apt install python3 python3-venv python3-pip",
		instructions: "Install Python 3.9+ via your package manager (e.g. 'sudo apt install python3 python3-venv python3-pip' on Debian/Ubuntu, or 'sudo dnf install python3 python3-pip' on Fedora).",
	};
}

/**
 * Failure Mode 1: Wrong PyTorch wheel detection and remediation.
 * Checks whether host has NVIDIA GPU drivers but Python has a CPU-only PyTorch wheel installed.
 */
export async function verifyAndRepairTorchWheel(
	pythonPath: string,
	hostHasNvidia: boolean,
	setupLogger?: LayaSetupLogger,
	onProgress?: (msg: string) => void,
): Promise<{ repaired: boolean; error?: string; cudaAvailable: boolean }> {
	const probeCode = `
import sys
try:
    import torch
    print(f"{torch.__version__}|{torch.cuda.is_available()}|{getattr(torch.version, 'cuda', None)}")
except Exception as e:
    print(f"ERROR:{e}")
`;

	let probeProc = Bun.spawn([pythonPath, "-c", probeCode], { stdout: "pipe", stderr: "pipe" });
	let stdout = (await new Response(probeProc.stdout).text()).trim();
	if (stdout.startsWith("ERROR:") || !stdout.includes("|")) {
		return { repaired: false, cudaAvailable: false };
	}

	const [torchVer, cudaAvailStr, torchCuda] = stdout.split("|");
	const cudaAvailable = cudaAvailStr === "True";

	// Signature: Host has NVIDIA GPU, but PyTorch was installed without CUDA support
	if (hostHasNvidia && !cudaAvailable) {
		onProgress?.("Detected NVIDIA GPU on host, but installed PyTorch is CPU-only. Remediating with CUDA 12.4 wheel...");
		await setupLogger?.recordHealing({
			timestamp: new Date().toISOString(),
			signature: "WRONG_TORCH_WHEEL",
			description: "CPU PyTorch wheel detected on host with NVIDIA GPU",
			detectedIssue: `hostHasNvidia=true, torch.cuda.is_available()=false, torch=${torchVer}, torch.version.cuda=${torchCuda}`,
			actionTaken: "Reinstalling PyTorch with CUDA 12.4 index URL (https://download.pytorch.org/whl/cu124)",
			result: "escalated",
		});

		try {
			const reinstallProc = Bun.spawn([
				pythonPath,
				"-m",
				"pip",
				"install",
				"--upgrade",
				"--force-reinstall",
				"--no-input",
				"--prefer-binary",
				"--extra-index-url",
				"https://download.pytorch.org/whl/cu124",
				"torch",
			], { stdout: "pipe", stderr: "pipe" });

			const code = await reinstallProc.exited;
			if (code === 0) {
				probeProc = Bun.spawn([pythonPath, "-c", probeCode], { stdout: "pipe", stderr: "pipe" });
				stdout = (await new Response(probeProc.stdout).text()).trim();
				const [, newCudaStr] = stdout.split("|");
				const newCuda = newCudaStr === "True";

				await setupLogger?.recordHealing({
					timestamp: new Date().toISOString(),
					signature: "WRONG_TORCH_WHEEL",
					description: "Reinstalled CUDA PyTorch wheel",
					detectedIssue: "Prior wheel lacked CUDA bindings",
					actionTaken: "Replaced with cu124 wheel",
					result: newCuda ? "resolved" : "unresolved",
					details: { cudaNowAvailable: newCuda },
				});
				return { repaired: true, cudaAvailable: newCuda };
			}
		} catch (err) {
			return { repaired: false, cudaAvailable: false, error: String(err) };
		}
	}

	return { repaired: false, cudaAvailable };
}

/**
 * Failure Mode 4: Model download corruption detection and bounded single retry.
 * Probes for partial/corrupt safetensors weights (<800MB or unreadable).
 */
export async function checkAndRemediateCorruptedModel(
	pythonPath: string,
	setupLogger?: LayaSetupLogger,
	onProgress?: (msg: string) => void,
): Promise<{ corrupted: boolean; remediated: boolean; error?: string }> {
	const verifyCode = `
import os, sys
from pathlib import Path
hf_home = os.environ.get("HF_HOME")
p = Path(hf_home) / "hub" / "models--convaiinnovations--laya-typed-decisions" if hf_home else Path.home() / ".cache" / "huggingface" / "hub" / "models--convaiinnovations--laya-typed-decisions"
if not p.exists():
    print("NOT_CACHED")
    sys.exit(0)

safetensors = list(p.glob("**/model.safetensors"))
if not safetensors:
    print("NO_SAFETENSORS")
    sys.exit(0)

sf = safetensors[0]
if sf.stat().st_size < 800_000_000:
    print("CORRUPTED_SIZE")
    sys.exit(1)

try:
    from safetensors import safe_open
    with safe_open(str(sf), framework="pt") as f:
        keys = f.keys()
        if len(keys) < 10:
            print("CORRUPTED_HEADER")
            sys.exit(1)
    print("VALID")
    sys.exit(0)
except Exception as e:
    print(f"CORRUPTED_READ:{e}")
    sys.exit(1)
`;

	const proc = Bun.spawn([pythonPath, "-c", verifyCode], { stdout: "pipe", stderr: "pipe" });
	const exitCode = await proc.exited;
	const out = (await new Response(proc.stdout).text()).trim();

	if (exitCode !== 0 || out.startsWith("CORRUPTED")) {
		onProgress?.(`Detected corrupt checkpoint (${out}); purging snapshot and retrying download once...`);
		await setupLogger?.recordHealing({
			timestamp: new Date().toISOString(),
			signature: "CORRUPTED_CHECKPOINT",
			description: "Incomplete or corrupt model.safetensors detected in cache",
			detectedIssue: `Integrity check failed: ${out}`,
			actionTaken: "Purging corrupt snapshot directory and retrying single clean download",
			result: "escalated",
		});

		try {
			// Find model directory to purge
			const purgeScript = `
import os, shutil
from pathlib import Path
hf_home = os.environ.get("HF_HOME")
p = Path(hf_home) / "hub" / "models--convaiinnovations--laya-typed-decisions" if hf_home else Path.home() / ".cache" / "huggingface" / "hub" / "models--convaiinnovations--laya-typed-decisions"
if p.exists():
    shutil.rmtree(str(p), ignore_errors=True)
    print("PURGED")
`;
			const purgeProc = Bun.spawn([pythonPath, "-c", purgeScript], { stdout: "pipe", stderr: "pipe" });
			await purgeProc.exited;
			return { corrupted: true, remediated: true };
		} catch (err) {
			return { corrupted: true, remediated: false, error: String(err) };
		}
	}

	return { corrupted: false, remediated: false };
}

/**
 * Assemble and emit structured diagnostic bundle when an unrecognized error occurs.
 */
export async function createDiagnosticBundle(
	error: Error | string,
	setupLogger: LayaSetupLogger,
	context: {
		pythonPath?: string;
		pythonVersion?: string;
		hostHasNvidia?: boolean;
		activePort?: number;
		targetDir?: string;
	} = {},
): Promise<{ bundle: DiagnosticBundle; bundlePath: string }> {
	const errString = error instanceof Error ? error.message : String(error);
	let diskMb: number | undefined;
	if (context.targetDir) {
		const available = await getAvailableDiskSpaceMb(context.targetDir);
		if (available >= 0) diskMb = available;
	}

	const bundle: DiagnosticBundle = {
		timestamp: new Date().toISOString(),
		error: errString,
		platform: process.platform,
		arch: process.arch,
		osRelease: os.release(),
		bunVersion: Bun.version,
		pythonPath: context.pythonPath,
		pythonVersion: context.pythonVersion,
		hostHasNvidia: context.hostHasNvidia ?? false,
		diskSpaceMbAvailable: diskMb,
		activePort: context.activePort ?? 8177,
		recentLogTail: setupLogger.getRecentLines(),
	};

	const bundlePath = path.join(path.dirname(setupLogger.logFilePath), `laya-diagnostic-${Date.now()}.json`);
	try {
		await fs.mkdir(path.dirname(bundlePath), { recursive: true });
		await fs.writeFile(bundlePath, JSON.stringify(bundle, null, 2), "utf8");
	} catch {
		// Ignore disk error
	}

	await setupLogger.log(`[UNRECOGNIZED_FAILURE] Diagnostic bundle generated at ${bundlePath}`, {
		error: errString,
	});

	return { bundle, bundlePath };
}

/**
 * Emit structured diagnostic bundle when an unrecognized error occurs.
 */
export async function emitDiagnosticBundle(
	error: Error | string,
	setupLogger: LayaSetupLogger,
	context: {
		pythonPath?: string;
		pythonVersion?: string;
		hostHasNvidia?: boolean;
		activePort?: number;
		targetDir?: string;
	} = {},
): Promise<string> {
	const { bundlePath } = await createDiagnosticBundle(error, setupLogger, context);
	return bundlePath;
}

/**
 * Failure Mode 2: Detect Windows HuggingFace cache symlink privilege failure (WinError 1314).
 */
export function isSymlinkPrivilegeError(err: unknown): boolean {
	const str = String(err);
	return (
		str.includes("WinError 1314") ||
		str.includes("A required privilege is not held by the client") ||
		str.includes("cannot create symlink") ||
		str.includes("symbolic link privilege")
	);
}

/**
 * Failure Mode 2: Remediate Windows HuggingFace cache symlink restriction.
 * Sets HF_HUB_DISABLE_SYMLINKS=1 and HF_HUB_DISABLE_SYMLINKS_WARNING=1 with copy fallback.
 */
export async function applyWindowsSymlinkRemediation(
	setupLogger?: LayaSetupLogger,
	onProgress?: (msg: string) => void,
): Promise<{ env: Record<string, string>; applied: boolean }> {
	onProgress?.("Applying Windows symlink bypass (HF_HUB_DISABLE_SYMLINKS=1 with copy fallback)...");
	await setupLogger?.recordHealing({
		timestamp: new Date().toISOString(),
		signature: "WINDOWS_HF_SYMLINK_RESTRICTION",
		description: "Windows HuggingFace cache symlink restriction (WinError 1314)",
		detectedIssue: "Windows user lacks SeCreateSymbolicLinkPrivilege without Developer Mode",
		actionTaken: "Set HF_HUB_DISABLE_SYMLINKS=1 and HF_HUB_DISABLE_SYMLINKS_WARNING=1 with safe copy fallback",
		result: "resolved",
	});

	return {
		env: {
			HF_HUB_DISABLE_SYMLINKS: "1",
			HF_HUB_DISABLE_SYMLINKS_WARNING: "1",
		},
		applied: true,
	};
}

/**
 * Check whether a TCP port is in use on 127.0.0.1.
 */
export async function isPortInUse(port: number): Promise<boolean> {
	return new Promise(resolve => {
		const server = net.createServer();
		server.once("error", (err: NodeJS.ErrnoException) => {
			if (err.code === "EADDRINUSE") {
				resolve(true);
			} else {
				resolve(false);
			}
		});
		server.once("listening", () => {
			server.close(() => resolve(false));
		});
		server.listen(port, "127.0.0.1");
	});
}

/**
 * Failure Mode 3: Non-destructive port conflict resolution.
 * If preferred port (8177) is occupied:
 * - If healthy Laya sidecar: reuse existing process without restarting.
 * - If foreign/unrecognized process: PRESERVE foreign process (never taskkill/kill -9)
 *   and allocate alternate port in range 8178-8185.
 */
export async function resolvePortConflict(
	preferredPort = 8177,
	preferredBaseUrl = "http://127.0.0.1:8177",
	options: {
		setupLogger?: LayaSetupLogger;
		onProgress?: (msg: string) => void;
		probeSidecarHealth?: (baseUrl: string) => Promise<boolean>;
	} = {},
): Promise<{
	port: number;
	baseUrl: string;
	reusedExistingSidecar: boolean;
	foreignProcessDetected: boolean;
	error?: string;
}> {
	const inUse = await isPortInUse(preferredPort);
	if (!inUse) {
		return {
			port: preferredPort,
			baseUrl: `http://127.0.0.1:${preferredPort}`,
			reusedExistingSidecar: false,
			foreignProcessDetected: false,
		};
	}

	const probeHealth = options.probeSidecarHealth ?? (async (url: string) => {
		try {
			const controller = new AbortController();
			const timeout = setTimeout(() => controller.abort(), 800);
			const res = await fetch(`${url}/health`, { signal: controller.signal });
			clearTimeout(timeout);
			if (!res.ok) return false;
			const data = (await res.json()) as { ready?: boolean; status?: string };
			return data.ready === true || data.status === "ok";
		} catch {
			return false;
		}
	});

	const isHarvestSidecar = await probeHealth(preferredBaseUrl);
	if (isHarvestSidecar) {
		options.onProgress?.(`Port ${preferredPort} is already hosting an active Harvest Laya sidecar; reusing existing process.`);
		await options.setupLogger?.recordHealing({
			timestamp: new Date().toISOString(),
			signature: "PORT_CONFLICT",
			description: `Port ${preferredPort} occupied by healthy Laya sidecar`,
			detectedIssue: `Port ${preferredPort} in use; /health responded successfully`,
			actionTaken: "Reusing active Laya sidecar without restarting or rebinding",
			result: "resolved",
			details: { port: preferredPort, baseUrl: preferredBaseUrl },
		});
		return {
			port: preferredPort,
			baseUrl: preferredBaseUrl,
			reusedExistingSidecar: true,
			foreignProcessDetected: false,
		};
	}

	// Port is occupied by an unrecognized / foreign process: PRESERVE foreign process
	options.onProgress?.(`Port ${preferredPort} is occupied by an unrecognized process. Preserving foreign process and allocating alternate port (8178-8185)...`);
	await options.setupLogger?.recordHealing({
		timestamp: new Date().toISOString(),
		signature: "PORT_CONFLICT",
		description: `Port ${preferredPort} occupied by unrecognized non-Laya process`,
		detectedIssue: `Port ${preferredPort} in use but /health probe failed; foreign process detected`,
		actionTaken: "Preserved foreign process without termination; searching alternate port range 8178-8185",
		result: "resolved",
		details: { occupiedPort: preferredPort },
	});

	const startCandidate = preferredPort === 8177 ? 8178 : preferredPort + 1;
	const endCandidate = preferredPort === 8177 ? 8185 : preferredPort + 8;
	for (let candidatePort = startCandidate; candidatePort <= endCandidate; candidatePort++) {
		const candidateInUse = await isPortInUse(candidatePort);
		if (!candidateInUse) {
			const candidateBaseUrl = `http://127.0.0.1:${candidatePort}`;
			options.onProgress?.(`Allocated alternate free port ${candidatePort} for Laya sidecar.`);
			await options.setupLogger?.log(`Allocated alternate port ${candidatePort} for Laya sidecar`);
			return {
				port: candidatePort,
				baseUrl: candidateBaseUrl,
				reusedExistingSidecar: false,
				foreignProcessDetected: true,
			};
		}
	}

	const error = `Port ${preferredPort} and all alternate fallback ports (${startCandidate}-${endCandidate}) are in use. Please free a port or configure 'laya.url' manually.`;
	await options.setupLogger?.recordHealing({
		timestamp: new Date().toISOString(),
		signature: "PORT_CONFLICT",
		description: "All candidate ports in range 8177-8185 occupied",
		detectedIssue: "No free port found in range 8177-8185",
		actionTaken: "Halted startup to avoid clobbering foreign processes",
		result: "escalated",
	});
	return {
		port: preferredPort,
		baseUrl: preferredBaseUrl,
		reusedExistingSidecar: false,
		foreignProcessDetected: true,
		error,
	};
}

/**
 * Failure Mode 5: Router accidental invocation prevention.
 * Structurally asserts that decision-sidecar/server.py loads the single checkpoint
 * and never imports or invokes laya.Router.
 */
export async function assertSingleCheckpointPolicy(
	sidecarDir: string,
	setupLogger?: LayaSetupLogger,
): Promise<{ ok: boolean; violation?: string }> {
	const serverPath = path.join(sidecarDir, "server.py");
	try {
		const text = await Bun.file(serverPath).text();
		if (text.includes("Router(") || text.includes("from laya.router import") || text.includes("laya.Router")) {
			const violation = "Structural violation: decision-sidecar/server.py imports or constructs Router, risking dual-checkpoint download.";
			await setupLogger?.recordHealing({
				timestamp: new Date().toISOString(),
				signature: "ROUTER_ACCIDENTAL_INVOCATION",
				description: "Router import detected in sidecar server script",
				detectedIssue: violation,
				actionTaken: "Flagged architecture violation before invocation",
				result: "escalated",
			});
			return { ok: false, violation };
		}
	} catch {
		// If server.py not found yet, skip inspection
	}
	return { ok: true };
}

/**
 * Failure Mode 7: Stale / mismatched calibration file detection.
 * If cached calibration signature does not match current hardware signature,
 * automatically triggers recalibration rather than silently using stale numbers.
 */
export async function checkCalibrationSignatureMismatch(
	cached: { hardware?: { signature?: string } } | null,
	currentSignature: string,
	setupLogger?: LayaSetupLogger,
): Promise<{ isStale: boolean; reason?: string }> {
	if (!cached || !cached.hardware || !cached.hardware.signature) {
		return { isStale: false };
	}
	if (cached.hardware.signature !== currentSignature) {
		const reason = `Cached calibration signature (${cached.hardware.signature}) does not match current hardware signature (${currentSignature})`;
		await setupLogger?.recordHealing({
			timestamp: new Date().toISOString(),
			signature: "STALE_CALIBRATION_SIGNATURE",
			description: "Hardware signature mismatch detected in calibration file",
			detectedIssue: reason,
			actionTaken: "Triggering automatic recalibration rather than using stale numbers",
			result: "resolved",
			details: { cachedSignature: cached.hardware.signature, currentSignature },
		});
		return { isStale: true, reason };
	}
	return { isStale: false };
}

/**
 * End-to-end smoke test: hit /health, then a trivial /v1/decide call with a throwaway question.
 */
export async function runLayaSmokeTest(
	client: { baseUrl: string; isHealthy: () => Promise<boolean>; decide: (...args: any[]) => Promise<any> },
	setupLogger?: LayaSetupLogger,
): Promise<{
	success: boolean;
	healthOk: boolean;
	decideOk: boolean;
	latencyMs: number;
	error?: string;
}> {
	const startTime = Date.now();
	const healthOk = await client.isHealthy();
	if (!healthOk) {
		const err = `Smoke test failed: /health is not responding at ${client.baseUrl}`;
		await setupLogger?.log(`[SMOKE_TEST] FAILED: /health offline at ${client.baseUrl}`);
		return { success: false, healthOk: false, decideOk: false, latencyMs: Date.now() - startTime, error: err };
	}

	try {
		const decideRes = await client.decide(
			"Smoke test verification for Harvest decision engine.",
			{
				ping: {
					type: "noul",
					instructions: "Is this a system check?",
				},
			},
			{ callSite: "smoke_test", timeoutMs: 15_000 },
		);

		const answerObj = decideRes.data ?? (decideRes as any).answers ?? (decideRes as any).allAnswers;
		const decideOk = decideRes.success === true && Boolean(answerObj?.ping);
		const latencyMs = Date.now() - startTime;

		if (!decideOk) {
			const err = `Smoke test failed: /v1/decide returned error: ${decideRes.fallbackReason ?? "invalid answer structure"}`;
			await setupLogger?.log(`[SMOKE_TEST] FAILED: ${err}`);
			return { success: false, healthOk: true, decideOk: false, latencyMs, error: err };
		}

		await setupLogger?.log(`[SMOKE_TEST] PASSED: /health OK, /v1/decide OK in ${latencyMs}ms at ${client.baseUrl}`);
		return { success: true, healthOk: true, decideOk: true, latencyMs };
	} catch (err) {
		const latencyMs = Date.now() - startTime;
		const errorMsg = `Smoke test failed with exception: ${String(err)}`;
		await setupLogger?.log(`[SMOKE_TEST] FAILED: ${errorMsg}`);
		return { success: false, healthOk: true, decideOk: false, latencyMs, error: errorMsg };
	}
}

export * from "./laya-llm-diagnosis";

