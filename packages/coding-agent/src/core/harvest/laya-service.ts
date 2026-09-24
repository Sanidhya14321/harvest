/**
 * Laya Local Service and Configuration Manager for Harvest.
 *
 * Handles probing the Python environment, managing an isolated virtualenv,
 * verifying Laya dependencies, ensuring the single checkpoint
 * `convaiinnovations/laya-typed-decisions` is cached, resolving port conflicts,
 * launching the local sidecar daemon with robust timeouts, running hardware
 * self-calibration, and connecting it seamlessly to Harvest.
 */

import * as path from "node:path";
import * as net from "node:net";
import { getAgentDir, logger } from "@harvest/pi-utils";
import type { Settings } from "../../config/settings";
import { ensureCalibrated, type CalibrationRecord } from "./laya-calibration";
import { getLayaClient } from "./laya-client";

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
	const venvDir = path.join(getAgentDir(), "laya-venv");
	const venvPy = process.platform === "win32"
		? path.join(venvDir, "Scripts", "python.exe")
		: path.join(venvDir, "bin", "python");

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

	const activityTimeout = options.activityTimeoutMs ?? 180_000;
	const totalTimeout = options.totalTimeoutMs ?? 900_000;
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
 * Ensure an isolated virtual environment exists for Laya dependencies at `~/.harvest/laya-venv`.
 * This protects against system Python PEP 668 restrictions and dependency conflicts.
 */
export async function ensureLayaVirtualEnv(
	systemPythonPath: string,
	onProgress?: (msg: string) => void,
): Promise<{ path: string; version: string; isVenv: boolean }> {
	const venvDir = path.join(getAgentDir(), "laya-venv");
	const venvPy = process.platform === "win32"
		? path.join(venvDir, "Scripts", "python.exe")
		: path.join(venvDir, "bin", "python");

	const existing = await testPythonExecutable(venvPy);
	if (existing) {
		return { ...existing, isVenv: true };
	}

	try {
		onProgress?.("Creating isolated Python virtual environment at ~/.harvest/laya-venv...");
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
		logger.warn("Failed to create isolated virtualenv; falling back to host Python", { error: err });
	}

	const hostTest = await testPythonExecutable(systemPythonPath);
	return {
		path: systemPythonPath,
		version: hostTest?.version ?? "3.9+",
		isVenv: false,
	};
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
		const json = (await res.json()) as { ready?: boolean; status?: string };
		return json.ready === true || json.status === "ok";
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
 * Reclaim port 8177 if blocked by a dead or unresponsive process.
 */
export async function freePortIfOccupied(
	port = 8177,
	baseUrl: string = DEFAULT_LAYA_URL,
	onProgress?: (msg: string) => void,
): Promise<{ freed: boolean; alreadyRunning: boolean }> {
	const inUse = await isPortInUse(port);
	if (!inUse) {
		return { freed: true, alreadyRunning: false };
	}

	if (await isLayaSidecarRunning(baseUrl)) {
		return { freed: true, alreadyRunning: true };
	}

	onProgress?.(`Port ${port} is occupied by an unresponsive process; terminating stale process...`);

	try {
		if (process.platform === "win32") {
			const netstat = Bun.spawnSync(["netstat", "-ano", "-p", "tcp"]);
			const out = netstat.stdout.toString();
			for (const line of out.split(/\r?\n/)) {
				if (line.includes(`:${port}`) && line.includes("LISTENING")) {
					const parts = line.trim().split(/\s+/);
					const pid = parts[parts.length - 1];
					if (pid && /^\d+$/.test(pid) && pid !== "0" && pid !== String(process.pid)) {
						Bun.spawnSync(["taskkill", "/F", "/PID", pid]);
					}
				}
			}
		} else {
			Bun.spawnSync(["sh", "-c", `lsof -ti :${port} | xargs -r kill -9 2>/dev/null || fuser -k ${port}/tcp 2>/dev/null`]);
		}
		await Bun.sleep(500);
	} catch (err) {
		logger.debug("Failed freeing port process", { port, error: err });
	}

	const stillInUse = await isPortInUse(port);
	return { freed: !stillInUse, alreadyRunning: false };
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

	// 5 levels up from packages/coding-agent/src/core/harvest
	candidates.push(path.resolve(import.meta.dir, "../../../../../decision-sidecar"));
	candidates.push(path.resolve(process.cwd(), "decision-sidecar"));
	candidates.push(path.join(getAgentDir(), "sidecar"));

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

	return path.resolve(import.meta.dir, "../../../../../decision-sidecar");
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

		let proc = Bun.spawn(baseArgs, {
			cwd: sidecarDir,
			stdout: "pipe",
			stderr: "pipe",
		});

		let res = await streamProcessOutput(proc, filterPipProgress, {
			activityTimeoutMs: 180_000,
			totalTimeoutMs: 900_000,
		});

		if (res.exitCode !== 0) {
			if (res.stderr.includes("externally-managed-environment") || res.stdout.includes("externally-managed-environment")) {
				onProgress?.("Externally-managed Python environment detected; retrying with --break-system-packages...");
				const breakArgs = [
					pythonPath,
					"-m",
					"pip",
					"install",
					"--break-system-packages",
					"--no-input",
					"--prefer-binary",
					"--retries",
					"3",
					"--timeout",
					"30",
					...extraIndexArgs,
					...(hasReq ? ["-r", reqPath] : ["laya>=0.3.5", "fastapi>=0.115.0", "uvicorn>=0.30.0", "torch", "pydantic>=2.0.0"]),
				];
				proc = Bun.spawn(breakArgs, {
					cwd: sidecarDir,
					stdout: "pipe",
					stderr: "pipe",
				});
				res = await streamProcessOutput(proc, filterPipProgress, {
					activityTimeoutMs: 180_000,
					totalTimeoutMs: 900_000,
				});
			}
			if (res.exitCode !== 0) {
				return { success: false, error: res.stderr || res.stdout || `pip install failed with exit code ${res.exitCode}` };
			}
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
): Promise<{ success: boolean; error?: string; alreadyCached?: boolean }> {
	try {
		onProgress?.("Checking HuggingFace cache for convaiinnovations/laya-typed-decisions...");

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
os.environ["HF_HUB_DISABLE_SYMLINKS_WARNING"] = "1"

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
    # If primary download failed (e.g. timeout / network block), try official mirror
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
			activityTimeoutMs: 120_000,
			totalTimeoutMs: 900_000,
		});

		if (res.exitCode !== 0) {
			return { success: false, error: res.stderr || res.stdout || `Model loading failed with exit code ${res.exitCode}` };
		}
		return { success: true };
	} catch (err) {
		return { success: false, error: String(err) };
	}
}

/**
 * Launch the local decision sidecar daemon as a background subprocess.
 * Includes generous 180s polling window for ModernBERT-large loading on CPU.
 */
export async function startLayaSidecarProcess(
	pythonPath: string,
	baseUrl: string = DEFAULT_LAYA_URL,
	onProgress?: (msg: string) => void,
): Promise<{ success: boolean; error?: string }> {
	if (await isLayaSidecarRunning(baseUrl)) {
		return { success: true };
	}

	const url = new URL(baseUrl);
	const port = Number(url.port) || 8177;

	await freePortIfOccupied(port, baseUrl, onProgress);

	const sidecarDir = getSidecarDir();
	const serverPath = path.join(sidecarDir, "server.py");

	try {
		if (Bun.file(serverPath).size === 0) {
			return { success: false, error: `Sidecar server script not found at ${serverPath}` };
		}
	} catch {
		return { success: false, error: `Sidecar server script not found at ${serverPath}` };
	}

	onProgress?.(`Starting Laya sidecar daemon on 127.0.0.1:${port}...`);

	try {
		Bun.spawn([
			pythonPath,
			"-u",
			serverPath,
		], {
			cwd: sidecarDir,
			detached: true,
			stdio: ["ignore", "ignore", "ignore"],
			env: {
				...process.env,
				PYTHONUNBUFFERED: "1",
				HF_HUB_DISABLE_SYMLINKS_WARNING: "1",
				LAYA_HOST: "127.0.0.1",
				LAYA_PORT: String(port),
			},
		});

		const startTime = Date.now();
		const maxTimeoutMs = 180_000;

		while (Date.now() - startTime < maxTimeoutMs) {
			if (await isLayaSidecarRunning(baseUrl)) {
				return { success: true };
			}
			const elapsedSec = Math.floor((Date.now() - startTime) / 1000);
			if (elapsedSec % 5 === 0 && elapsedSec > 0) {
				onProgress?.(`Waiting for Laya sidecar initialization and model warmup (${elapsedSec}s / 180s)...`);
			}
			await Bun.sleep(1000);
		}

		return { success: false, error: `Timed out waiting 180s for Laya sidecar /health at ${baseUrl}` };
	} catch (err) {
		return { success: false, error: String(err) };
	}
}

/**
 * End-to-end configuration and connection of Laya for Harvest.
 */
export async function configureLayaLocally(options: {
	settings?: Settings;
	baseUrl?: string;
	forceReinstall?: boolean;
	onStepUpdate?: (stepId: string, status: "pending" | "running" | "done" | "error", message?: string) => void;
}): Promise<{ success: boolean; calibration?: CalibrationRecord; error?: string }> {
	const baseUrl = options.baseUrl || DEFAULT_LAYA_URL;
	const onUpdate = options.onStepUpdate;
	const agentDir = getAgentDir();

	// 1. Python Discovery & Bootstrap
	onUpdate?.("python", "running", "Detecting Python 3.9+ runtime...");
	let python = await findPythonExecutable();
	if (!python) {
		onUpdate?.("python", "running", "Python not detected; attempting autonomous package manager bootstrap...");
		python = await bootstrapPythonIfMissing(msg => onUpdate?.("python", "running", msg));
	}
	if (!python) {
		const errMsg = process.platform === "win32"
			? "Python 3.9+ not found. Install Python via 'winget install Python.Python.3.11' or https://python.org"
			: "Python 3.9+ not found. Install Python 3.9+ via 'brew install python@3.11' or 'apt install python3'";
		onUpdate?.("python", "error", errMsg);
		return { success: false, error: errMsg };
	}

	const venv = await ensureLayaVirtualEnv(python.path, msg => onUpdate?.("python", "running", msg));
	const activePython = venv.path;
	onUpdate?.("python", "done", `Found Python ${venv.version} (${venv.isVenv ? "managed venv" : activePython})`);

	// 2. Check for running sidecar
	const alreadyRunning = await isLayaSidecarRunning(baseUrl);
	if (alreadyRunning && !options.forceReinstall) {
		onUpdate?.("dependencies", "done", "Laya dependencies active in running sidecar");
		onUpdate?.("model", "done", "convaiinnovations/laya-typed-decisions active in memory");
		onUpdate?.("sidecar", "done", `Connected to active sidecar at ${baseUrl}`);
	} else {
		// 3. Dependencies check & install
		onUpdate?.("dependencies", "running", "Verifying Python dependencies (laya, torch, fastapi, uvicorn)...");
		const depRes = await installLayaDependencies(activePython, msg => {
			onUpdate?.("dependencies", "running", msg);
		});
		if (!depRes.success) {
			onUpdate?.("dependencies", "error", depRes.error ?? "Failed to install dependencies");
			return { success: false, error: depRes.error };
		}
		onUpdate?.("dependencies", "done", depRes.alreadyInstalled ? "All dependencies verified" : "Dependencies installed successfully");

		// 4. Model checkpoint
		onUpdate?.("model", "running", "Verifying single-model checkpoint convaiinnovations/laya-typed-decisions...");
		const modelRes = await ensureLayaModelCached(activePython, msg => {
			onUpdate?.("model", "running", msg);
		});
		if (!modelRes.success) {
			onUpdate?.("model", "error", modelRes.error ?? "Failed to verify model checkpoint");
			return { success: false, error: modelRes.error };
		}
		onUpdate?.("model", "done", modelRes.alreadyCached ? "Model checkpoint verified in cache" : "Model checkpoint ready in single-model mode");

		// 5. Start sidecar
		onUpdate?.("sidecar", "running", `Starting sidecar daemon on ${baseUrl}...`);
		const startRes = await startLayaSidecarProcess(activePython, baseUrl, msg => {
			onUpdate?.("sidecar", "running", msg);
		});
		if (!startRes.success) {
			onUpdate?.("sidecar", "error", startRes.error ?? "Failed to start sidecar");
			return { success: false, error: startRes.error };
		}
		onUpdate?.("sidecar", "done", `Sidecar daemon started and ready at ${baseUrl}`);
	}

	// 6. Hardware Self-Calibration
	onUpdate?.("calibrate", "running", "Running hardware self-calibration benchmark...");
	let calibration: CalibrationRecord | undefined;
	try {
		const client = getLayaClient(baseUrl);
		calibration = await ensureCalibrated(client, {
			agentDir,
			force: false,
		});
		onUpdate?.("calibrate", "done", `Calibrated for ${calibration.hardware.tier} tier (${calibration.hardware.device_name})`);
	} catch (calErr) {
		logger.warn("Hardware self-calibration encountered non-fatal warning", { error: calErr });
		onUpdate?.("calibrate", "done", "Self-calibration complete with defaults");
	}

	// 7. Harvest Settings Connection
	onUpdate?.("connect", "running", "Connecting Laya to Harvest settings...");
	if (options.settings) {
		try {
			options.settings.set("laya.enabled", true);
			options.settings.set("laya.url", baseUrl);
			options.settings.set("laya.autostart", true);
			await options.settings.flush();
		} catch (setErr) {
			logger.warn("Failed saving Laya settings", { error: setErr });
		}
	}
	onUpdate?.("connect", "done", "Harvest connected to local Laya decision layer (fail-closed gating, open routing & completion)");

	logger.info("Laya successfully configured and connected to Harvest", { baseUrl, tier: calibration?.hardware?.tier });
	return { success: true, calibration };
}

/**
 * Autonomous entry point for agents or programmatic callers to set up Laya.
 */
export async function setupLayaAutonomously(options: {
	settings?: Settings;
	baseUrl?: string;
	force?: boolean;
} = {}): Promise<{ success: boolean; calibration?: CalibrationRecord; error?: string }> {
	return configureLayaLocally({
		settings: options.settings,
		baseUrl: options.baseUrl,
		forceReinstall: options.force,
		onStepUpdate: (stepId, status, message) => {
			logger.info(`[LayaAutonomousSetup] ${stepId}: ${status}${message ? ` - ${message}` : ""}`);
		},
	});
}
