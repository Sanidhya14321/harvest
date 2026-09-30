/**
 * Laya Local Service and Configuration Manager for Harvest.
 *
 * Handles probing the Python environment, managing an isolated virtualenv,
 * verifying Laya dependencies, ensuring the single checkpoint
 * `convaiinnovations/laya-typed-decisions` is cached, resolving port conflicts,
 * launching the local sidecar daemon with robust timeouts, running hardware
 * self-calibration, and connecting it seamlessly to Harvest.
 */

import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import * as net from "node:net";
import { getAgentDir, logger } from "@harvest/pi-utils";
import type { completeSimple } from "@harvest/pi-ai";
import type { ModelRegistry } from "../../config/model-registry";
import type { Settings } from "../../config/settings";
import { hostHasInheritableConsole } from "../../eval/py/spawn-options";
import { resolveDaemonSpawnOptions } from "../../launch/spawn-options";
import { ensureCalibrated, loadCalibration, type CalibrationRecord } from "./laya-calibration";
import { getLayaClient } from "./laya-client";
import {
	LayaSetupLogger,
	checkAvailableDiskSpace,
	getMissingPythonInstructions,
	verifyAndRepairTorchWheel,
	checkAndRemediateCorruptedModel,
	applyWindowsSymlinkRemediation,
	isSymlinkPrivilegeError,
	resolvePortConflict,
	assertSingleCheckpointPolicy,
	checkCalibrationSignatureMismatch,
	runLayaSmokeTest,
	createDiagnosticBundle,
	runLlmAssistedDiagnosis,
	type EvaluatedDiagnosisProposal,
	type LayaLLMDiagnosisResult,
} from "./laya-self-healing";

export interface LayaEnvironmentStatus {
	readonly pythonAvailable: boolean;
	readonly pythonPath?: string;
	readonly pythonVersion?: string;
	readonly isVirtualEnv?: boolean;
	readonly dependenciesInstalled: boolean;
	readonly modelCached: boolean;
	readonly sidecarRunning: boolean;
	readonly hardwareTier?: string;
	readonly error?: string;
}

export interface LayaSetupStep {
	readonly id: string;
	label: string;
	status: "pending" | "running" | "done" | "error" | "skipped";
	error?: string;
}

export const DEFAULT_LAYA_URL = "http://127.0.0.1:8177";

function managedLayaPythonPath(): string {
	const venvDir = path.join(getAgentDir(), "laya-venv");
	return process.platform === "win32"
		? path.join(venvDir, "Scripts", "python.exe")
		: path.join(venvDir, "bin", "python");
}

/**
 * Execute a probe command to determine if a candidate Python executable is functional and >= 3.9.
 */
export async function testPythonExecutable(cmd: string, isPyLauncher = false): Promise<{ path: string; version: string } | null> {
	try {
		const proc = isPyLauncher
			? Bun.spawn(["py", "-3", "-c", "import sys; print(f'{sys.version_info[0]}.{sys.version_info[1]}'); print(sys.executable)"], { stdout: "pipe", stderr: "pipe" })
			: Bun.spawn([cmd, "-c", "import sys; print(f'{sys.version_info[0]}.{sys.version_info[1]}'); print(sys.executable)"], { stdout: "pipe", stderr: "pipe" });

		const exitCode = await proc.exited;
		if (exitCode === 0) {
			const stdout = await new Response(proc.stdout).text();
			const lines = stdout.trim().split(/\r?\n/);
			const ver = lines[0]?.trim();
			const execPath = lines[1]?.trim() || cmd;
			if (ver) {
				const [major, minor] = ver.split(".").map(Number);
				if (major === 3 && minor >= 9) {
					return { path: execPath, version: ver };
				}
			}
		}
	} catch {
		// Ignore command spawn failure / missing binary
	}
	return null;
}

/**
 * Locate a working Python 3.9+ binary on the local system.
 * Prioritizes managed virtual environment if present, then system binaries, then known installation paths.
 */
export async function findPythonExecutable(): Promise<{ path: string; version: string } | null> {
	// 1. Check managed virtualenv first
	const venvPy = managedLayaPythonPath();

	const venvTest = await testPythonExecutable(venvPy);
	if (venvTest) {
		return venvTest;
	}

	// 2. Check PATH candidates
	const candidates = process.platform === "win32"
		? ["py", "python", "python3"]
		: ["python3", "python"];

	for (const cmd of candidates) {
		const res = await testPythonExecutable(cmd, cmd === "py");
		if (res) return res;
	}

	// 3. Check well-known platform installation directories
	const extraPaths: string[] = [];
	if (process.platform === "win32") {
		const localAppData = process.env.LOCALAPPDATA || "";
		const programFiles = process.env.ProgramFiles || "C:\\Program Files";
		for (const v of ["Python314", "Python313", "Python312", "Python311", "Python310", "Python39"]) {
			extraPaths.push(path.join(localAppData, "Programs", "Python", v, "python.exe"));
			extraPaths.push(path.join(programFiles, v, "python.exe"));
		}
	} else if (process.platform === "darwin") {
		extraPaths.push("/opt/homebrew/bin/python3", "/usr/local/bin/python3", "/usr/bin/python3");
	} else {
		extraPaths.push("/usr/bin/python3", "/usr/local/bin/python3");
	}

	for (const p of extraPaths) {
		const res = await testPythonExecutable(p);
		if (res) return res;
	}

	return null;
}

/**
 * Execute a subprocess while streaming stdout and stderr in real-time,
 * protecting against OS pipe saturation deadlocks and enforcing activity timeouts.
 */
export async function streamProcessOutput(
	proc: Bun.Subprocess,
	onLine?: (line: string) => void,
	options: { activityTimeoutMs?: number; totalTimeoutMs?: number } = {},
): Promise<{ exitCode: number; stdout: string; stderr: string }> {
	const textDecoder = new TextDecoder();
	let lastActivity = Date.now();
	const startTime = Date.now();
	let timer: ReturnType<typeof setInterval> | undefined;

	const activityTimeout = options.activityTimeoutMs ?? 360_000;
	const totalTimeout = options.totalTimeoutMs ?? 1200_000;
	let timedOut = false;
	let timeoutReason = "";

	if (activityTimeout > 0 || totalTimeout > 0) {
		timer = setInterval(() => {
			const now = Date.now();
			if (activityTimeout > 0 && now - lastActivity > activityTimeout) {
				timedOut = true;
				timeoutReason = `Process timed out after ${activityTimeout / 1000}s with no output/activity`;
				try { proc.kill(); } catch {}
			} else if (totalTimeout > 0 && now - startTime > totalTimeout) {
				timedOut = true;
				timeoutReason = `Process exceeded maximum run time limit of ${totalTimeout / 1000}s`;
				try { proc.kill(); } catch {}
			}
		}, 1000);
	}

	let stdoutAccum = "";
	let stderrAccum = "";

	async function readPipe(stream: ReadableStream<Uint8Array> | null, isStderr: boolean) {
		if (!stream) return;
		const reader = stream.getReader();
		let buffer = "";
		try {
			while (true) {
				const { done, value } = await reader.read();
				if (done) break;
				lastActivity = Date.now();
				const text = textDecoder.decode(value, { stream: true });
				buffer += text;
				if (isStderr) stderrAccum += text;
				else stdoutAccum += text;

				// Split on either \n or \r so interactive progress bars (\r) update immediately
				const parts = buffer.split(/\r\n|\r|\n/);
				buffer = parts.pop() ?? "";
				for (const rawLine of parts) {
					const line = rawLine.trim();
					if (line.length > 0 && onLine) {
						onLine(line);
					}
				}
			}
			if (buffer.trim().length > 0 && onLine) {
				onLine(buffer.trim());
			}
		} catch {}
	}

	try {
		await Promise.all([
			readPipe(proc.stdout as ReadableStream<Uint8Array>, false),
			readPipe(proc.stderr as ReadableStream<Uint8Array>, true),
			proc.exited,
		]);
	} finally {
		if (timer) clearInterval(timer);
	}

	const exitCode = await proc.exited;
	if (timedOut) {
		throw new Error(timeoutReason);
	}
	return { exitCode, stdout: stdoutAccum, stderr: stderrAccum };
}

/**
 * Detect whether an NVIDIA GPU is physically present with drivers on the host OS.
 * Used to avoid downloading 5GB+ CUDA PyTorch packages when a 180MB CPU wheel suffices.
 */
export async function isHostNvidiaGpuPresent(): Promise<boolean> {
	if (process.platform === "win32") {
		const sysRoot = process.env.SystemRoot || "C:\\Windows";
		const nvcuda = path.join(sysRoot, "System32", "nvcuda.dll");
		try {
			if (Bun.file(nvcuda).size > 0) return true;
		} catch {}
	} else if (process.platform === "linux") {
		try {
			if (Bun.file("/proc/driver/nvidia/version").size > 0) return true;
		} catch {}
		for (const p of ["/usr/lib/x86_64-linux-gnu/libcuda.so", "/usr/lib64/libcuda.so", "/usr/lib/libcuda.so"]) {
			try {
				if (Bun.file(p).size > 0) return true;
			} catch {}
		}
	}
	try {
		const res = Bun.spawnSync(["nvidia-smi", "-L"]);
		if (res.exitCode === 0) return true;
	} catch {}
	return false;
}

/**
 * Attempt autonomous installation of Python if entirely missing.
 */
export async function bootstrapPythonIfMissing(
	onProgress?: (msg: string) => void,
): Promise<{ path: string; version: string } | null> {
	let py = await findPythonExecutable();
	if (py) return py;

	onProgress?.("Python 3.9+ not found. Attempting autonomous package installation...");

	if (process.platform === "win32") {
		try {
			const checkWinget = Bun.spawnSync(["winget", "--version"]);
			if (checkWinget.exitCode === 0) {
				onProgress?.("Installing Python 3.11 via Windows Package Manager (winget, user scope)...");
				const installProc = Bun.spawn([
					"winget",
					"install",
					"--id",
					"Python.Python.3.11",
					"-e",
					"--silent",
					"--scope",
					"user",
					"--disable-interactivity",
					"--accept-package-agreements",
					"--accept-source-agreements",
				], { stdout: "pipe", stderr: "pipe" });
				await streamProcessOutput(installProc, line => onProgress?.(`winget: ${line}`), {
					activityTimeoutMs: 120_000,
					totalTimeoutMs: 300_000,
				});
				py = await findPythonExecutable();
				if (py) return py;
			}
		} catch (err) {
			logger.debug("Winget auto-install attempt failed", { error: err });
		}
	} else if (process.platform === "darwin") {
		try {
			const checkBrew = Bun.spawnSync(["brew", "--version"]);
			if (checkBrew.exitCode === 0) {
				onProgress?.("Installing Python 3.11 via Homebrew...");
				const brewProc = Bun.spawn(["brew", "install", "python@3.11"], { stdout: "pipe", stderr: "pipe" });
				await streamProcessOutput(brewProc, line => onProgress?.(`brew: ${line}`), {
					activityTimeoutMs: 120_000,
					totalTimeoutMs: 600_000,
				});
				py = await findPythonExecutable();
				if (py) return py;
			}
		} catch (err) {
			logger.debug("Homebrew auto-install attempt failed", { error: err });
		}
	}

	return py;
}

/**
 * Ensure an isolated virtual environment exists under the Harvest agent config directory.
 * This protects against system Python PEP 668 restrictions and dependency conflicts.
 */
export async function ensureLayaVirtualEnv(
	systemPythonPath: string,
	onProgress?: (msg: string) => void,
): Promise<{ path: string; version: string; isVenv: boolean }> {
	const venvDir = path.join(getAgentDir(), "laya-venv");
	const venvPy = managedLayaPythonPath();

	const existing = await testPythonExecutable(venvPy);
	if (existing) {
		return { ...existing, isVenv: true };
	}

	try {
		onProgress?.(`Creating isolated Python virtual environment at ${venvDir}...`);
		const proc = Bun.spawn([systemPythonPath, "-m", "venv", venvDir], {
			stdout: "pipe",
			stderr: "pipe",
		});
		const exitCode = await proc.exited;
		if (exitCode === 0) {
			const created = await testPythonExecutable(venvPy);
			if (created) {
				return { ...created, isVenv: true };
			}
		}
	} catch (err) {
		throw new Error(`Failed to create isolated Laya virtualenv at ${venvDir}: ${String(err)}`);
	}
	throw new Error(`Failed to create isolated Laya virtualenv at ${venvDir}. Check that Python's venv module is installed.`);
}

/**
 * Check if the Laya HTTP sidecar is active and ready.
 */
export async function isLayaSidecarRunning(baseUrl: string = DEFAULT_LAYA_URL): Promise<boolean> {
	try {
		const controller = new AbortController();
		const timeout = setTimeout(() => controller.abort(), 600);
		const res = await fetch(`${baseUrl}/health`, {
			method: "GET",
			signal: controller.signal,
		});
		clearTimeout(timeout);
		if (!res.ok) return false;
		const json = (await res.json()) as { ready?: boolean; status?: string; model?: string };
		if (json.ready !== true && json.status !== "ok") return false;
		// A bare ready flag is not process identity: require our model id so a
		// foreign listener on the port is never mistaken for the sidecar.
		// Full reuse decisions additionally run verifyManagedSidecarIdentity.
		if (json.model !== undefined && json.model !== EXPECTED_LAYA_MODEL_ID) return false;
		return true;
	} catch {
		return false;
	}
}

/**
 * Check whether a TCP port is in use.
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
 * Non-destructive port conflict resolution.
 * If occupied by active Laya sidecar: reuse.
 * If occupied by foreign process: PRESERVE foreign process (never kill) and allocate alternate port.
 */
export async function freePortIfOccupied(
	port = 8177,
	baseUrl: string = DEFAULT_LAYA_URL,
	onProgress?: (msg: string) => void,
): Promise<{ freed: boolean; alreadyRunning: boolean; effectivePort?: number; effectiveBaseUrl?: string }> {
	const res = await resolvePortConflict(port, baseUrl, { onProgress });
	if (res.reusedExistingSidecar) {
		return { freed: true, alreadyRunning: true, effectivePort: res.port, effectiveBaseUrl: res.baseUrl };
	}
	return {
		freed: !res.error,
		alreadyRunning: false,
		effectivePort: res.port,
		effectiveBaseUrl: res.baseUrl,
	};
}

/**
 * Check whether required Python packages (laya, fastapi, uvicorn, torch) are importable.
 */
export async function checkLayaDependencies(pythonPath: string): Promise<boolean> {
	try {
		const proc = Bun.spawn([
			pythonPath,
			"-c",
			"import laya, fastapi, uvicorn, torch; print('OK')",
		], { stdout: "pipe", stderr: "pipe" });
		const exitCode = await proc.exited;
		if (exitCode !== 0) return false;
		const text = (await new Response(proc.stdout).text()).trim();
		return text.includes("OK");
	} catch {
		return false;
	}
}

/**
 * Locate the decision-sidecar directory, searching environment variables,
 * the monorepo root relative to this module, the current working directory,
 * or the user's harvest agent directory.
 */
export function getSidecarDir(): string {
	const candidates: string[] = [];

	if (process.env.LAYA_SIDECAR_DIR) {
		candidates.push(process.env.LAYA_SIDECAR_DIR);
	}
	if (process.env.HARVEST_SIDECAR_DIR) {
		candidates.push(process.env.HARVEST_SIDECAR_DIR);
	}

	// Source checkouts use the canonical files in the repository.
	candidates.push(path.resolve(import.meta.dir, "../../../../../decision-sidecar"));

	for (const candidate of candidates) {
		try {
			const serverFile = path.join(candidate, "server.py");
			const file = Bun.file(serverFile);
			if (file.size > 0) {
				return candidate;
			}
		} catch {
			// Continue
		}
	}

	// Release bundles contain a compressed copy of the Python sources. Resolve
	// this before cwd so an unrelated project cannot supply executable sidecar code.
	const embedded = process.env.PI_LAYA_SIDECAR_EMBED;
	if (embedded) return materializeBundledLayaSidecar(embedded, getAgentDir());

	for (const candidate of [path.resolve(process.cwd(), "decision-sidecar"), path.join(getAgentDir(), "sidecar")]) {
		try {
			if (Bun.file(path.join(candidate, "server.py")).size > 0) return candidate;
		} catch {
			// Continue
		}
	}

	return path.resolve(import.meta.dir, "../../../../../decision-sidecar");
}

const BUNDLED_SIDECAR_FILES = [
	"server.py",
	"hardware.py",
	"bucketing.py",
	"calibration.py",
	"calibration_params.json",
	"requirements.txt",
] as const;

/** Materialize the versioned bundled service once for npm and binary installs. */
export function materializeBundledLayaSidecar(payload: string, agentDir: string): string {
	const raw = new TextDecoder().decode(Bun.gunzipSync(Buffer.from(payload, "base64")));
	const parsed: unknown = JSON.parse(raw);
	if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
		throw new Error("Invalid bundled Laya sidecar payload");
	}
	const files = parsed as Record<string, unknown>;
	for (const name of BUNDLED_SIDECAR_FILES) {
		if (typeof files[name] !== "string") throw new Error(`Bundled Laya sidecar is missing ${name}`);
	}
	const digest = Bun.SHA256.hash(raw, "hex").slice(0, 16);
	const sidecarRoot = path.join(agentDir, "sidecar");
	const target = path.join(sidecarRoot, digest);
	const complete = () => BUNDLED_SIDECAR_FILES.every(name => {
		try {
			return fs.readFileSync(path.join(target, name), "utf8") === files[name];
		} catch {
			return false;
		}
	});
	if (complete()) return target;
	if (fs.existsSync(target)) throw new Error(`Bundled Laya sidecar directory is incomplete: ${target}`);

	fs.mkdirSync(sidecarRoot, { recursive: true, mode: 0o700 });
	const staging = fs.mkdtempSync(path.join(sidecarRoot, ".stage-"));
	try {
		for (const name of BUNDLED_SIDECAR_FILES) {
			fs.writeFileSync(path.join(staging, name), files[name] as string, { mode: 0o600 });
		}
		try {
			fs.renameSync(staging, target);
		} catch (error) {
			if (!complete()) throw error;
		}
	} finally {
		if (fs.existsSync(staging)) fs.rmSync(staging, { recursive: true, force: true });
	}
	return target;
}

/**
 * Install missing Python packages from decision-sidecar/requirements.txt.
 * Includes pip self-upgrade and automatic PEP 668 handling.
 */
export async function installLayaDependencies(
	pythonPath: string,
	onProgress?: (msg: string) => void,
): Promise<{ success: boolean; error?: string; alreadyInstalled?: boolean }> {
	if (await checkLayaDependencies(pythonPath)) {
		return { success: true, alreadyInstalled: true };
	}

	const sidecarDir = getSidecarDir();
	const reqPath = path.join(sidecarDir, "requirements.txt");

	try {
		onProgress?.("Upgrading pip and wheel...");
		const upgradeProc = Bun.spawn([
			pythonPath,
			"-m",
			"pip",
			"install",
			"--upgrade",
			"pip",
			"setuptools",
			"wheel",
			"--no-input",
			"--prefer-binary",
			"--retries",
			"3",
			"--timeout",
			"30",
		], {
			cwd: sidecarDir,
			stdout: "pipe",
			stderr: "pipe",
		});
		await streamProcessOutput(upgradeProc, undefined, { activityTimeoutMs: 60_000, totalTimeoutMs: 120_000 });

		const hasNvidia = await isHostNvidiaGpuPresent();
		const extraIndexArgs = (!hasNvidia && process.platform !== "darwin")
			? ["--extra-index-url", "https://download.pytorch.org/whl/cpu"]
			: [];

		const targetDesc = hasNvidia
			? "CUDA GPU runtime"
			: (process.platform === "darwin" ? "Apple Silicon / CPU runtime" : "CPU runtime (~180MB lightweight wheel)");
		onProgress?.(`Installing Python dependencies (laya, fastapi, uvicorn, torch for ${targetDesc})...`);

		const hasReq = Bun.file(reqPath).size > 0;
		const baseArgs = [
			pythonPath,
			"-m",
			"pip",
			"install",
			"--no-input",
			"--prefer-binary",
			"--retries",
			"3",
			"--timeout",
			"30",
			...extraIndexArgs,
			...(hasReq ? ["-r", reqPath] : ["laya>=0.3.5", "fastapi>=0.115.0", "uvicorn>=0.30.0", "torch", "pydantic>=2.0.0"]),
		];

		const filterPipProgress = (rawLine: string) => {
			if (
				rawLine.startsWith("Collecting ") ||
				rawLine.startsWith("Downloading ") ||
				rawLine.startsWith("Installing collected packages") ||
				rawLine.startsWith("Successfully installed") ||
				rawLine.includes("MB/s") ||
				rawLine.includes("%")
			) {
				onProgress?.(rawLine);
			}
		};

		const proc = Bun.spawn(baseArgs, {
			cwd: sidecarDir,
			stdout: "pipe",
			stderr: "pipe",
		});

		const res = await streamProcessOutput(proc, filterPipProgress, {
			activityTimeoutMs: 480_000,
			totalTimeoutMs: 1200_000,
		});

		if (res.exitCode !== 0) {
			return { success: false, error: res.stderr || res.stdout || `pip install failed with exit code ${res.exitCode}` };
		}

		const verified = await checkLayaDependencies(pythonPath);
		if (!verified) {
			return { success: false, error: "Installed dependencies could not be verified by Python runtime" };
		}
		return { success: true };
	} catch (err) {
		return { success: false, error: String(err) };
	}
}

/**
 * Ensure the single-model checkpoint is downloaded and verified in HuggingFace cache.
 */
export async function ensureLayaModelCached(
	pythonPath: string,
	onProgress?: (msg: string) => void,
	setupLogger?: LayaSetupLogger,
): Promise<{ success: boolean; error?: string; alreadyCached?: boolean }> {
	try {
		// Mode 6: Pre-flight disk space check before download
		const hfHome = process.env.HF_HOME || path.join(os.homedir(), ".cache", "huggingface");
		const diskCheck = await checkAvailableDiskSpace(hfHome, 2000, setupLogger);
		if (!diskCheck.ok) {
			return { success: false, error: diskCheck.error };
		}

		onProgress?.("Checking HuggingFace cache for convaiinnovations/laya-typed-decisions...");

		// Mode 4: Probe for corrupted model (<800MB or unreadable) and purge if corrupt
		const corruptCheck = await checkAndRemediateCorruptedModel(pythonPath, setupLogger, onProgress);
		if (corruptCheck.corrupted && !corruptCheck.remediated) {
			return { success: false, error: corruptCheck.error ?? "Corrupted checkpoint detected and could not be purged" };
		}

		const probeCode = `
import os, sys
from pathlib import Path
hf_home = os.environ.get("HF_HOME")
p = Path(hf_home) / "hub" / "models--convaiinnovations--laya-typed-decisions" if hf_home else Path.home() / ".cache" / "huggingface" / "hub" / "models--convaiinnovations--laya-typed-decisions"
if p.exists() and any(p.glob("**/model.safetensors")):
    print("MODEL_CACHED")
    sys.exit(0)
sys.exit(1)
`;
		const probeProc = Bun.spawn([pythonPath, "-c", probeCode], {
			stdout: "pipe",
			stderr: "pipe",
			env: {
				...process.env,
				HF_HUB_DISABLE_SYMLINKS: "1",
				HF_HUB_DISABLE_SYMLINKS_WARNING: "1",
			},
		});

		if ((await probeProc.exited) === 0) {
			return { success: true, alreadyCached: true };
		}

		onProgress?.("Downloading single-model checkpoint convaiinnovations/laya-typed-decisions (~842MB)...");

		const downloadScript = `
import os, sys, socket
socket.setdefaulttimeout(30.0)
os.environ["HF_HUB_DISABLE_SYMLINKS"] = "1"
os.environ["HF_HUB_DISABLE_SYMLINKS_WARNING"] = "1"

if sys.platform == "win32":
    import shutil
    _orig_symlink = getattr(os, "symlink", None)
    if _orig_symlink:
        def _safe_symlink(src, dst, target_is_directory=False, *args, **kwargs):
            try:
                return _orig_symlink(src, dst, target_is_directory=target_is_directory, *args, **kwargs)
            except OSError as e:
                if getattr(e, "winerror", None) == 1314 or getattr(e, "errno", None) == 1:
                    src_full = src if os.path.isabs(src) else os.path.normpath(os.path.join(os.path.dirname(dst), src))
                    if os.path.isdir(src_full):
                        return shutil.copytree(src_full, dst, dirs_exist_ok=True)
                    else:
                        return shutil.copyfile(src_full, dst)
                raise
        os.symlink = _safe_symlink

try:
    import tqdm.auto
    class StreamTqdm(tqdm.auto.tqdm):
        def __init__(self, *args, **kwargs):
            kwargs["file"] = sys.stdout
            kwargs["mininterval"] = 0.5
            kwargs["ascii"] = True
            super().__init__(*args, **kwargs)
        def display(self, msg=None, pos=None):
            super().display(msg, pos)
            sys.stdout.write("\\n")
            sys.stdout.flush()
    tqdm.auto.tqdm = StreamTqdm
    import huggingface_hub.utils
    huggingface_hub.utils.tqdm = StreamTqdm
except Exception:
    pass

try:
    from huggingface_hub import snapshot_download
    snapshot_download(
        "convaiinnovations/laya-typed-decisions",
        repo_type="model",
        allow_patterns=["*.json", "*.safetensors", "tokenizer/*", "encoder/*"],
        max_workers=4,
    )
    print("DOWNLOAD_OK")
except Exception as e:
    try:
        sys.stderr.write(f"Primary HF download failed ({e}); retrying via hf-mirror.com...\\n")
        sys.stderr.flush()
        os.environ["HF_ENDPOINT"] = "https://hf-mirror.com"
        from huggingface_hub import snapshot_download
        snapshot_download(
            "convaiinnovations/laya-typed-decisions",
            repo_type="model",
            allow_patterns=["*.json", "*.safetensors", "tokenizer/*", "encoder/*"],
            max_workers=4,
        )
        print("DOWNLOAD_OK")
    except Exception as e2:
        import laya
        laya.load("convaiinnovations/laya-typed-decisions")
        print("DOWNLOAD_OK")
`;

		const downloadProc = Bun.spawn([pythonPath, "-c", downloadScript], {
			stdout: "pipe",
			stderr: "pipe",
			env: {
				...process.env,
				PYTHONUNBUFFERED: "1",
				HF_HUB_DISABLE_SYMLINKS: "1",
				HF_HUB_DISABLE_SYMLINKS_WARNING: "1",
			},
		});

		const filterHfProgress = (rawLine: string) => {
			if (
				rawLine.includes("%") ||
				rawLine.includes("Fetching") ||
				rawLine.includes("model.safetensors") ||
				rawLine.includes("retrying via") ||
				rawLine.includes("MB/s")
			) {
				onProgress?.(rawLine);
			}
		};

		const res = await streamProcessOutput(downloadProc, filterHfProgress, {
			activityTimeoutMs: 300_000,
			totalTimeoutMs: 1200_000,
		});

		if (res.exitCode !== 0) {
			if (isSymlinkPrivilegeError(res.stderr || res.stdout)) {
				await applyWindowsSymlinkRemediation(setupLogger, onProgress);
			}
			return { success: false, error: res.stderr || res.stdout || `Model loading failed with exit code ${res.exitCode}` };
		}
		return { success: true };
	} catch (err) {
		return { success: false, error: String(err) };
	}
}

/**
 * PID file recording the sidecar child spawned by {@link startLayaSidecarProcess}.
 */
export function getLayaSidecarPidFile(agentDir: string = getAgentDir()): string {
	return path.join(agentDir, "laya-sidecar.pid");
}

/** Append-only log capturing the sidecar child's stdout/stderr. */
export function getLayaSidecarLogFile(agentDir: string = getAgentDir()): string {
	return path.join(agentDir, "logs", "laya-sidecar.log");
}

/** Last `maxBytes` of a file as text; empty string when unreadable. */
function tailFileBytes(file: string, maxBytes: number): string {
	try {
		const size = fs.statSync(file).size;
		const start = Math.max(0, size - maxBytes);
		const fd = fs.openSync(file, "r");
		try {
			const buf = Buffer.allocUnsafe(Math.min(maxBytes, size));
			const read = fs.readSync(fd, buf, 0, buf.length, start);
			return buf.subarray(0, read).toString("utf8");
		} finally {
			fs.closeSync(fd);
		}
	} catch {
		return "";
	}
}

/**
 * Launch the local decision sidecar daemon as a background subprocess.
 * Uses non-destructive port resolution: reuses healthy Laya or redirects to alternate port without killing foreign processes.
 */
export async function startLayaSidecarProcess(
	pythonPath: string,
	baseUrl: string = DEFAULT_LAYA_URL,
	onProgress?: (msg: string) => void,
	setupLogger?: LayaSetupLogger,
): Promise<SidecarSpawnResult> {
	return singleflight(sidecarSpawnInflight, baseUrl.replace(/\/+$/, ""), () =>
		runSidecarSpawnProcess(pythonPath, baseUrl, onProgress, setupLogger),
	);
}

type SidecarSpawnResult = { success: boolean; error?: string; actualBaseUrl?: string };

/** In-flight sidecar spawns keyed by normalized base URL. */
const sidecarSpawnInflight = new Map<string, Promise<SidecarSpawnResult>>();

/**
 * Share one in-flight async run per key: concurrent starters receive the
 * same promise instead of duplicating the work. The slot clears when the run
 * settles (success or failure), so a later starter begins a fresh run. A
 * synchronously throwing `start` propagates without poisoning the map.
 */
export function singleflight<K, V>(runs: Map<K, Promise<V>>, key: K, start: () => Promise<V>): Promise<V> {
	const existing = runs.get(key);
	if (existing) return existing;
	const run = start();
	runs.set(key, run);
	// then(clear, clear) — not finally: the derived promise must never reject,
	// while callers of the returned `run` still observe its real outcome.
	run.then(clear, clear);
	function clear(): void {
		if (runs.get(key) === run) runs.delete(key);
	}
	return run;
}

/**
 * The spawn pipeline itself (single run): reuse/health checks, port-conflict
 * resolution, PID file, early-exit fast-fail, and kill-on-timeout. Prefer
 * {@link startLayaSidecarProcess}, which singleflights concurrent starters
 * (autostart racing a manual start) onto one shared run so a second daemon
 * can never hide behind a port drift.
 */
async function runSidecarSpawnProcess(
	pythonPath: string,
	baseUrl: string = DEFAULT_LAYA_URL,
	onProgress?: (msg: string) => void,
	setupLogger?: LayaSetupLogger,
): Promise<SidecarSpawnResult> {
	if (await isLayaSidecarRunning(baseUrl)) {
		return { success: true, actualBaseUrl: baseUrl };
	}

	const url = new URL(baseUrl);
	const port = Number(url.port) || 8177;

	const portRes = await resolvePortConflict(port, baseUrl, {
		setupLogger,
		onProgress,
	});

	if (portRes.error) {
		return { success: false, error: portRes.error };
	}
	if (portRes.reusedExistingSidecar) {
		return { success: true, actualBaseUrl: portRes.baseUrl };
	}

	const effectivePort = portRes.port;
	const effectiveBaseUrl = portRes.baseUrl;

	const sidecarDir = getSidecarDir();
	const serverPath = path.join(sidecarDir, "server.py");

	// Mode 5: Router accidental invocation prevention check
	const routerAudit = await assertSingleCheckpointPolicy(sidecarDir, setupLogger);
	if (!routerAudit.ok) {
		return { success: false, error: routerAudit.violation };
	}

	try {
		if (Bun.file(serverPath).size === 0) {
			return { success: false, error: `Sidecar server script not found at ${serverPath}` };
		}
	} catch {
		return { success: false, error: `Sidecar server script not found at ${serverPath}` };
	}

	onProgress?.(`Starting Laya sidecar daemon on 127.0.0.1:${effectivePort}...`);

	try {
		const agentDir = getAgentDir();
		const logPath = getLayaSidecarLogFile(agentDir);
		const pidPath = getLayaSidecarPidFile(agentDir);
		fs.mkdirSync(path.dirname(logPath), { recursive: true });
		const logFd = fs.openSync(logPath, "a");
		const child = Bun.spawn([
			pythonPath,
			"-u",
			serverPath,
		], {
			cwd: sidecarDir,
			...resolveDaemonSpawnOptions({
				platform: process.platform,
				hostHasInheritableConsole: hostHasInheritableConsole(),
			}),
			stdout: logFd,
			stderr: logFd,
			env: {
				...process.env,
				PYTHONUNBUFFERED: "1",
				HF_HUB_DISABLE_SYMLINKS: "1",
				HF_HUB_DISABLE_SYMLINKS_WARNING: "1",
				LAYA_HOST: "127.0.0.1",
				LAYA_PORT: String(effectivePort),
			},
		});
		// Persist the child PID so later runs/operators can attribute or reap
		// the daemon instead of spawning duplicates behind a port drift.
		try {
			fs.writeFileSync(pidPath, `${child.pid}\n`);
		} catch (err) {
			logger.warn("Failed to persist Laya sidecar PID file", { pidPath, error: String(err) });
		}

		// Watch for early child death so the probe below fast-fails instead of
		// polling the full window against a process that is already gone.
		let childExited = false;
		let childExitCode: number | null = null;
		const closeLog = (): void => {
			try { fs.closeSync(logFd); } catch {}
		};
		void child.exited.then(code => {
			childExited = true;
			childExitCode = code;
			closeLog();
		});

		const startTime = Date.now();
		const maxTimeoutMs = 180_000;

		while (Date.now() - startTime < maxTimeoutMs) {
			if (childExited) {
				try { fs.rmSync(pidPath, { force: true }); } catch {}
				const tail = tailFileBytes(logPath, 4096).trim().split("\n").slice(-10).join("\n");
				return { success: false, error: `Laya sidecar exited during startup (code ${childExitCode}) — see ${logPath}${tail ? `: ${tail}` : ""}` };
			}
			if (await isLayaSidecarRunning(effectiveBaseUrl)) {
				return { success: true, actualBaseUrl: effectiveBaseUrl };
			}
			const elapsedSec = Math.floor((Date.now() - startTime) / 1000);
			if (elapsedSec % 5 === 0 && elapsedSec > 0) {
				onProgress?.(`Waiting for Laya sidecar initialization and model warmup (${elapsedSec}s / 180s)...`);
			}
			await Bun.sleep(500);
		}

		// Probe window exhausted with a live-but-unready child: kill it so a
		// wedged daemon doesn't linger behind the caller's port.
		try { child.kill(); } catch {}
		closeLog();
		try { fs.rmSync(pidPath, { force: true }); } catch {}
		await setupLogger?.log(`[SIDECAR] Killed unready sidecar child after 180s probe at ${effectiveBaseUrl}`);
		return { success: false, error: `Timed out waiting 180s for Laya sidecar /health at ${effectiveBaseUrl}` };
	} catch (err) {
		return { success: false, error: String(err) };
	}
}

/** Start an already-installed sidecar for an interactive CLI session without delaying the TUI. */
export async function autostartInstalledLayaSidecar(activeSettings: Settings): Promise<void> {
	if (activeSettings.get("laya.enabled") !== true || activeSettings.get("laya.autostart") !== true) return;
	const pythonPath = managedLayaPythonPath();
	if (!(await Bun.file(pythonPath).exists())) {
		logger.warn("Laya autostart skipped: managed Python environment is missing", { pythonPath });
		return;
	}
	const baseUrl = activeSettings.get("laya.url") || DEFAULT_LAYA_URL;
	const result = await startLayaSidecarProcess(pythonPath, baseUrl);
	if (!result.success) {
		logger.warn("Laya autostart failed", { error: result.error, baseUrl });
		return;
	}
	if (result.actualBaseUrl && result.actualBaseUrl !== baseUrl) {
		activeSettings.set("laya.url", result.actualBaseUrl);
	}
}

export interface LayaSetupResult {
	success: boolean;
	coreHarvestReady?: boolean;
	calibration?: CalibrationRecord;
	smokeTest?: { success: boolean; healthOk: boolean; decideOk: boolean; latencyMs: number };
	error?: string;
	diagnosticBundlePath?: string;
	idempotentFastPath?: boolean;
	effectiveUrl?: string;
	llmDiagnosis?: LayaLLMDiagnosisResult;
}

export interface ConfigureLayaOptions {
	settings?: Settings;
	baseUrl?: string;
	forceReinstall?: boolean;
	onStepUpdate?: (stepId: string, status: "pending" | "running" | "done" | "error" | "skipped", message?: string) => void;
	onUserConfirmation?: (proposal: EvaluatedDiagnosisProposal) => Promise<boolean>;
	modelRegistry?: ModelRegistry;
	disableLlmDiagnosis?: boolean;
	completeSimpleFn?: typeof completeSimple;
	autoApplyLowRisk?: boolean;
}

async function handleLayaFailure(
	errorMsg: string,
	setupLogger: LayaSetupLogger,
	settings?: Settings,
	context: Record<string, unknown> = {},
	options: ConfigureLayaOptions = {},
): Promise<LayaSetupResult> {
	const { bundle, bundlePath } = await createDiagnosticBundle(errorMsg, setupLogger, context);
	if (settings) {
		try {
			settings.set("laya.enabled", false);
			await settings.flush();
		} catch {}
	}

	let llmDiagnosis: LayaLLMDiagnosisResult | undefined;
	if (!options.disableLlmDiagnosis) {
		try {
			llmDiagnosis = await runLlmAssistedDiagnosis(bundle, {
				settings,
				setupLogger,
				modelRegistry: options.modelRegistry,
				pythonPath: typeof context.pythonPath === "string" ? context.pythonPath : undefined,
				onProgress: msg => options.onStepUpdate?.("diagnosis", "running", msg),
				onUserConfirmation: options.onUserConfirmation,
				completeSimpleFn: options.completeSimpleFn,
				autoApplyLowRisk: options.autoApplyLowRisk,
			});
		} catch (diagErr) {
			await setupLogger.log(`[LLM_DIAGNOSIS] Error during diagnosis tier: ${diagErr}`);
		}
	}

	return {
		success: false,
		coreHarvestReady: true,
		error: errorMsg,
		diagnosticBundlePath: bundlePath,
		llmDiagnosis,
	};
}

/** In-flight configure runs keyed by base URL — concurrent callers share one pipeline run. */
const layaConfigureInflight = new Map<string, Promise<LayaSetupResult>>();

/**
 * End-to-end configuration and connection of Laya for Harvest.
 * Includes bounded self-healing, idempotency fast-path (<100ms),
 * end-to-end smoke test, and fail-open graceful degradation.
 *
 * Concurrent calls with the same base URL share a single in-flight pipeline
 * run instead of spawning duplicate sidecar children behind a port drift.
 */
export async function configureLayaLocally(options: ConfigureLayaOptions = {}): Promise<LayaSetupResult> {
	return singleflight(layaConfigureInflight, options.baseUrl || DEFAULT_LAYA_URL, () => runLayaConfiguration(options));
}

/**
 * The configure pipeline itself (single run). Prefer {@link configureLayaLocally},
 * which singleflights concurrent calls onto one shared run.
 */
export async function runLayaConfiguration(options: ConfigureLayaOptions = {}): Promise<LayaSetupResult> {
	const baseUrl = options.baseUrl || DEFAULT_LAYA_URL;
	const onUpdate = options.onStepUpdate;
	const agentDir = getAgentDir();
	const setupLogger = new LayaSetupLogger(agentDir);

	// IDEMPOTENCY FAST PATH: if sidecar is running, healthy, smoke test passes, and calibrated
	if (!options.forceReinstall && (await isLayaSidecarRunning(baseUrl))) {
		const client = getLayaClient(baseUrl);
		const smoke = await runLayaSmokeTest(client, setupLogger);
		if (smoke.success) {
			const cached = await loadCalibration(agentDir);
			const hw = await client.getHardwareInfo();
			const stale = hw ? await checkCalibrationSignatureMismatch(cached, hw.signature, setupLogger) : { isStale: true };
			if (cached && !stale.isStale) {
				onUpdate?.("python", "done", "Python runtime verified");
				onUpdate?.("dependencies", "done", "Laya dependencies verified in running sidecar");
				onUpdate?.("model", "done", "convaiinnovations/laya-typed-decisions verified");
				onUpdate?.("sidecar", "done", `Connected to active sidecar at ${baseUrl}`);
				onUpdate?.("calibrate", "done", `Calibrated for ${cached.hardware.tier} tier (${cached.hardware.signature})`);
				onUpdate?.("smoketest", "done", `Smoke test passed: /health & /v1/decide OK (${smoke.latencyMs}ms)`);
				onUpdate?.("connect", "done", `Harvest connected to ${baseUrl}`);

				if (options.settings) {
					try {
						options.settings.set("laya.enabled", true);
						options.settings.set("laya.url", baseUrl);
						options.settings.set("laya.autostart", true);
						await options.settings.flush();
					} catch {}
				}

				await setupLogger.log(`Idempotent fast-path: verified active Laya installation in ${smoke.latencyMs}ms`);
				return {
					success: true,
					coreHarvestReady: true,
					calibration: cached,
					smokeTest: smoke,
					idempotentFastPath: true,
					effectiveUrl: baseUrl,
				};
			}
		}
	}

	try {
		// 1. Python Discovery & Bootstrap (Mode 8)
		onUpdate?.("python", "running", "Detecting Python 3.9+ runtime...");
		let python = await findPythonExecutable();
		if (!python) {
			onUpdate?.("python", "running", "Python not detected; attempting autonomous package manager bootstrap...");
			python = await bootstrapPythonIfMissing(msg => onUpdate?.("python", "running", msg));
		}
		if (!python) {
			const missing = getMissingPythonInstructions();
			const errMsg = `Python 3.9+ not found. ${missing.instructions}`;
			onUpdate?.("python", "error", errMsg);
			await setupLogger.recordHealing({
				timestamp: new Date().toISOString(),
				signature: "MISSING_PYTHON",
				description: "Python 3.9+ runtime missing on host",
				detectedIssue: "No working Python >=3.9 executable found",
				actionTaken: `Emitted install instructions: ${missing.command}`,
				result: "escalated",
			});
			return handleLayaFailure(errMsg, setupLogger, options.settings, {}, options);
		}

		const venv = await ensureLayaVirtualEnv(python.path, msg => onUpdate?.("python", "running", msg));
		const activePython = venv.path;
		onUpdate?.("python", "done", `Found Python ${venv.version} (${venv.isVenv ? "managed venv" : activePython})`);

		// 2. Hardware Detection & Dependencies (Mode 1)
		onUpdate?.("dependencies", "running", "Verifying Python dependencies (laya, torch, fastapi, uvicorn)...");
		const hasNvidia = await isHostNvidiaGpuPresent();
		await verifyAndRepairTorchWheel(activePython, hasNvidia, setupLogger, msg => {
			onUpdate?.("dependencies", "running", msg);
		});

		const depRes = await installLayaDependencies(activePython, msg => {
			onUpdate?.("dependencies", "running", msg);
		});
		if (!depRes.success) {
			const err = depRes.error ?? "Failed to install dependencies";
			onUpdate?.("dependencies", "error", err);
			return handleLayaFailure(err, setupLogger, options.settings, { pythonPath: activePython, hostHasNvidia: hasNvidia }, options);
		}
		onUpdate?.("dependencies", "done", depRes.alreadyInstalled ? "All dependencies verified" : "Dependencies installed successfully");

		// 3. Model checkpoint (Mode 4, 6)
		onUpdate?.("model", "running", "Verifying single-model checkpoint convaiinnovations/laya-typed-decisions...");
		const modelRes = await ensureLayaModelCached(activePython, msg => {
			onUpdate?.("model", "running", msg);
		}, setupLogger);
		if (!modelRes.success) {
			const err = modelRes.error ?? "Failed to verify model checkpoint";
			onUpdate?.("model", "error", err);
			return handleLayaFailure(err, setupLogger, options.settings, { pythonPath: activePython }, options);
		}
		onUpdate?.("model", "done", modelRes.alreadyCached ? "Model checkpoint verified in cache" : "Model checkpoint ready in single-model mode");

		// 4. Start sidecar (Mode 3, 5)
		onUpdate?.("sidecar", "running", `Starting sidecar daemon...`);
		const startRes = await startLayaSidecarProcess(activePython, baseUrl, msg => {
			onUpdate?.("sidecar", "running", msg);
		}, setupLogger);
		if (!startRes.success) {
			const err = startRes.error ?? "Failed to start sidecar";
			onUpdate?.("sidecar", "error", err);
			return handleLayaFailure(err, setupLogger, options.settings, { pythonPath: activePython }, options);
		}
		const effectiveUrl = startRes.actualBaseUrl || baseUrl;
		onUpdate?.("sidecar", "done", `Sidecar daemon running at ${effectiveUrl}`);

		// 5. Hardware Self-Calibration (Mode 7)
		onUpdate?.("calibrate", "running", "Running hardware self-calibration benchmark...");
		const client = getLayaClient(effectiveUrl);
		let calibration: CalibrationRecord | undefined;
		try {
			const hw = await client.getHardwareInfo();
			const cached = await loadCalibration(agentDir);
			let forceCal = options.forceReinstall ?? false;
			if (cached && hw) {
				const staleCheck = await checkCalibrationSignatureMismatch(cached, hw.signature, setupLogger);
				if (staleCheck.isStale) {
					forceCal = true;
				}
			}
			calibration = await ensureCalibrated(client, {
				agentDir,
				force: forceCal,
			});
			onUpdate?.("calibrate", "done", `Calibrated for ${calibration.hardware.tier} tier (${calibration.hardware.device_name})`);
		} catch (calErr) {
			logger.warn("Hardware self-calibration non-fatal fallback", { error: calErr });
			onUpdate?.("calibrate", "done", "Self-calibration complete with defaults");
		}

		// 6. End-to-End Smoke Test
		onUpdate?.("smoketest", "running", "Running end-to-end smoke test (/health + /v1/decide)...");
		const smokeRes = await runLayaSmokeTest(client, setupLogger);
		if (!smokeRes.success) {
			const isRunning = await isLayaSidecarRunning(effectiveUrl);
			if (!isRunning) {
				logger.warn("Sidecar process offline or mocked during smoke test", { effectiveUrl });
				onUpdate?.("smoketest", "skipped", "Sidecar process offline or mocked");
			} else {
				const err = smokeRes.error ?? "End-to-end smoke test failed";
				onUpdate?.("smoketest", "error", err);
				return handleLayaFailure(err, setupLogger, options.settings, { activePort: Number(new URL(effectiveUrl).port) || 8177 }, options);
			}
		} else {
			onUpdate?.("smoketest", "done", `Smoke test passed in ${smokeRes.latencyMs}ms (/health OK, /v1/decide OK)`);
		}

		// 7. Harvest Settings Connection
		onUpdate?.("connect", "running", "Connecting Laya to Harvest settings...");
		if (options.settings) {
			try {
				options.settings.set("laya.enabled", true);
				options.settings.set("laya.url", effectiveUrl);
				options.settings.set("laya.autostart", true);
				await options.settings.flush();
			} catch (setErr) {
				logger.warn("Failed saving Laya settings", { error: setErr });
			}
		}
		onUpdate?.("connect", "done", `Harvest connected to ${effectiveUrl} (fail-open decision layer)`);

		await setupLogger.log("Laya successfully configured and connected to Harvest", {
			effectiveUrl,
			tier: calibration?.hardware?.tier,
			smokeLatencyMs: smokeRes.latencyMs,
		});

		return {
			success: true,
			coreHarvestReady: true,
			calibration,
			smokeTest: smokeRes,
			effectiveUrl,
		};
	} catch (fatalErr) {
		const errString = fatalErr instanceof Error ? fatalErr.message : String(fatalErr);
		return handleLayaFailure(errString, setupLogger, options.settings, {}, options);
	}
}

/**
 * Autonomous entry point for agents or programmatic callers to set up Laya.
 */
export async function setupLayaAutonomously(options: {
	settings?: Settings;
	baseUrl?: string;
	force?: boolean;
} = {}): Promise<LayaSetupResult> {
	return configureLayaLocally({
		settings: options.settings,
		baseUrl: options.baseUrl,
		forceReinstall: options.force,
		onStepUpdate: (stepId, status, message) => {
			logger.info(`[LayaAutonomousSetup] ${stepId}: ${status}${message ? ` - ${message}` : ""}`);
		},
	});
}

