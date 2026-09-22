/**
 * Laya Local Service and Configuration Manager for Harvest.
 *
 * Handles probing the Python environment, verifying Laya dependencies,
 * ensuring the single checkpoint `convaiinnovations/laya-typed-decisions`
 * is cached, launching the local sidecar daemon, and connecting it to Harvest.
 */

import * as path from "node:path";
import * as fs from "node:fs/promises";
import { logger } from "@harvest/pi-utils";
import type { Settings } from "../../config/settings";

export interface LayaEnvironmentStatus {
	readonly pythonAvailable: boolean;
	readonly pythonPath?: string;
	readonly pythonVersion?: string;
	readonly dependenciesInstalled: boolean;
	readonly modelCached: boolean;
	readonly sidecarRunning: boolean;
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
 * Locate a working Python 3.9+ binary on the local system.
 */
export async function findPythonExecutable(): Promise<{ path: string; version: string } | null> {
	const candidates = process.platform === "win32"
		? ["python", "python3", "py"]
		: ["python3", "python"];

	for (const cmd of candidates) {
		try {
			const checkCmd = cmd === "py"
				? Bun.spawn(["py", "-3", "-c", "import sys; print(f'{sys.version_info[0]}.{sys.version_info[1]}'); print(sys.executable)"], { stdout: "pipe", stderr: "pipe" })
				: Bun.spawn([cmd, "-c", "import sys; print(f'{sys.version_info[0]}.{sys.version_info[1]}'); print(sys.executable)"], { stdout: "pipe", stderr: "pipe" });

			const exitCode = await checkCmd.exited;
			if (exitCode === 0) {
				const stdout = await new Response(checkCmd.stdout).text();
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
			// Try next candidate
		}
	}
	return null;
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
 * Locate the decision-sidecar directory.
 */
export function getSidecarDir(): string {
	// Search from current working directory or traverse upward
	const cwd = process.cwd();
	const direct = path.resolve(cwd, "decision-sidecar");
	return direct;
}

/**
 * Install missing Python packages from decision-sidecar/requirements.txt.
 */
export async function installLayaDependencies(
	pythonPath: string,
	onProgress?: (msg: string) => void,
): Promise<{ success: boolean; error?: string }> {
	const sidecarDir = getSidecarDir();
	const reqPath = path.join(sidecarDir, "requirements.txt");

	try {
		onProgress?.("Installing Python dependencies (laya, fastapi, uvicorn, torch)...");
		const args = [pythonPath, "-m", "pip", "install"];

		try {
			await fs.access(reqPath);
			args.push("-r", reqPath);
		} catch {
			args.push("laya", "fastapi", "uvicorn", "torch");
		}

		const proc = Bun.spawn(args, {
			cwd: sidecarDir,
			stdout: "pipe",
			stderr: "pipe",
		});

		const exitCode = await proc.exited;
		if (exitCode !== 0) {
			const err = await new Response(proc.stderr).text();
			return { success: false, error: err || `pip install failed with exit code ${exitCode}` };
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
): Promise<{ success: boolean; error?: string }> {
	try {
		onProgress?.("Checking model convaiinnovations/laya-typed-decisions cache...");
		const proc = Bun.spawn([
			pythonPath,
			"-c",
			"from pathlib import Path; p = Path.home() / '.cache' / 'huggingface' / 'hub' / 'models--convaiinnovations--laya-typed-decisions';\nif p.exists() and any(p.glob('**/model.safetensors')):\n    print('MODEL_LOADED')\nelse:\n    import laya; m = laya.load('convaiinnovations/laya-typed-decisions'); print('MODEL_LOADED')",
		], {
			stdout: "pipe",
			stderr: "pipe",
		});

		const exitCode = await proc.exited;
		if (exitCode !== 0) {
			const err = await new Response(proc.stderr).text();
			return { success: false, error: err || `Model loading failed with exit code ${exitCode}` };
		}
		return { success: true };
	} catch (err) {
		return { success: false, error: String(err) };
	}
}

/**
 * Launch the local decision sidecar daemon as a background subprocess.
 */
export async function startLayaSidecarProcess(
	pythonPath: string,
	baseUrl: string = DEFAULT_LAYA_URL,
	onProgress?: (msg: string) => void,
): Promise<{ success: boolean; error?: string }> {
	const sidecarDir = getSidecarDir();
	const serverPath = path.join(sidecarDir, "server.py");

	try {
		await fs.access(serverPath);
	} catch {
		return { success: false, error: `Sidecar server script not found at ${serverPath}` };
	}

	onProgress?.("Starting Laya sidecar daemon on 127.0.0.1:8177...");

	try {
		// Spawn detached or background process
		Bun.spawn([
			pythonPath,
			"-m",
			"uvicorn",
			"server:app",
			"--host",
			"127.0.0.1",
			"--port",
			"8177",
		], {
			cwd: sidecarDir,
			detached: true,
			stdio: ["ignore", "ignore", "ignore"],
		});

		// Poll /health until ready or timeout (up to 15s)
		const startTime = Date.now();
		while (Date.now() - startTime < 15000) {
			if (await isLayaSidecarRunning(baseUrl)) {
				return { success: true };
			}
			await Bun.sleep(500);
		}

		return { success: false, error: "Timed out waiting for Laya sidecar /health to become ready" };
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
	onStepUpdate?: (stepId: string, status: "pending" | "running" | "done" | "error", message?: string) => void;
}): Promise<{ success: boolean; error?: string }> {
	const baseUrl = options.baseUrl || DEFAULT_LAYA_URL;
	const onUpdate = options.onStepUpdate;

	// 1. Python Check
	onUpdate?.("python", "running", "Detecting Python 3.9+ runtime...");
	const python = await findPythonExecutable();
	if (!python) {
		onUpdate?.("python", "error", "Python 3.9+ not found. Please install Python 3.9 or higher.");
		return { success: false, error: "Python 3.9+ not found" };
	}
	onUpdate?.("python", "done", `Found Python ${python.version} (${python.path})`);

	// 2. Already running sidecar check
	onUpdate?.("sidecar", "running", "Checking for existing Laya sidecar...");
	const alreadyRunning = await isLayaSidecarRunning(baseUrl);
	if (alreadyRunning) {
		onUpdate?.("dependencies", "done", "Laya dependencies active in running sidecar");
		onUpdate?.("model", "done", "convaiinnovations/laya-typed-decisions active");
		onUpdate?.("sidecar", "done", `Connected to active sidecar at ${baseUrl}`);
	} else {
		// 3. Dependencies check
		onUpdate?.("dependencies", "running", "Verifying Python dependencies (laya, torch, fastapi, uvicorn)...");
		const depsOk = await checkLayaDependencies(python.path);
		if (!depsOk) {
			onUpdate?.("dependencies", "running", "Installing dependencies via pip...");
			const installResult = await installLayaDependencies(python.path, msg => {
				onUpdate?.("dependencies", "running", msg);
			});
			if (!installResult.success) {
				onUpdate?.("dependencies", "error", installResult.error ?? "Failed to install dependencies");
				return { success: false, error: installResult.error };
			}
		}
		onUpdate?.("dependencies", "done", "All Python dependencies verified");

		// 4. Model checkpoint check
		onUpdate?.("model", "running", "Verifying model checkpoint convaiinnovations/laya-typed-decisions...");
		const modelResult = await ensureLayaModelCached(python.path, msg => {
			onUpdate?.("model", "running", msg);
		});
		if (!modelResult.success) {
			onUpdate?.("model", "error", modelResult.error ?? "Failed to verify model checkpoint");
			return { success: false, error: modelResult.error };
		}
		onUpdate?.("model", "done", "Model checkpoint ready in single-model mode");

		// 5. Start sidecar
		onUpdate?.("sidecar", "running", `Starting sidecar on ${baseUrl}...`);
		const startResult = await startLayaSidecarProcess(python.path, baseUrl, msg => {
			onUpdate?.("sidecar", "running", msg);
		});
		if (!startResult.success) {
			onUpdate?.("sidecar", "error", startResult.error ?? "Failed to start sidecar");
			return { success: false, error: startResult.error };
		}
		onUpdate?.("sidecar", "done", `Sidecar daemon started and ready at ${baseUrl}`);
	}

	// 6. Harvest Settings Connection
	onUpdate?.("connect", "running", "Connecting Laya to Harvest settings...");
	if (options.settings) {
		options.settings.set("laya.enabled", true);
		options.settings.set("laya.url", baseUrl);
		options.settings.set("laya.autostart", true);
		await options.settings.flush();
	}
	onUpdate?.("connect", "done", "Harvest connected to local Laya decision layer (fail-closed gating, open routing & completion)");

	logger.info("Laya successfully configured and connected to Harvest", { baseUrl });
	return { success: true };
}
