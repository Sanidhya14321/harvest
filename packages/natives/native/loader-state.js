import * as childProcess from "node:child_process";
import * as fs from "node:fs";
import { createRequire } from "node:module";
import * as os from "node:os";
import * as path from "node:path";
import * as zlib from "node:zlib";
import packageJson from "../package.json" with { type: "json" };
import { embeddedAddon } from "./embedded-addon.js";
import {
	matchesKey as jsMatchesKey,
	matchesKittySequence as jsMatchesKittySequence,
	matchesLegacySequence as jsMatchesLegacySequence,
	parseKey as jsParseKey,
	parseKittySequence as jsParseKittySequence,
} from "./keys-fallback.js";

/**
 * Native addon loader for `@harvest/pi-natives`.
 *
 * Owns every step between "Node imports `native/index.js`" and "the right
 * `pi_natives.<platform>-<arch>*.node` is required, validated, and returned":
 * platform/variant detection, candidate-path resolution, on-disk staging from
 * `node_modules` (Windows update safety), embedded-addon extraction (Bun
 * standalone binaries), version-sentinel validation, and the aggregated error
 * surface for diagnostic-friendly failures.
 *
 * `native/index.js` is reduced to one `loadNative()` call plus the generated
 * surface-area exports between `MARKER_START`/`MARKER_END` (rewritten by
 * `scripts/gen-enums.ts`); everything else lives here so the pure helpers stay
 * unit-testable without triggering the side-effectful module-load path.
 *
 * Background (issue #823): `bun build --compile --define PI_COMPILED=true`
 * substitutes the bare identifier `PI_COMPILED`, NOT `process.env.PI_COMPILED`,
 * so a runtime read of the env var returns `undefined`. Older CommonJS loader
 * code also saw the original build-host absolute path in `__filename`; ESM
 * `import.meta.url` is rewritten to the bunfs URL. The embedded-addon
 * presence (true iff the build pipeline ran `embed:native`, false in the
 * post-build `--reset` stub) is the authoritative compiled-mode signal.
 */

const SUPPORTED_PLATFORMS = [
	"linux-x64",
	"linux-arm64",
	"darwin-x64",
	"darwin-arm64",
	"win32-x64",
	"win32-arm64",
];

/**
 * Streaming startup marker, enabled by `PI_DEBUG_STARTUP`. Local copy of the
 * pi-utils helper (this loader cannot depend on pi-utils). Synchronous on
 * purpose: extraction/dlopen hangs must still leave the `:start` marker.
 * @param {string} text
 */
function startupMarker(text) {
	if (!process.env.PI_DEBUG_STARTUP) return;
	try {
		fs.writeSync(2, `[startup] ${text}\n`);
	} catch {
		// stderr unavailable; markers are best-effort
	}
}

function getNativesDir() {
	const xdgDataHome = process.env.XDG_DATA_HOME;
	if (xdgDataHome && fs.existsSync(path.join(xdgDataHome, "omp"))) {
		return path.join(xdgDataHome, "omp", "natives");
	}
	return path.join(os.homedir(), ".omp", "natives");
}

function resolveLeafPackageDir(platformTag) {
	try {
		const require_ = createRequire(import.meta.url);
		return path.dirname(require_.resolve(`@harvest/pi-natives-${platformTag}/package.json`));
	} catch {
		return null;
	}
}

// =========================================================================
// Pure helpers — re-exported for unit tests in `packages/natives/test/`.
// =========================================================================

/**
 * @param {{
 *   embeddedAddon: { platformTag: string; version: string; files: unknown[] } | null | undefined;
 *   env: Record<string, string | undefined>;
 *   importMetaUrl: string | null | undefined;
 * }} input
 * @returns {boolean}
 */
export function detectCompiledBinary({ embeddedAddon, env, importMetaUrl }) {
	if (embeddedAddon) return true;
	if (env && env.PI_COMPILED) return true;
	if (typeof importMetaUrl === "string") {
		if (importMetaUrl.includes("$bunfs")) return true;
		if (importMetaUrl.includes("~BUN")) return true;
		if (importMetaUrl.includes("%7EBUN")) return true;
	}
	return false;
}
/**
 * @param {{ tag: string; arch: string; variant: "modern" | "baseline" | null | undefined }} input
 * @returns {string[]}
 */
export function getAddonFilenames({ tag, arch, variant }) {
	const defaultFilename = `pi_natives.${tag}.node`;
	if (arch !== "x64" || !variant) return [defaultFilename];
	const baselineFilename = `pi_natives.${tag}-baseline.node`;
	const modernFilename = `pi_natives.${tag}-modern.node`;
	if (variant === "modern") {
		return [modernFilename, baselineFilename, defaultFilename];
	}
	return [baselineFilename, defaultFilename];
}

/**
 * Decide whether the loader should mirror the package's `native/<filename>.node`
 * into the per-version cache directory (`~/.omp/natives/<version>/`) before loading.
 *
 * Windows-only safety net for `bun install -g` updates: when a previous `omp`
 * process is running, bun cannot overwrite the locked `.node` inside
 * `node_modules/@harvest/pi-natives/native/`, leaving an old binary next to a
 * newer `index.js` and producing `<sym> is not a function` crashes on the next
 * launch. Staging into the version-pinned cache:
 *   1. Gives every package version its own filesystem path, so concurrent omp
 *      processes never collide on the same file.
 *   2. Makes the running process keep its handle on the cache copy, freeing bun
 *      to overwrite the `node_modules` copy on subsequent updates.
 * Disabled on non-Windows (no file-lock problem), in workspace dev (`nativeDir`
 * is not inside a `node_modules` segment), and for compiled binaries (handled
 * by `maybeExtractEmbeddedAddon`).
 *
 * @param {{ platform: NodeJS.Platform | string; isCompiledBinary: boolean; nativeDir: string }} input
 * @returns {boolean}
 */
export function shouldStageNodeModulesAddon({ platform, isCompiledBinary, nativeDir }) {
	if (platform !== "win32") return false;
	if (isCompiledBinary) return false;
	// Check both separators independently of the host's `path.sep`: this helper
	// is shared by the loader (running on Windows with `\`) and the test suite
	// (typically running on POSIX hosts when CI executes the regression test).
	const normalizedNativeDir = nativeDir.toLowerCase();
	return normalizedNativeDir.includes("\\node_modules\\") || normalizedNativeDir.includes("/node_modules/");
}

/**
 * @param {{
 *   addonFilenames: string[];
 *   isCompiledBinary: boolean;
 *   stageFromNodeModules?: boolean;
 *   nativeDir: string;
 *   leafPackageDir?: string | null;
 *   execDir: string;
 *   versionedDir: string;
 *   userDataDir: string;
 * }} input
 * @returns {string[]}
 */
export function resolveLoaderCandidates({
	addonFilenames,
	isCompiledBinary,
	stageFromNodeModules = false,
	nativeDir,
	leafPackageDir = null,
	execDir,
	versionedDir,
	userDataDir,
}) {
	const baseReleaseCandidates = addonFilenames.flatMap(filename => [
		path.join(nativeDir, filename),
		path.join(execDir, filename),
	]);
	const leafCandidates = leafPackageDir ? addonFilenames.map(filename => path.join(leafPackageDir, filename)) : [];
	const compiledCandidates = addonFilenames.flatMap(filename => [
		path.join(versionedDir, filename),
		path.join(userDataDir, filename),
	]);
	const stagedCandidates = stageFromNodeModules ? addonFilenames.map(filename => path.join(versionedDir, filename)) : [];
	let releaseCandidates;
	if (isCompiledBinary) {
		releaseCandidates = [...compiledCandidates, ...baseReleaseCandidates];
	} else if (stageFromNodeModules) {
		releaseCandidates = [...stagedCandidates, ...leafCandidates, ...baseReleaseCandidates];
	} else {
		releaseCandidates = [...leafCandidates, ...baseReleaseCandidates];
	}
	return [...new Set(releaseCandidates)];
}

// =========================================================================

function parseReleaseVersion(version) {
	const match = /^(\d+)\.(\d+)\.(\d+)$/.exec(version);
	return match ? [Number(match[1]), Number(match[2]), Number(match[3])] : null;
}

function isOlderReleaseVersion(candidate, current) {
	const candidateParts = parseReleaseVersion(candidate);
	const currentParts = parseReleaseVersion(current);
	if (!candidateParts || !currentParts) return false;
	for (let index = 0; index < candidateParts.length; index++) {
		if (candidateParts[index] !== currentParts[index]) {
			return candidateParts[index] < currentParts[index];
		}
	}
	return false;
}

// A concurrently starting older OMP binary creates or refreshes this directory
// before extracting its addon. Keep fresh directories long enough for that
// startup to finish; a later launch can reclaim them once they are genuinely
// stale.
const NATIVE_CACHE_CLEANUP_GRACE_MS = 10 * 60_000;

/**
 * Create a version cache directory and refresh its activity timestamp before
 * extraction or staging begins. Recursive mkdir does not update the mtime of
 * an existing directory, so the explicit touch is what protects interrupted
 * or partially populated caches from concurrent cleanup.
 *
 * @param {string} versionedDir
 */
export function prepareNativeVersionDir(versionedDir) {
	fs.mkdirSync(versionedDir, { recursive: true });
	const now = new Date();
	fs.utimesSync(versionedDir, now, now);
}

/**
 * Remove version-pinned native cache directories older than the loaded package.
 * Best-effort by design: permission errors and concurrent processes must not
 * abort startup after the native addon has already loaded successfully.
 *
 * @param {{ nativesDir: string; currentVersion: string }} input
 * @returns {string[]}
 */
export function cleanupStaleNativeVersions({ nativesDir, currentVersion }) {
	const removed = [];
	let entries;
	try {
		entries = fs.readdirSync(nativesDir, { withFileTypes: true });
	} catch {
		return removed;
	}

	for (const entry of entries) {
		if (!entry.isDirectory() || !isOlderReleaseVersion(entry.name, currentVersion)) continue;
		const targetPath = path.join(nativesDir, entry.name);
		try {
			const stat = fs.statSync(targetPath);
			if (Date.now() - stat.mtimeMs < NATIVE_CACHE_CLEANUP_GRACE_MS) continue;
			fs.rmSync(targetPath, { recursive: true, force: true });
			removed.push(targetPath);
		} catch {
			// Stale caches are opportunistic cleanup only.
		}
	}
	return removed;
}

// Side-effectful loader. Everything below runs only when `loadNative()` is
// called from `native/index.js` — tests that only import the pure helpers
// above pay nothing for variant detection, subprocess spawns, or fs probes.
// =========================================================================

/**
 * Hidden env key for the resolved x64 variant. Once any context (main thread,
 * worker, subprocess) finishes variant detection, the result is written here
 * so every Bun worker and child process spawned afterwards inherits the same
 * verdict and skips re-detection. See `selectCpuVariant` for the lookup order.
 */
const VARIANT_CACHE_ENV_KEY = "__PI_NATIVE_VARIANT_CACHE";

/**
 * Spawn `command` with `args` and capture stdout. Prefers `Bun.spawnSync`
 * because Bun's `child_process.spawnSync` shim has been observed to return
 * non-zero / null in worker threads on macOS even when the same binary works
 * fine from the parent — the failure mode behind issue #3238, where the worker
 * silently falls back to the "baseline" variant. Falls back to the Node shim
 * for non-Bun embeds.
 */
function runCommand(command, args) {
	if (typeof Bun !== "undefined" && typeof Bun.spawnSync === "function") {
		try {
			const result = Bun.spawnSync([command, ...args], { stdout: "pipe", stderr: "pipe" });
			if (result.exitCode === 0) {
				return result.stdout.toString("utf-8").trim();
			}
		} catch {
			// fall through to childProcess
		}
	}
	try {
		const result = childProcess.spawnSync(command, args, { encoding: "utf-8" });
		if (result.error) return null;
		if (result.status !== 0) return null;
		return (result.stdout || "").trim();
	} catch {
		return null;
	}
}

function getVariantOverride() {
	const value = process.env.PI_NATIVE_VARIANT;
	if (!value) return null;
	if (value === "modern" || value === "baseline") return value;
	return null;
}

function detectAvx2Support() {
	if (process.arch !== "x64") {
		return false;
	}

	if (process.platform === "linux") {
		try {
			const cpuInfo = fs.readFileSync("/proc/cpuinfo", "utf8");
			return /\bavx2\b/i.test(cpuInfo);
		} catch {
			return false;
		}
	}

	if (process.platform === "darwin") {
		// Try the absolute path before bare `sysctl`: PATH may not include
		// `/usr/sbin` in worker/embedded spawn contexts (issue #3238).
		for (const sysctlBin of ["/usr/sbin/sysctl", "sysctl"]) {
			const leaf7 = runCommand(sysctlBin, ["-n", "machdep.cpu.leaf7_features"]);
			if (leaf7 && /\bAVX2\b/i.test(leaf7)) return true;
			const features = runCommand(sysctlBin, ["-n", "machdep.cpu.features"]);
			if (features && /\bAVX2\b/i.test(features)) return true;
		}
		return false;
	}

	if (process.platform === "win32") {
		// Under Bun, ask the kernel: PF_AVX2_INSTRUCTIONS_AVAILABLE == 40. Exact,
		// and ~0.5 ms against ~270 ms for the PowerShell spawn it replaces on the
		// startup path.
		if (typeof Bun !== "undefined") {
			try {
				const { dlopen, FFIType } = createRequire(import.meta.url)("bun:ffi");
				const kernel32 = dlopen("kernel32.dll", {
					IsProcessorFeaturePresent: { args: [FFIType.u32], returns: FFIType.i32 },
				});
				try {
					return kernel32.symbols.IsProcessorFeaturePresent(40) !== 0;
				} finally {
					kernel32.close();
				}
			} catch {
				// No FFI (embedder policy, unusual host): fall through to the shell probe.
			}
		}
		// Node embeds have no `bun:ffi`. `[System.Runtime.Intrinsics.X86.Avx2]`
		// exists only on .NET Core, so `pwsh` (PowerShell 7) answers correctly
		// while a stock `powershell.exe` (Windows PowerShell 5.1, .NET Framework)
		// raises TypeNotFound and pins such hosts to the baseline addon.
		for (const shell of ["pwsh.exe", "powershell.exe"]) {
			const output = runCommand(shell, [
				"-NoProfile",
				"-NonInteractive",
				"-Command",
				"[System.Runtime.Intrinsics.X86.Avx2]::IsSupported",
			]);
			if (output && output.toLowerCase() === "true") return true;
			if (output && output.toLowerCase() === "false") return false;
		}
		return false;
	}

	return false;
}

/**
 * Pure variant-selection helper, exposed for unit tests. Resolution order:
 *
 *   1. `override` (user-facing `PI_NATIVE_VARIANT` env var). Always wins.
 *   2. The private `__PI_NATIVE_VARIANT_CACHE` env var, populated by the first
 *      context that detected at runtime. Lets child workers / subprocesses
 *      inherit the main thread's verdict instead of re-spawning `sysctl` etc.
 *      from a worker context where the spawn may fail (issue #3238).
 *   3. `detectAvx2()` — the slow path, called at most once per process.
 *
 * Non-x64 architectures return `{ variant: null }` and never set the cache.
 * When detection runs, the result is surfaced as `cacheEnvKey`/`cacheEnvValue`
 * so the caller can write `process.env` (the pure helper itself stays
 * side-effect-free, which keeps it easy to test).
 *
 * @param {{
 *   arch: string;
 *   override: "modern" | "baseline" | null | undefined;
 *   env: Record<string, string | undefined>;
 *   detectAvx2: () => boolean;
 * }} input
 * @returns {{
 *   variant: "modern" | "baseline" | null;
 *   source: "non-x64" | "override" | "cache" | "detect";
 *   cacheEnvKey?: string;
 *   cacheEnvValue?: string;
 * }}
 */
export function selectCpuVariant({ arch, override, env, detectAvx2 }) {
	if (arch !== "x64") return { variant: null, source: "non-x64" };
	if (override === "modern" || override === "baseline") {
		return { variant: override, source: "override" };
	}
	const cached = env[VARIANT_CACHE_ENV_KEY];
	if (cached === "modern" || cached === "baseline") {
		return { variant: cached, source: "cache" };
	}
	const variant = detectAvx2() ? "modern" : "baseline";
	return {
		variant,
		source: "detect",
		cacheEnvKey: VARIANT_CACHE_ENV_KEY,
		cacheEnvValue: variant,
	};
}

function resolveCpuVariant(override) {
	const result = selectCpuVariant({
		arch: process.arch,
		override,
		env: process.env,
		detectAvx2: detectAvx2Support,
	});
	if (result.cacheEnvKey) {
		process.env[result.cacheEnvKey] = result.cacheEnvValue;
	}
	return result.variant;
}

function selectEmbeddedAddonFile(selectedVariant) {
	if (!embeddedAddon) return null;
	const defaultFile = embeddedAddon.files.find(file => file.variant === "default") || null;
	if (process.arch !== "x64") return defaultFile || embeddedAddon.files[0] || null;
	if (selectedVariant === "modern") {
		return (
			embeddedAddon.files.find(file => file.variant === "modern") ||
			embeddedAddon.files.find(file => file.variant === "baseline") ||
			null
		);
	}
	return embeddedAddon.files.find(file => file.variant === "baseline") || null;
}

function readTarString(buffer, offset, length) {
	const end = Math.min(offset + length, buffer.length);
	let stringEnd = offset;
	while (stringEnd < end && buffer[stringEnd] !== 0) stringEnd++;
	return buffer.toString("utf8", offset, stringEnd);
}

function readTarOctal(buffer, offset, length) {
	const value = readTarString(buffer, offset, length).trim();
	if (!value) return 0;
	const parsed = Number.parseInt(value, 8);
	if (!Number.isFinite(parsed)) {
		throw new Error(`Invalid tar octal value: ${value}`);
	}
	return parsed;
}

function isZeroTarBlock(buffer, offset) {
	for (let index = 0; index < 512; index++) {
		if (buffer[offset + index] !== 0) return false;
	}
	return true;
}

function getTarEntryName(header) {
	const name = readTarString(header, 0, 100);
	const prefix = readTarString(header, 345, 155);
	return prefix ? `${prefix}/${name}` : name;
}

function isSafeEmbeddedAddonFilename(filename) {
	return filename.length > 0 && path.basename(filename) === filename && !filename.includes("/") && !filename.includes("\\");
}

function isEmbeddedAddonFileCurrent(targetPath, file) {
	try {
		const stat = fs.statSync(targetPath);
		if (!stat.isFile()) return false;
		return typeof file.size !== "number" || stat.size === file.size;
	} catch (err) {
		if (err && err.code === "ENOENT") return false;
		throw err;
	}
}

function writeEmbeddedAddonFile(targetPath, content) {
	const tempPath = `${targetPath}.tmp.${process.pid}.${Date.now()}`;
	try {
		fs.writeFileSync(tempPath, content, { mode: 0o755 });
		fs.renameSync(tempPath, targetPath);
	} catch (err) {
		try {
			fs.unlinkSync(tempPath);
		} catch {
			// Best-effort cleanup only.
		}
		throw err;
	}
}

export function extractEmbeddedAddonArchive({ archivePath, files, targetDir }) {
	const pending = new Map();
	for (const file of files) {
		if (!isSafeEmbeddedAddonFilename(file.filename)) {
			throw new Error(`Unsafe embedded addon filename: ${file.filename}`);
		}
		const targetPath = path.join(targetDir, file.filename);
		if (!isEmbeddedAddonFileCurrent(targetPath, file)) {
			pending.set(file.filename, file);
		}
	}
	if (pending.size === 0) return [];

	const archive = zlib.gunzipSync(fs.readFileSync(archivePath));
	const writtenPaths = [];
	let offset = 0;

	while (offset + 512 <= archive.length) {
		if (isZeroTarBlock(archive, offset)) break;
		const header = archive.subarray(offset, offset + 512);
		const filename = getTarEntryName(header);
		const size = readTarOctal(header, 124, 12);
		const typeflag = header[156] === 0 ? "0" : String.fromCharCode(header[156]);
		offset += 512;

		if (offset + size > archive.length) {
			throw new Error(`Truncated embedded addon archive entry: ${filename}`);
		}

		if (!isSafeEmbeddedAddonFilename(filename)) {
			throw new Error(`Unsafe embedded addon archive entry: ${filename}`);
		}
		if (typeflag !== "0") {
			throw new Error(`Unsupported embedded addon archive entry type ${typeflag}: ${filename}`);
		}

		const file = pending.get(filename);
		if (file) {
			if (typeof file.size === "number" && file.size !== size) {
				throw new Error(`Embedded addon size mismatch for ${filename}: expected ${file.size}, got ${size}`);
			}
			const targetPath = path.join(targetDir, filename);
			writeEmbeddedAddonFile(targetPath, archive.subarray(offset, offset + size));
			pending.delete(filename);
			writtenPaths.push(targetPath);
		}

		offset += Math.ceil(size / 512) * 512;
	}

	if (pending.size > 0) {
		throw new Error(`Embedded addon archive missing: ${[...pending.keys()].join(", ")}`);
	}

	return writtenPaths;
}

function maybeExtractEmbeddedAddon(ctx, errors) {
	if (!ctx.isCompiledBinary || !embeddedAddon) return null;
	if (embeddedAddon.platformTag !== ctx.platformTag || embeddedAddon.version !== ctx.packageVersion) return null;

	const selectedEmbeddedFile = selectEmbeddedAddonFile(ctx.selectedVariant);
	if (!selectedEmbeddedFile) return null;
	const targetPath = path.join(ctx.versionedDir, selectedEmbeddedFile.filename);

	startupMarker("native:extractEmbeddedAddon:start");
	try {
		prepareNativeVersionDir(ctx.versionedDir);
	} catch (err) {
		const message = err instanceof Error ? err.message : String(err);
		errors.push(`embedded addon dir: ${message}`);
		return null;
	}

	if (embeddedAddon.archive) {
		try {
			extractEmbeddedAddonArchive({
				archivePath: embeddedAddon.archive.filePath,
				files: embeddedAddon.files,
				targetDir: ctx.versionedDir,
			});
			if (isEmbeddedAddonFileCurrent(targetPath, selectedEmbeddedFile)) {
				return targetPath;
			}
			errors.push(`embedded addon archive (${embeddedAddon.archive.filename}): missing ${selectedEmbeddedFile.filename}`);
			return null;
		} catch (err) {
			const message = err instanceof Error ? err.message : String(err);
			errors.push(`embedded addon archive (${embeddedAddon.archive.filename}): ${message}`);
			return null;
		}
	}

	if (isEmbeddedAddonFileCurrent(targetPath, selectedEmbeddedFile)) {
		return targetPath;
	}
	if (!selectedEmbeddedFile.filePath) {
		errors.push(`embedded addon metadata missing file path for ${selectedEmbeddedFile.filename}`);
		return null;
	}

	try {
		const buffer = fs.readFileSync(selectedEmbeddedFile.filePath);
		fs.writeFileSync(targetPath, buffer);
		return targetPath;
	} catch (err) {
		const message = err instanceof Error ? err.message : String(err);
		errors.push(`embedded addon write (${selectedEmbeddedFile.filename}): ${message}`);
		return null;
	}
}

/**
 * Mirror `leafPackageDir ?? nativeDir` addon binaries to
 * `versionedDir/<filename>.node` on Windows installs so the running process
 * cache path, never on the `node_modules` copy that bun must overwrite on
 * update. No-op on non-Windows, in workspace dev, and for compiled binaries —
 * see `shouldStageNodeModulesAddon` for the gating rules.
 */
function maybeStageNodeModulesAddon(ctx, errors) {
	if (!ctx.stageFromNodeModules) return null;

	let stagedPath = null;
	for (const filename of ctx.addonFilenames) {
		const sourcePath = path.join(ctx.leafPackageDir ?? ctx.nativeDir, filename);
		const targetPath = path.join(ctx.versionedDir, filename);

		if (fs.existsSync(targetPath)) {
			stagedPath = stagedPath || targetPath;
			continue;
		}
		if (!fs.existsSync(sourcePath)) continue;

		try {
			prepareNativeVersionDir(ctx.versionedDir);
		} catch (err) {
			const message = err instanceof Error ? err.message : String(err);
			errors.push(`staged addon dir: ${message}`);
			continue;
		}

		try {
			// `copyFileSync` is atomic on Windows (CopyFileW) and avoids holding
			// two large buffers in JS for the read/write dance.
			fs.copyFileSync(sourcePath, targetPath);
			stagedPath = stagedPath || targetPath;
		} catch (err) {
			const message = err instanceof Error ? err.message : String(err);
			errors.push(`staged addon copy (${filename}): ${message}`);
		}
	}
	return stagedPath;
}


/**
 * Before version sentinels were exported, published native addons still shared
 * this stable core ABI. Let those on-disk addons bridge a package-version bump
 * when they expose the signature; keep every versioned addon and a current
 * on-disk file paired with resident old exports on the strict path below.
 */
function isCompatiblePreSentinelNativeAddon(bindings, diskHasExpectedSentinel) {
	if (diskHasExpectedSentinel) return false;
	if (Object.keys(bindings).some(key => /^__piNativesV[A-Za-z0-9_]+$/.test(key))) return false;
	return (
		typeof bindings.countTokens === "function" &&
		typeof bindings.executeShell === "function" &&
		typeof bindings.visibleWidth === "function" &&
		typeof bindings.DesktopSession === "function" &&
		typeof bindings.DesktopSession.prototype?.capture === "function" &&
		typeof bindings.DesktopSession.prototype?.execute === "function" &&
		typeof bindings.DesktopSession.prototype?.close === "function"
	);
}

export function validateLoadedBindings(ctx, bindings, candidate) {
	// In workspace dev (running out of `packages/natives/native/` rather than a
	// `node_modules` install or a compiled bundle) the local `.node` only gains
	// the renamed sentinel after `bun --cwd=packages/natives run build`. Skip
	// validation there so a stale post-pull dev tree boots while the rebuild
	// completes; install and compiled-binary paths still validate.
	if (ctx.isWorkspaceLoad) return;
	if (typeof bindings[ctx.versionSentinelExport] === "function") return;

	// The expected sentinel is missing. Distinguish two failure modes by the
	// sentinel the bindings DO carry:
	//   - disk stale: the `.node` on disk predates this loader (its own build);
	//     reinstalling re-syncs the file.
	//   - process stale: an in-place upgrade landed a new release on disk while
	//     this process still holds the previous addon generation resident in the
	//     dynamic-loader's native-module cache. `require` returns those old
	//     exports, which carry the PRIOR sentinel — disk is already consistent,
	//     so reinstall is a no-op and only restarting the process re-syncs.
	const residentSentinel = Object.keys(bindings).find(
		key => key !== ctx.versionSentinelExport && /^__piNativesV[A-Za-z0-9_]+$/.test(key),
	);
	// A prior sentinel alone cannot distinguish a resident old module from an
	// actually stale file: `require` returns the same exports in both cases.
	// The restart diagnosis is valid only when the selected file itself carries
	// the current sentinel; otherwise a restart would simply reload stale disk.
	let diskHasExpectedSentinel = false;
	try {
		diskHasExpectedSentinel = fs.readFileSync(candidate).includes(ctx.versionSentinelExport);
	} catch {
		// The successful require above normally guarantees readability. If the
		// file disappears concurrently, retain the safe reinstall diagnosis.
	}
	if (isCompatiblePreSentinelNativeAddon(bindings, diskHasExpectedSentinel)) return;
	if (residentSentinel && diskHasExpectedSentinel) {
		const residentVersion = residentSentinel.slice("__piNativesV".length).replace(/_/g, ".");
		throw new Error(
			`Loaded ${candidate}, which exposes the @harvest/pi-natives@${residentVersion} version ` +
				`sentinel \`${residentSentinel}\` but not the @${ctx.packageVersion} sentinel ` +
				`\`${ctx.versionSentinelExport}\` this loader expects. omp was upgraded to ` +
				`${ctx.packageVersion} while this session was running; the ${residentVersion} addon is ` +
				"still resident in this process. Disk is already consistent — restart omp to pick up " +
				`${ctx.packageVersion} (reinstalling changes nothing).`,
		);
	}
	throw new Error(
		`Loaded ${candidate} but it does not expose the @harvest/pi-natives@${ctx.packageVersion} ` +
			`version sentinel \`${ctx.versionSentinelExport}\`. The .node file on disk is from a different ` +
			"release than this loader — reinstall to re-sync.",
	);
}

/**
 * Install the addon's bounded Tokio runtime now that `dlopen` has returned and
 * the dynamic-loader lock is released. The Rust `#[module_init]` deliberately
 * does NOT build the runtime — spawning worker threads under the loader lock
 * deadlocks on some hosts — so it exposes `__ompInstallTokioRuntime` for the
 * loader to call once, before any async native runs. Best-effort: older addons
 * predating this export simply fall back to napi-rs's default runtime.
 */
function installNativeTokioRuntime(bindings) {
	const install = bindings.__ompInstallTokioRuntime;
	if (typeof install !== "function") return;
	try {
		install();
		startupMarker("native:tokioRuntime:installed");
	} catch (err) {
		startupMarker(`native:tokioRuntime:failed:${err instanceof Error ? err.message : String(err)}`);
	}
}


function buildHelpMessage(ctx) {
	if (ctx.isCompiledBinary) {
		const expectedPaths = ctx.addonFilenames.map(filename => `  ${path.join(ctx.versionedDir, filename)}`).join("\n");
		const downloadHints = ctx.addonFilenames
			.map(filename => {
				const downloadUrl = `https://github.com/harvest/harvest/releases/latest/download/${filename}`;
				const targetPath = path.join(ctx.versionedDir, filename);
				return `  curl -fsSL "${downloadUrl}" -o "${targetPath}"`;
			})
			.join("\n");
		return (
			`The compiled binary should extract one of:\n${expectedPaths}\n\n` +
			`If missing, delete ${ctx.versionedDir} and re-run, or download manually:\n${downloadHints}`
		);
	}
	return (
		"If installed via npm/bun, try reinstalling: bun install @harvest/pi-natives\n" +
		"If developing locally, build with: bun --cwd=packages/natives run build\n" +
		"Explicit targets: bun scripts/bazel-natives.ts <target> --dest packages/natives/native"
	);
}

/**
 * Initialize the loader context: resolves every path, variant, and policy
 * decision once so the inner load loop stays a pure require/validate pipeline.
 * Called from `loadNative()` rather than at module scope so importing pure
 * helpers from this file doesn't trigger AVX2 detection or filesystem probes.
 */
/**
 * @param {{ nativeDir?: string; platform?: NodeJS.Platform | string; isCompiledBinary?: boolean; leafPackageDir?: string | null }} [overrides]
 */
export function initLoaderContext(overrides = {}) {
	const platform = overrides.platform ?? process.platform;
	const platformTag = `${platform}-${process.arch}`;
	const packageVersion = packageJson.version;
	const nativeDir = overrides.nativeDir ?? path.join(import.meta.dir, "..", "native");
	const execDir = path.dirname(process.execPath);
	const nativesDir = getNativesDir();
	const versionedDir = path.join(nativesDir, packageVersion);
	const userDataDir =
		platform === "win32"
			? path.join(process.env.LOCALAPPDATA || path.join(os.homedir(), "AppData", "Local"), "omp")
			: path.join(os.homedir(), ".local", "bin");

	const isCompiledBinary =
		overrides.isCompiledBinary ??
		detectCompiledBinary({
			embeddedAddon,
			env: process.env,
			importMetaUrl: import.meta.url,
		});
	const normalizedNativeDir = platform === "win32" ? nativeDir.toLowerCase() : nativeDir;
	const isWorkspaceLoad =
		!isCompiledBinary &&
		!normalizedNativeDir.includes("\\node_modules\\") &&
		!normalizedNativeDir.includes("/node_modules/");
	const leafPackageDir =
		isCompiledBinary || isWorkspaceLoad
			? null
			: overrides.leafPackageDir === undefined
				? resolveLeafPackageDir(platformTag)
				: overrides.leafPackageDir;
	const stageFromNodeModules = shouldStageNodeModulesAddon({
		platform,
		isCompiledBinary,
		nativeDir: normalizedNativeDir,
	});

	const selectedVariant = resolveCpuVariant(getVariantOverride());
	const addonFilenames = getAddonFilenames({ tag: platformTag, arch: process.arch, variant: selectedVariant });
	const addonLabel = selectedVariant ? `${platformTag} (${selectedVariant})` : platformTag;

	const candidates = resolveLoaderCandidates({
		addonFilenames,
		isCompiledBinary,
		stageFromNodeModules,
		nativeDir,
		leafPackageDir,
		execDir,
		versionedDir,
		userDataDir,
	});

	// Version sentinel emitted by the Rust addon under a `js_name` that encodes
	// the package version (`__piNativesV{major}_{minor}_{patch}`).
	// `scripts/release.ts` bumps the name in `crates/pi-natives/src/lib.rs` in
	// lock-step with the version, so a `.node` from a different release
	// physically cannot expose the symbol this loader is looking for. That
	// turns the silent `<sym> is not a function` crash from a Windows
	// locked-file update into an actionable load-time error.
	const versionSentinelExport = `__piNativesV${packageVersion.replace(/[^A-Za-z0-9]/g, "_")}`;

	return {
		platformTag,
		packageVersion,
		nativeDir,
		leafPackageDir,
		versionedDir,
		isCompiledBinary,
		stageFromNodeModules,
		selectedVariant,
		addonFilenames,
		addonLabel,
		candidates,
		versionSentinelExport,
		isWorkspaceLoad,
		nativesDir,
	};
}

export function loadNative() {
	startupMarker("native:loadNative:start");
	const ctx = initLoaderContext();
	const require_ = createRequire(import.meta.url);

	const errors = [];
	const embeddedCandidate = maybeExtractEmbeddedAddon(ctx, errors);
	const stagedCandidate = embeddedCandidate ? null : maybeStageNodeModulesAddon(ctx, errors);
	const prepended = [embeddedCandidate, stagedCandidate].filter(c => typeof c === "string");
	const runtimeCandidates = prepended.length > 0 ? [...prepended, ...ctx.candidates] : ctx.candidates;

	for (const candidate of runtimeCandidates) {
		try {
			startupMarker(`native:require:${path.basename(candidate)}`);
			const bindings = require_(candidate);
			validateLoadedBindings(ctx, bindings, candidate);
			installNativeTokioRuntime(bindings);
	        cleanupStaleNativeVersions({ nativesDir: ctx.nativesDir, currentVersion: ctx.packageVersion });
			startupMarker("native:loadNative:done");
			return bindings;
		} catch (err) {
			const message = err instanceof Error ? err.message : String(err);
			errors.push(`${candidate}: ${message}`);
		}
	}

	// If native addon is unavailable (e.g. host without precompiled Rust binaries),
	// provide fallback bindings to prevent CLI boot crash.
	// Text/width helpers use Bun's built-in ANSI-aware APIs as a JS substitute.
	return createFallbackBindings();
}

/** Create the JavaScript bindings without probing or loading a native addon. */
export function createFallbackBindings() {
	// Strip ANSI escape codes for width measurement
	const ANSI_ESCAPE_RE = /\x1b(?:\[[0-?]*[ -/]*[@-~]|\][^\x07\x1b]*(?:\x07|\x1b\\)|[_PX^][\s\S]*?(?:\x07|\x1b\\)|[@-Z\\-_])/g;
	function stripAnsi(str) {
		return str.replace(ANSI_ESCAPE_RE, "");
	}
	function visibleLen(str, tabWidth = 3) {
		if (str.includes("\x1b]66;") || str.includes("\t") || /\x1b[_PX^]/.test(str)) {
			let width = 0;
			for (const token of tokenizeLine(str, tabWidth)) width += token.width;
			return width;
		}
		// Bun.stringWidth strips ANSI escapes natively and handles wide chars
		return typeof Bun !== "undefined" && typeof Bun.stringWidth === "function"
			? Bun.stringWidth(str, { countAnsiEscapeCodes: false })
			: stripAnsi(str).length;
	}

	// JS fallback for wrapTextWithAnsi: splits text at width boundaries,
	// preserving ANSI escape codes by re-opening them on continuation lines.
	function jsWrapTextWithAnsi(text, width, _tabWidth) {
		if (!text || width <= 0) return text === "" ? [""] : [];
		// Normalise newlines
		const paragraphs = String(text).split("\n");
		const result = [];
		for (const para of paragraphs) {
			if (visibleLen(para) <= width) {
				result.push(para);
				continue;
			}
			// Naive split: break on word boundaries, respecting visible width
			const words = para.split(" ");
			let currentLine = "";
			let currentWidth = 0;
			for (const word of words) {
				const wordWidth = visibleLen(word);
				if (currentWidth === 0) {
					currentLine = word;
					currentWidth = wordWidth;
				} else if (currentWidth + 1 + wordWidth <= width) {
					currentLine += " " + word;
					currentWidth += 1 + wordWidth;
				} else {
					result.push(currentLine);
					currentLine = word;
					currentWidth = wordWidth;
				}
			}
			if (currentLine.length > 0 || result.length === 0) {
				result.push(currentLine);
			}
		}
		return result.length === 0 ? [""] : result;
	}

	// JS fallback helpers for ANSI / Unicode terminal text operations
	function parseOsc66Info(seq, tabWidth) {
		const m = /^\x1b\]66;([^;]*);([\s\S]*?)(?:\x07|\x1b\\)$/.exec(seq);
		if (!m) return null;
		let scale = 1;
		let explicitWidth = null;
		for (const part of m[1].split(":")) {
			if (part.indexOf("=") !== 1) continue;
			const val = Number.parseInt(part.slice(2), 10);
			if (!Number.isFinite(val)) continue;
			if (part[0] === "s" && val >= 1 && val <= 7) scale = val;
			else if (part[0] === "w" && val > 0) explicitWidth = val;
		}
		const baseW = explicitWidth ?? visibleLen(m[2], tabWidth);
		return { meta: m[1], payload: m[2], scale, width: scale * baseW };
	}

	const graphemeSegmenter = new Intl.Segmenter(undefined, { granularity: "grapheme" });
	function* tokenizeLine(text, tabWidth = 3) {
		const str = String(text ?? "");
		let skipUntil = 0;
		for (const { index: i, segment } of graphemeSegmenter.segment(str)) {
			if (i < skipUntil) continue;
			if (str.charCodeAt(i) === 0x1b) {
				const sub = str.slice(i);
				const osc66Match = /^\x1b\]66;[\s\S]*?(?:\x07|\x1b\\)/.exec(sub);
				if (osc66Match) {
					const raw = osc66Match[0];
					const info = parseOsc66Info(raw, tabWidth);
					yield { type: "osc66", raw, info, width: info ? info.width : 0 };
					skipUntil = i + raw.length;
					continue;
				}
				// Match complete protocol sequences before two-byte ESC commands.
				// Otherwise ESC ] / ESC _ consume only the opening bytes.
				const ansiMatch = /^\x1b(?:\[[0-?]*[ -/]*[@-~]|\][^\x07\x1b]*(?:\x07|\x1b\\)|[_PX^][\s\S]*?(?:\x07|\x1b\\)|[@-Z\\-_])/.exec(sub);
				if (ansiMatch) {
					const raw = ansiMatch[0];
					const isApc = raw.startsWith("\x1b_");
					const hyperlink = /^\x1b\]8;[^;]*;([\s\S]*?)(?:\x07|\x1b\\)$/.exec(raw);
					yield {
						type: "ansi",
						raw,
						width: 0,
						isApc,
						isSgr: raw.startsWith("\x1b[") && raw.endsWith("m"),
						hyperlinkUri: hyperlink?.[1],
					};
					skipUntil = i + raw.length;
					continue;
				}
			}
			if (segment === "\t") {
				yield { type: "char", raw: "\t", width: tabWidth };
				continue;
			}
			yield { type: "char", raw: segment, width: visibleLen(segment) };
		}
	}

	// JS fallback for truncateToWidth: returns string
	function jsTruncateToWidth(text, width, ellipsisKind, pad, tabWidth = 3) {
		const str = String(text ?? "");
		const maxW = Math.max(0, width | 0);
		if (maxW === 0) return "";
		const ellipsis = ellipsisKind === 2 || ellipsisKind === "" || ellipsisKind === "omit"
			? ""
			: (ellipsisKind === 1 || ellipsisKind === "ascii" ? "..." : "…");
		const ell = ellipsis.slice(0, maxW);
		const ellW = visibleLen(ell);
		const textWidth = visibleLen(str, tabWidth);
		if (textWidth <= maxW) {
			if (pad && textWidth < maxW) {
				return str + " ".repeat(maxW - textWidth);
			}
			return str;
		}

		const targetW = Math.max(0, maxW - ellW);
		const tokens = tokenizeLine(str, tabWidth);
		let currentW = 0;
		let out = "";
		let sawSgr = false;
		let hyperlinkOpen = false;

		for (const tok of tokens) {
			if (tok.type === "ansi") {
				out += tok.raw;
				if (tok.isSgr) sawSgr = true;
				if (tok.hyperlinkUri !== undefined) hyperlinkOpen = tok.hyperlinkUri !== "";
				continue;
			}
			if (tok.type === "osc66") {
				if (currentW + tok.width <= targetW) {
					out += tok.raw;
					currentW += tok.width;
				} else {
					const partial = jsSliceWithWidth(tok.info?.payload ?? "", 0, targetW - currentW, true, tabWidth);
					out += partial.text;
					currentW += partial.width;
					break;
				}
				continue;
			}
			if (currentW + tok.width > targetW) {
				break;
			}
			out += tok.raw;
			currentW += tok.width;
		}

		if (hyperlinkOpen) out += "\x1b]8;;\x1b\\";
		if (sawSgr) {
			out += "\x1b[0m";
		}
		out += ell;
		if (pad && currentW + ellW < maxW) {
			out += " ".repeat(maxW - (currentW + ellW));
		}
		return out;
	}

	// JS fallback for sliceWithWidth: returns { text: string, width: number }
	function jsSliceWithWidth(text, startCol, length, strict, tabWidth = 3) {
		const str = String(text ?? "");
		const start = Math.max(0, startCol | 0);
		const len = Math.max(0, length | 0);
		if (len === 0) return { text: "", width: 0 };
		const endCol = start + len;

		const tokens = tokenizeLine(str, tabWidth);
		let col = 0;
		let out = "";
		let outW = 0;
		let pendingAnsi = "";

		for (const tok of tokens) {
			if (tok.type === "ansi") {
				if (col >= start) {
					out += tok.raw;
				} else {
					pendingAnsi += tok.raw;
				}
				continue;
			}

			const nextCol = col + tok.width;
			if (tok.type === "osc66") {
				if (col >= start && nextCol <= endCol) {
					out += pendingAnsi + tok.raw;
					pendingAnsi = "";
					outW += tok.width;
				} else if (col < endCol && nextCol > start) {
					// Overlapping OSC 66 span
					const payload = tok.info ? tok.info.payload : "";
					const scale = tok.info ? tok.info.scale : 1;
					const overlapStart = Math.max(0, start - col);
					const overlapEnd = Math.min(nextCol, endCol) - col;
					const pStart = strict ? Math.ceil(overlapStart / scale) : Math.floor(overlapStart / scale);
					const pEnd = strict ? Math.floor(overlapEnd / scale) : Math.ceil(overlapEnd / scale);
					const sliced = jsSliceWithWidth(payload, pStart, Math.max(0, pEnd - pStart), strict, tabWidth);
					out += pendingAnsi + sliced.text;
					pendingAnsi = "";
					outW += sliced.width;
				}
				col = nextCol;
				continue;
			}

			if (col >= endCol) break;

			const inRange = col >= start;
			const fits = !strict || nextCol <= endCol;

			if (inRange && fits) {
				out += pendingAnsi + tok.raw;
				pendingAnsi = "";
				outW += tok.width;
			}
			col = nextCol;
		}

		return { text: out, width: outW };
	}

	// JS fallback for extractSegments: returns { before, beforeWidth, after, afterWidth }
	function jsExtractSegments(line, beforeEnd, afterStart, afterLen, strictAfter, tabWidth = 3) {
		const str = String(line ?? "");
		const before = jsSliceWithWidth(str, 0, beforeEnd, false, tabWidth);
		const after = jsSliceWithWidth(str, afterStart, afterLen, strictAfter, tabWidth);
		return { before: before.text, beforeWidth: before.width, after: after.text, afterWidth: after.width };
	}

	// Edit mode descriptions (inlined from crates/pi-edit/prompts/)
	const EDIT_DESCRIPTIONS = {
		replace: `Single file string replacement; fuzzy whitespace matching.\n\n<instruction>\n- MUST use smallest \`old_string\` uniquely identifying change.\n- Nonunique \`old_string\` → MUST add context or use \`replace_all: true\` for all occurrences.\n- Rename a string across file → use \`replace_all: true\`.\n- SHOULD edit existing files, not create new.\n</instruction>\n\n<output>\nSuccess/failure status.\nSuccess: file modified in place; replacement applied.\nFailure — e.g., \`old_string\` absent or multiple matches without \`replace_all: true\`: error describes issue.\n</output>\n\n<critical>\n- MUST read file at least once in conversation before editing. Tool errors on edit before read.\n</critical>`,
		patch: `Patches files given diff hunks. Primary tool for existing-file edits.\n\n<instruction>\n**Hunk Headers:**\n- \`@@\` — bare header when context lines unique\n- \`@@ $ANCHOR\` — anchor copied verbatim from file (full line or unique substring)\n**Context Lines:**\nUse enough lines to make match unique (usually 2-8)\n</instruction>\n\n<critical>\n- You MUST read the target file before editing\n- You MUST copy anchors and context lines verbatim (including whitespace)\n- If edit fails, re-read the file and produce a new patch from current content\n</critical>`,
		hashline: `Line-anchored patch language: name original lines/gaps to replace, insert, cut, or paste.\n\n<critical>\n1. RE-GROUND AFTER EVERY EDIT: edits renumber and change #TAG.\n2. RANGES TIGHT: changed lines only.\n3. BODY FINAL CONTENT: every row starts +.\n</critical>`,
		apply_patch: `Edit files: apply_patch shell command.\n\n\`apply_patch\`: stripped-down, file-oriented diff.\n\n<critical>\nMUST use Add/Delete/Update header; new-file lines MUST start +; file references relative, NEVER absolute.\n</critical>`,
		sloppy: `Anchored edit format: quote current text in <SM:FIND>, state final text in <SM:PUT>, elide unchanged runs with ….\n\n<critical>\n1. First line is <SM:EDIT path="relative/path.ts">.\n2. Content between tags is RAW: NEVER XML-escape <, >, & — write file bytes exactly.\n</critical>`,
	};

	// Pure JS Git discovery and fallback repo implementation for environments
	// where native compiled Rust addon is not present.
	function findGitRepo(startDir) {
		if (!startDir || typeof startDir !== "string") return null;
		let current = path.resolve(startDir);
		while (true) {
			const gitPath = path.join(current, ".git");
			try {
				const stat = fs.statSync(gitPath);
				if (stat.isDirectory()) {
					return {
						repoRoot: current,
						gitDir: gitPath,
						commonDir: gitPath,
						primaryRoot: current,
						isLinkedWorktree: false,
						worktreeRoot: null,
					};
				}
				if (stat.isFile()) {
					const content = fs.readFileSync(gitPath, "utf8").trim();
					const match = /^gitdir:\s*(.+)$/im.exec(content);
					if (match) {
						const rawGitDir = match[1].trim();
						const gitDir = path.resolve(current, rawGitDir);
						let commonDir = gitDir;
						let primaryRoot = current;
						const commondirFile = path.join(gitDir, "commondir");
						try {
							const commonRel = fs.readFileSync(commondirFile, "utf8").trim();
							commonDir = path.resolve(gitDir, commonRel);
							primaryRoot = path.dirname(commonDir);
						} catch {}
						return {
							repoRoot: current,
							gitDir,
							commonDir,
							primaryRoot,
							isLinkedWorktree: true,
							worktreeRoot: current,
						};
					}
				}
			} catch {}
			const parent = path.dirname(current);
			if (parent === current) break;
			current = parent;
		}
		return null;
	}

	function findJjRepo(startDir) {
		if (!startDir || typeof startDir !== "string") return null;
		let current = path.resolve(startDir);
		while (true) {
			const jjRepo = path.join(current, ".jj", "repo");
			const jjDir = path.join(current, ".jj");
			try {
				if (fs.existsSync(jjRepo) || fs.existsSync(jjDir)) {
					return {
						root: current,
						storeDir: jjDir,
					};
				}
			} catch {}
			const parent = path.dirname(current);
			if (parent === current) break;
			current = parent;
		}
		return null;
	}

	class FallbackVcsGitRepo {
		#data;
		constructor(data) {
			this.#data = data;
		}
		info() {
			return {
				repoRoot: this.#data.repoRoot,
				gitEntryPath: this.#data.gitDir,
				gitDir: this.#data.gitDir,
				commonDir: this.#data.commonDir,
				headPath: path.join(this.#data.gitDir, "HEAD"),
				isReftable: false,
				kind: "git",
			};
		}
		primaryRoot() {
			return this.#data.primaryRoot;
		}
		root() {
			return this.#data.repoRoot;
		}
		linkedWorktree() {
			if (!this.#data.isLinkedWorktree) return null;
			return {
				root: this.#data.worktreeRoot,
				primaryRoot: this.#data.primaryRoot,
			};
		}
		prefixOf(dir) {
			const rel = path.relative(this.#data.repoRoot, dir);
			return rel.startsWith("..") ? null : rel;
		}
		watchTarget() {
			return path.join(this.#data.gitDir, "HEAD");
		}
		headSync() {
			try {
				const headContent = fs.readFileSync(path.join(this.#data.gitDir, "HEAD"), "utf8").trim();
				if (headContent.startsWith("ref:")) {
					const ref = headContent.slice(4).trim();
					const name = ref.replace(/^refs\/heads\//, "");
					return {
						kind: "ref",
						branch: name,
						refName: ref,
						commit: "",
						detached: false,
						name,
						sha: "",
					};
				}
				return {
					kind: "detached",
					branch: undefined,
					refName: undefined,
					commit: headContent,
					detached: true,
					name: "HEAD",
					sha: headContent,
				};
			} catch {
				return {
					kind: "ref",
					branch: "main",
					refName: "refs/heads/main",
					commit: "",
					detached: false,
					name: "main",
					sha: "",
				};
			}
		}
		async head() {
			return this.headSync();
		}
		async headSha() {
			const h = this.headSync();
			if (h.detached && h.sha) return h.sha;
			try {
				const headContent = fs.readFileSync(path.join(this.#data.gitDir, "HEAD"), "utf8").trim();
				if (headContent.startsWith("ref:")) {
					const ref = headContent.slice(4).trim();
					const refPath = path.join(this.#data.commonDir, ref);
					return fs.readFileSync(refPath, "utf8").trim();
				}
				return headContent;
			} catch {
				return null;
			}
		}
		async currentBranch() {
			const h = this.headSync();
			return h.detached ? null : h.name;
		}
		async defaultBranch() {
			return "main";
		}
		async resolveRef() {
			return null;
		}
		async refExists() {
			return false;
		}
		async tagsAt() {
			return [];
		}
		async listBranches() {
			return [];
		}
		async isDirty() {
			return false;
		}
		async statusPorcelain() {
			return "";
		}
		async statusSummary() {
			return { staged: 0, unstaged: 0, untracked: 0, conflicted: 0 };
		}
		async configGet() {
			return null;
		}
		async configSet() {
			return undefined;
		}
		async remoteList() {
			return [];
		}
		async remoteUrl() {
			return null;
		}
		async remoteAdd() {
			return undefined;
		}
		async worktrees() {
			return [];
		}
		async worktreeAdd() {
			throw new Error("worktreeAdd not supported in fallback mode");
		}
		async worktreeRemove() {
			return false;
		}
		async worktreePrune() {
			return undefined;
		}
		async logSubjects() {
			return [];
		}
		async logOnelines() {
			return [];
		}
		async revListRange() {
			return [];
		}
		async mergeBase() {
			return null;
		}
		async revListTouching() {
			return [];
		}
		async commitDetails() {
			return { subject: "", author: "", date: "", hash: "", body: "" };
		}
		async lsFiles() {
			return [];
		}
		async lsTree() {
			return [];
		}
		async changedFiles() {
			return [];
		}
		async commitCreate() {
			throw new Error("commitCreate not supported in fallback mode");
		}
		async push() {
			throw new Error("push not supported in fallback mode");
		}
		async showCommit() {
			return "";
		}
		async diffTree() {
			return "";
		}
		async diffText() {
			return "";
		}
		async diffNoIndex() {
			return "";
		}
		async numstat() {
			return [];
		}
		async hasDiff() {
			return false;
		}
		async stageFiles() {}
		async unstage() {}
		async stageHunks() {}
		async checkout() {}
		async createBranch() {}
		async deleteBranch() {
			return false;
		}
		async checkoutNewBranch() {}
		async restore() {}
		async reset() {}
		async clean() {}
		async readTree() {}
		async writeTree() {
			return "";
		}
		async applyPatch() {}
		async canApplyPatch() {
			return false;
		}
		async cherryPick() {
			const err = new Error("cherryPick not supported in fallback mode");
			err.name = "VcsError";
			err.code = "Unsupported";
			throw err;
		}
		async cherryPickAbort() {}
		async cherryPickSkip() {}
		async stashPush() {
			return false;
		}
		async stashTryPop() {
			return false;
		}
		async fetch() {}
		async submodulePaths() {
			return [];
		}
		async showBlob() {
			return { data: Buffer.from(""), truncated: false };
		}
		async lfsMediaDir() {
			return null;
		}
	}

	class FallbackVcsJjWorkspace {
		#data;
		constructor(data) {
			this.#data = data;
		}
		root() {
			return this.#data.root;
		}
		primaryRoot() {
			return this.#data.root;
		}
		prefixOf(dir) {
			const rel = path.relative(this.#data.root, dir);
			return rel.startsWith("..") ? null : rel;
		}
		storeDir() {
			return this.#data.storeDir;
		}
		watchTarget() {
			return path.join(this.#data.storeDir, "working_copy");
		}
		async workingCopyLabel() {
			return null;
		}
		async headId() {
			return null;
		}
		async statusSummary() {
			return { staged: 0, unstaged: 0, untracked: 0, conflicted: 0 };
		}
		async diffText() {
			return "";
		}
		async changedFiles() {
			return [];
		}
		async logSubjects() {
			return [];
		}
		async logOnelines() {
			return [];
		}
		async commitDetails() {
			return { subject: "", author: "", date: "", hash: "", body: "" };
		}
		async lsFiles() {
			return [];
		}
	}

	class FallbackVcsRepo {
		#kind;
		#git;
		#jj;
		constructor(data, kind = "git") {
			this.#kind = kind;
			if (kind === "git") {
				this.#git = new FallbackVcsGitRepo(data);
				this.#jj = null;
			} else {
				this.#git = null;
				this.#jj = new FallbackVcsJjWorkspace(data);
			}
		}
		kind() {
			return this.#kind;
		}
		root() {
			return this.#kind === "git" ? this.#git.root() : this.#jj.root();
		}
		primaryRoot() {
			return this.#kind === "git" ? this.#git.primaryRoot() : this.#jj.primaryRoot();
		}
		prefixOf(dir) {
			if (this.#kind === "git") return this.#git.prefixOf(dir);
			return this.#jj.prefixOf(dir);
		}
		watchTarget() {
			return this.#kind === "git" ? this.#git.watchTarget() : this.#jj.watchTarget();
		}
		supports(feature) {
			if (this.#kind === "git") {
				if (feature === "stagedDiff" || feature === "revDiff") return true;
			} else if (this.#kind === "jj") {
				if (feature === "stagedDiff" || feature === "revDiff") return false;
			}
			const err = new Error(`unknown feature \`${feature}\`; valid: stagedDiff, revDiff`);
			err.name = "VcsError";
			err.code = "Backend";
			throw err;
		}
		asGit() {
			return this.#git;
		}
		as_git() {
			return this.#git;
		}
		asJj() {
			return this.#jj;
		}
		as_jj() {
			return this.#jj;
		}
		async label(signal) {
			return this.#kind === "git" ? this.#git.currentBranch() : this.#jj.workingCopyLabel();
		}
		async headId(signal) {
			return this.#kind === "git" ? this.#git.headSha() : this.#jj.headId();
		}
		async head_id(signal) {
			return this.headId(signal);
		}
		async statusSummary(signal) {
			return this.#kind === "git" ? this.#git.statusSummary() : this.#jj.statusSummary();
		}
		async status_counts(signal) {
			return this.statusSummary(signal);
		}
		async statusPorcelain(options, signal) {
			return this.#kind === "git" ? this.#git.statusPorcelain(options) : "";
		}
		async diffText(options, signal) {
			return "";
		}
		async changedFiles(options, signal) {
			return this.#kind === "git" ? this.#git.changedFiles(options) : this.#jj.changedFiles();
		}
		async numstat(options, signal) {
			return [];
		}
		async uncommittedDiff(files, signal) {
			return "";
		}
		async logSubjects(count, signal) {
			return this.#kind === "git" ? this.#git.logSubjects(count) : this.#jj.logSubjects(count);
		}
		async logOnelines(count, signal) {
			return this.#kind === "git" ? this.#git.logOnelines(count) : this.#jj.logOnelines(count);
		}
		async commitDetails(rev, signal) {
			return this.#kind === "git" ? this.#git.commitDetails(rev) : this.#jj.commitDetails(rev);
		}
		async revListRange(base, head, limit, signal) {
			return this.#kind === "git" ? this.#git.revListRange(base, head, limit) : [];
		}
		async mergeBase(left, right, signal) {
			return this.#kind === "git" ? this.#git.mergeBase(left, right) : null;
		}
		async listFiles(patterns, signal) {
			return this.#kind === "git" ? this.#git.lsFiles() : [];
		}
		async lsFiles(others, excludeStandard, signal) {
			return this.#kind === "git" ? this.#git.lsFiles(others, excludeStandard) : [];
		}
		async commitCreate(message, signal) {
			if (this.#kind === "git") return this.#git.commitCreate(message);
			throw new Error("commitCreate not supported in fallback mode");
		}
		async push(remote, branch, signal) {
			if (this.#kind === "git") return this.#git.push(remote, branch);
			throw new Error("push not supported in fallback mode");
		}
	}

	// Fallback FileLock implementation using fs.openSync with exclusive flag ('wx')
	// or in-process lock tracker for advisory file serialization.
	const activeFileLocks = new Map();
	class FallbackFileLock {
		#path;
		#fd;
		#acquired;

		constructor(path, fd) {
			this.#path = path;
			this.#fd = fd;
			this.#acquired = fd !== null;
		}

		get acquired() {
			return this.#acquired;
		}

		release() {
			if (!this.#acquired) return;
			this.#acquired = false;
			activeFileLocks.delete(this.#path);
			if (this.#fd !== null) {
				try {
					fs.closeSync(this.#fd);
				} catch {}
				try {
					fs.unlinkSync(this.#path);
				} catch {}
				this.#fd = null;
			}
		}

		static tryAcquire(lockPath) {
			const resolved = path.resolve(String(lockPath));
			if (activeFileLocks.has(resolved)) {
				return new FallbackFileLock(resolved, null);
			}
			const dir = path.dirname(resolved);
			if (!fs.existsSync(dir)) {
				try {
					fs.mkdirSync(dir, { recursive: true });
				} catch {}
			}

			// Check if existing lock file is stale (process died)
			try {
				if (fs.existsSync(resolved)) {
					const content = fs.readFileSync(resolved, "utf8").trim();
					const pid = Number.parseInt(content, 10);
					if (Number.isFinite(pid)) {
						let isAlive = false;
						try {
							process.kill(pid, 0);
							isAlive = true;
						} catch (e) {
							isAlive = false;
						}
						if (!isAlive) {
							// Holder process is dead; unlink stale lock
							try {
								fs.unlinkSync(resolved);
							} catch {}
						}
					}
				}
			} catch {}

			try {
				const fd = fs.openSync(resolved, "wx");
				try {
					fs.writeSync(fd, `${process.pid}\n`);
				} catch {}
				activeFileLocks.set(resolved, fd);
				return new FallbackFileLock(resolved, fd);
			} catch (err) {
				// EEXIST or lock contention
				return new FallbackFileLock(resolved, null);
			}
		}
	}

	class FallbackShell {
		#cwd;
		#child = null;

		constructor(options) {
			this.#cwd = options?.cwd ? path.resolve(options.cwd) : process.cwd();
		}

		async abort() {
			if (this.#child) {
				try {
					this.#child.kill();
				} catch {}
				this.#child = null;
			}
		}

		async liveBackgroundJobCount() {
			return 0;
		}

		run(options, onChunk) {
			const { command, cwd, env, timeoutMs, signal } = options || {};
			const runCwd = cwd ? path.resolve(this.#cwd, cwd) : this.#cwd;

			if (signal?.aborted) {
				return Promise.resolve({
					exitCode: undefined,
					cancelled: true,
					timedOut: false,
					workingDir: this.#cwd,
				});
			}

			let shellPath = "sh";
			let isBash = false;
			if (process.platform === "win32") {
				const gitRoots = [
					process.env.ProgramFiles && path.join(process.env.ProgramFiles, "Git"),
					process.env["ProgramFiles(x86)"] && path.join(process.env["ProgramFiles(x86)"], "Git"),
					process.env.LOCALAPPDATA && path.join(process.env.LOCALAPPDATA, "Programs", "Git"),
				];
				for (const root of gitRoots) {
					if (!root) continue;
					const candidate = path.join(root, "bin", "bash.exe");
					try {
						if (fs.existsSync(candidate)) {
							shellPath = candidate;
							isBash = true;
							break;
						}
					} catch {}
				}
				if (!isBash) {
					shellPath = process.env.ComSpec || "cmd.exe";
				}
			} else {
				shellPath = "/bin/bash";
				isBash = true;
			}

			let cmdToRun = String(command ?? "");
			if (isBash && process.platform === "win32") {
				cmdToRun = cmdToRun.replace(/([a-zA-Z]:)\\[^"'\s]*/g, (m) => m.replace(/\\/g, "/"));
			}

			const runId = `${process.pid}-${Date.now()}-${Math.random().toString(36).slice(2)}`;
			const cwdMarkerFile = path.join(os.tmpdir(), `omp-shell-cwd-${runId}.txt`);
			const scriptFile = path.join(os.tmpdir(), `omp-shell-run-${runId}.sh`);

			let spawnArgs = [];
			if (isBash) {
				const preamble = process.platform === "win32"
					? 'pwd() { if [ "$#" -eq 0 ]; then cygpath -w "$(builtin pwd)" 2>/dev/null || (builtin pwd -W 2>/dev/null || builtin pwd); else builtin pwd "$@"; fi; }; export -f pwd 2>/dev/null || true;\n'
					: "";
				const scriptContent = `${preamble}${cmdToRun}\n__omp_ec=$?\n(pwd -W 2>/dev/null || pwd) > "${cwdMarkerFile.replace(/\\/g, "/")}"\nexit $__omp_ec\n`;
				fs.writeFileSync(scriptFile, scriptContent);
				spawnArgs = [scriptFile];
			} else {
				spawnArgs = ["/c", cmdToRun];
			}

			const mergedEnv = { ...process.env, ...(env || {}) };

			return new Promise((resolve) => {
				let timer = null;
				let finished = false;

				let child;
				try {
					child = childProcess.spawn(shellPath, spawnArgs, {
						cwd: runCwd,
						env: mergedEnv,
						stdio: ["pipe", "pipe", "pipe"],
						windowsHide: true,
					});
				} catch {
					try { fs.unlinkSync(scriptFile); } catch {}
					try { fs.unlinkSync(cwdMarkerFile); } catch {}
					resolve({
						exitCode: 1,
						cancelled: false,
						timedOut: false,
						workingDir: this.#cwd,
					});
					return;
				}
				this.#child = child;

				const cleanup = () => {
					if (timer) clearTimeout(timer);
					try { fs.unlinkSync(scriptFile); } catch {}
					this.#child = null;
				};

				if (timeoutMs && timeoutMs > 0) {
					timer = setTimeout(() => {
						if (finished) return;
						finished = true;
						try { child.kill(); } catch {}
						cleanup();
						try { fs.unlinkSync(cwdMarkerFile); } catch {}
						resolve({
							exitCode: undefined,
							cancelled: true,
							timedOut: true,
							workingDir: this.#cwd,
						});
					}, timeoutMs);
				}

				const abortListener = () => {
					if (finished) return;
					finished = true;
					try { child.kill(); } catch {}
					cleanup();
					try { fs.unlinkSync(cwdMarkerFile); } catch {}
					resolve({
						exitCode: undefined,
						cancelled: true,
						timedOut: false,
						workingDir: this.#cwd,
					});
				};

				if (signal) {
					signal.addEventListener("abort", abortListener, { once: true });
				}

				child.stdout?.on("data", (buf) => {
					onChunk?.(null, buf.toString());
				});

				child.stderr?.on("data", (buf) => {
					onChunk?.(null, buf.toString());
				});

				child.on("error", (err) => {
					if (finished) return;
					finished = true;
					cleanup();
					try { fs.unlinkSync(cwdMarkerFile); } catch {}
					onChunk?.(err, "");
					resolve({
						exitCode: 1,
						cancelled: false,
						timedOut: false,
						workingDir: this.#cwd,
					});
				});

				child.on("close", (code) => {
					if (finished) return;
					finished = true;
					cleanup();

					let newCwd = this.#cwd;
					if (fs.existsSync(cwdMarkerFile)) {
						try {
							const captured = fs.readFileSync(cwdMarkerFile, "utf8").trim();
							if (captured) {
								newCwd = path.resolve(captured);
								this.#cwd = newCwd;
							}
						} catch {}
						try { fs.unlinkSync(cwdMarkerFile); } catch {}
					}

					resolve({
						exitCode: code ?? 0,
						cancelled: false,
						timedOut: false,
						workingDir: newCwd,
					});
				});
			});
		}
	}

	function fallbackGlobMatches(pattern, relPath) {
		if (pattern === "**/*" || pattern === "*") return true;
		let p = pattern;
		if (p.startsWith("**/")) {
			p = p.slice(3);
			const subRegex = `^(?:.*\\/)?${p
				.replace(/[.+^${}()|[\]\\]/g, "\\$&")
				.replace(/\*\*/g, ".*")
				.replace(/\*/g, "[^/]*")}$`;
			try {
				return new RegExp(subRegex).test(relPath);
			} catch {
				return false;
			}
		}
		const regexStr = `^${p
			.replace(/[.+^${}()|[\]\\]/g, "\\$&")
			.replace(/\*\*/g, ".*")
			.replace(/\*/g, "[^/]*")}$`;
		try {
			return new RegExp(regexStr).test(relPath);
		} catch {
			return false;
		}
	}

	const EXCLUDED_WORKSPACE_DIRS = new Set([
		"node_modules",
		".git",
		".next",
		"dist",
		"build",
		"target",
		".venv",
		".cache",
		".turbo",
		".parcel-cache",
		"coverage",
	]);

	function parseGitignoreLines(content) {
		const rules = [];
		for (let line of content.split("\n")) {
			line = line.trim();
			if (!line || line.startsWith("#")) continue;
			const isDirOnly = line.endsWith("/");
			if (isDirOnly) line = line.slice(0, -1);
			rules.push({ pattern: line, isDirOnly });
		}
		return rules;
	}

	function isPathGitignored(name, rel, isDir, rules) {
		for (const rule of rules) {
			if (rule.isDirOnly && !isDir) continue;
			if (rule.pattern.includes("/")) {
				if (rel === rule.pattern || rel.endsWith(`/${rule.pattern}`)) return true;
			} else {
				if (name === rule.pattern || rel.split("/").includes(rule.pattern)) return true;
				if (rule.pattern.startsWith("*.")) {
					const ext = rule.pattern.slice(1);
					if (name.endsWith(ext)) return true;
				}
			}
		}
		return false;
	}

	function fallbackGlob(options, onMatch) {
		const rootPath = path.resolve(options?.path || ".");
		const hidden = options?.hidden ?? false;
		const maxResults = options?.maxResults ?? 1000;
		const useGitignore = options?.gitignore !== false;
		const matches = [];

		function walk(currentDir, relativePrefix, parentRules) {
			if (matches.length >= maxResults) return;
			let dirEntries;
			try {
				dirEntries = fs.readdirSync(currentDir, { withFileTypes: true });
			} catch {
				return;
			}

			let localRules = parentRules ? [...parentRules] : [];
			if (useGitignore) {
				const giPath = path.join(currentDir, ".gitignore");
				try {
					if (fs.existsSync(giPath)) {
						const giContent = fs.readFileSync(giPath, "utf8");
						localRules = localRules.concat(parseGitignoreLines(giContent));
					}
				} catch {}
			}

			for (const dirent of dirEntries) {
				if (matches.length >= maxResults) return;
				if (dirent.name === ".DS_Store") continue;
				if (!hidden && dirent.name.startsWith(".")) continue;
				const isDirectory = dirent.isDirectory();
				if (useGitignore && isDirectory && EXCLUDED_WORKSPACE_DIRS.has(dirent.name)) continue;

				const relPath = relativePrefix ? `${relativePrefix}/${dirent.name}` : dirent.name;
				const fullPath = path.join(currentDir, dirent.name);

				if (useGitignore && isPathGitignored(dirent.name, relPath, isDirectory, localRules)) {
					continue;
				}

				let fileType = 1; // File
				let mtime = 0;
				let size = 0;
				try {
					const stat = fs.statSync(fullPath);
					mtime = stat.mtimeMs;
					size = stat.size;
					if (stat.isDirectory()) fileType = 2; // Dir
					else if (stat.isSymbolicLink()) fileType = 3; // Symlink
				} catch {}

				if (fallbackGlobMatches(options?.pattern || "*", relPath)) {
					const match = { path: relPath, fileType, mtime, size };
					matches.push(match);
					try { onMatch?.(null, match); } catch {}
				}

				if (fileType === 2 && options?.recursive !== false) {
					walk(fullPath, relPath, localRules);
				}
			}
		}

		walk(rootPath, "", []);
		return Promise.resolve({
			matches,
			totalMatches: matches.length,
		});
	}

	function fallbackGrep(options, onMatch) {
		const rootPath = path.resolve(options?.path || ".");
		const maxCount = options?.maxCount ?? 1000;
		const maxCountPerFile = options?.maxCountPerFile ?? Number.POSITIVE_INFINITY;
		const useGitignore = options?.gitignore !== false;
		const matches = [];
		let filesSearched = 0;
		const filesWithMatchesSet = new Set();

		let regex;
		try {
			const flags = options?.ignoreCase ? "i" : "";
			regex = new RegExp(options?.pattern || "", flags);
		} catch {
			return Promise.resolve({
				matches: [],
				totalMatches: 0,
				filesWithMatches: 0,
				filesSearched: 0,
				limitReached: false,
			});
		}

		function searchFile(filePath, relPath) {
			if (options?.glob && !fallbackGlobMatches(options.glob, relPath)) return;
			filesSearched++;
			let content;
			try {
				content = fs.readFileSync(filePath, "utf8");
			} catch {
				return;
			}
			// Skip binary files (null bytes)
			if (content.includes("\0")) return;

			const lines = content.split("\n");
			let fileMatches = 0;
			for (let i = 0; i < lines.length; i++) {
				if (matches.length >= maxCount) return;
				if (fileMatches >= maxCountPerFile) break;
				const line = lines[i];
				if (regex.test(line)) {
					filesWithMatchesSet.add(relPath);
					fileMatches++;
					const match = {
						path: relPath,
						lineNumber: i + 1,
						line: line.replace(/\r$/, ""),
					};
					matches.push(match);
					try { onMatch?.(null, match); } catch {}
				}
			}
		}

		function walk(currentDir, relativePrefix, parentRules) {
			if (matches.length >= maxCount) return;
			let dirEntries;
			try {
				dirEntries = fs.readdirSync(currentDir, { withFileTypes: true });
			} catch {
				return;
			}

			let localRules = parentRules ? [...parentRules] : [];
			if (useGitignore) {
				const giPath = path.join(currentDir, ".gitignore");
				try {
					if (fs.existsSync(giPath)) {
						const giContent = fs.readFileSync(giPath, "utf8");
						localRules = localRules.concat(parseGitignoreLines(giContent));
					}
				} catch {}
			}

			for (const dirent of dirEntries) {
				if (matches.length >= maxCount) return;
				if (dirent.name === ".DS_Store") continue;
				if (!options?.hidden && dirent.name.startsWith(".")) continue;
				const isDirectory = dirent.isDirectory();
				if (useGitignore && isDirectory && EXCLUDED_WORKSPACE_DIRS.has(dirent.name)) continue;

				const relPath = relativePrefix ? `${relativePrefix}/${dirent.name}` : dirent.name;
				const fullPath = path.join(currentDir, dirent.name);

				if (useGitignore && isPathGitignored(dirent.name, relPath, isDirectory, localRules)) {
					continue;
				}

				if (isDirectory) {
					walk(fullPath, relPath, localRules);
				} else if (dirent.isFile()) {
					searchFile(fullPath, relPath);
				}
			}
		}

		try {
			const stat = fs.statSync(rootPath);
			if (stat.isDirectory()) {
				walk(rootPath, "", []);
			} else {
				searchFile(rootPath, path.basename(rootPath));
			}
		} catch {}

		return Promise.resolve({
			matches,
			totalMatches: matches.length,
			filesWithMatches: filesWithMatchesSet.size,
			filesSearched,
			limitReached: matches.length >= maxCount,
		});
	}

	function fallbackListWorkspace(options) {
		const rootPath = path.resolve(options?.path || ".");
		const maxDepth = options?.maxDepth ?? 1;
		const hidden = options?.hidden ?? false;
		const useGitignore = options?.gitignore !== false;
		const collectAgentsMd = Boolean(options?.collectAgentsMd);
		const maxWalkDepth = collectAgentsMd ? Math.max(maxDepth, 4) : maxDepth;

		const entries = [];
		const agentsMdFiles = [];

		function walk(currentDir, relativePrefix, depth, parentRules) {
			if (depth > maxWalkDepth) return;
			let dirEntries;
			try {
				dirEntries = fs.readdirSync(currentDir, { withFileTypes: true });
			} catch {
				return;
			}

			let localRules = parentRules ? [...parentRules] : [];
			if (useGitignore) {
				const giPath = path.join(currentDir, ".gitignore");
				try {
					if (fs.existsSync(giPath)) {
						const giContent = fs.readFileSync(giPath, "utf8");
						localRules = localRules.concat(parseGitignoreLines(giContent));
					}
				} catch {}
			}

			for (const dirent of dirEntries) {
				if (dirent.name === ".DS_Store") continue;
				if (!hidden && dirent.name.startsWith(".")) continue;

				const isDirectory = dirent.isDirectory();
				if (isDirectory && EXCLUDED_WORKSPACE_DIRS.has(dirent.name)) continue;

				const relPath = relativePrefix ? `${relativePrefix}/${dirent.name}` : dirent.name;
				const fullPath = path.join(currentDir, dirent.name);

				const gitignored = useGitignore && isPathGitignored(dirent.name, relPath, isDirectory, localRules);
				if (gitignored && isDirectory) continue;

				let fileType = 1; // File
				let mtime = 0;
				let size = 0;
				try {
					const stat = fs.statSync(fullPath);
					mtime = stat.mtimeMs;
					size = stat.size;
					if (stat.isDirectory()) {
						fileType = 2; // Dir
					} else if (stat.isSymbolicLink()) {
						fileType = 3; // Symlink
					}
				} catch {}

				const isAgentsMd = dirent.isFile() && dirent.name.toLowerCase() === "agents.md";
				if (collectAgentsMd && isAgentsMd && depth >= 1 && depth <= 4) {
					agentsMdFiles.push(relPath);
				}

				if (!gitignored || (collectAgentsMd && isAgentsMd)) {
					if (depth + 1 <= maxDepth) {
						entries.push({
							path: relPath,
							fileType,
							mtime,
							size,
						});
					}
				}

				if (fileType === 2 && depth < maxWalkDepth) {
					walk(fullPath, relPath, depth + 1, localRules);
				}
			}
		}

		walk(rootPath, "", 0, []);
		return Promise.resolve({
			entries,
			agentsMdFiles,
			truncated: false,
		});
	}

	const fallback = {
		__ompInstallTokioRuntime: () => {},
		__piNativesV0_1_0: true,
		Ellipsis: "…",
		ProcessStatus: { Running: 0, Sleeping: 1, Stopped: 2, Zombie: 3, Dead: 4 },
		FileType: { File: 1, Dir: 2, Directory: 2, Symlink: 3, Unknown: 0 },
		GrepOutputMode: { Content: "content", Count: "count", FilesWithMatches: "filesWithMatches", Standard: 0, Json: 1 },
		FileLock: FallbackFileLock,
		Shell: FallbackShell,
		// VCS classes and functions
		VcsGitRepo: FallbackVcsGitRepo,
		VcsRepo: FallbackVcsRepo,
		VcsJjWorkspace: FallbackVcsJjWorkspace,
		vcsGitDiscover: (dir) => {
			const data = findGitRepo(dir);
			return data ? new FallbackVcsGitRepo(data) : null;
		},
		vcsDiscover: (dir) => {
			const jjData = findJjRepo(dir);
			const gitData = findGitRepo(dir);
			if (jjData && (!gitData || (gitData.repoRoot !== jjData.root && jjData.root.startsWith(gitData.repoRoot)))) {
				return new FallbackVcsRepo(jjData, "jj");
			}
			if (gitData) {
				return new FallbackVcsRepo(gitData, "git");
			}
			if (jjData) {
				return new FallbackVcsRepo(jjData, "jj");
			}
			return null;
		},
		vcsGitRepoInfo: (dir) => {
			const data = findGitRepo(dir);
			return data
				? {
						repoRoot: data.repoRoot,
						gitEntryPath: data.gitDir,
						gitDir: data.gitDir,
						commonDir: data.commonDir,
						headPath: path.join(data.gitDir, "HEAD"),
						isReftable: false,
						kind: "git",
					}
				: null;
		},
		vcsJjDiscover: (dir) => {
			const data = findJjRepo(dir);
			return data ? new FallbackVcsJjWorkspace(data) : null;
		},
		vcsIsPureJj: (dir) => {
			const jjData = findJjRepo(dir);
			const gitData = findGitRepo(dir);
			return !!(jjData && (!gitData || (gitData.repoRoot !== jjData.root && jjData.root.startsWith(gitData.repoRoot))));
		},
		vcsDetachGitDir: async () => "no-git",
		vcsGitClone: async () => {},
		vcsValidateHunkSelections: () => [],
		vcsJoinPatches: (parts) => (Array.isArray(parts) ? parts.join("\n") : ""),
		// Text/layout helpers — must return correct types (string[] / string / object)
		wrapTextWithAnsi: jsWrapTextWithAnsi,
		truncateToWidth: jsTruncateToWidth,
		sliceWithWidth: jsSliceWithWidth,
		extractSegments: jsExtractSegments,
		setHangulCompatJamoWidthOverride: () => {},
		visibleWidth: (text, tabWidth) => visibleLen(String(text ?? ""), tabWidth),
		// Edit tool description and grammar — returns strings, NOT arrays
		editDescription: (mode) => EDIT_DESCRIPTIONS[mode] ?? EDIT_DESCRIPTIONS.replace,
		editGrammar: (_mode) => null,
		editAutoGeneratedMessage: (_absolutePath, _displayPath) => null,
		editInspect: (_text, _mode) => ({ valid: true, errors: [] }),
		// Hashline helpers
		hashlineFileHash: (text) => {
			let h = 0x811c9dc5;
			for (let i = 0; i < (text ?? "").length; i++) {
				h ^= (text ?? "").charCodeAt(i);
				h = (h * 0x01000193) >>> 0;
			}
			return h.toString(16).padStart(4, "0").slice(0, 4).toUpperCase();
		},
		hashlineFormatHeader: (path, tag) => `[${path}#${tag}]`,
		hashlineFormatNumberedLines: (text, startLine) => {
			const start = startLine ?? 1;
			return String(text ?? "").split("\n").map((line, i) => `${start + i}:${line}`).join("\n");
		},
		hashlineCountOps: () => 0,
		hashlineStripPrefixes: (t) => t ?? "",
		// Code highlighting — fall back to plain text
		highlightCode: (code) => String(code ?? ""),
		warmHighlighter: () => {},
		// AST / search stubs
		search: () => ({ matches: [], matchCount: 0, limitReached: false }),
		grep: fallbackGrep,
		fuzzyFind: (options) => fallbackGlob({ ...options, pattern: "**/*" }),
		astEdit: () =>
			Promise.resolve({
				changes: [],
				fileChanges: [],
				totalReplacements: 0,
				filesTouched: 0,
				filesSearched: 0,
				applied: false,
				limitReached: false,
				parseErrors: [],
			}),
		astGrep: () => Promise.resolve({ matches: [], totalMatches: 0, filesWithMatches: 0, filesSearched: 0, limitReached: false, parseErrors: [] }),
		astMatch: () => false,
		hasMatch: () => false,
		matchesKey: jsMatchesKey,
		matchesKittySequence: jsMatchesKittySequence,
		matchesLegacySequence: jsMatchesLegacySequence,
		supportsLanguage: () => false,
		// ISO stubs
		isoIsUnavailableError: () => true,
		isoProbe: () => null,
		isoResolve: () => null,
		isoStart: () => null,
		isoStop: () => null,
		isoBackend: () => null,
		isoDiff: () => [],
		// Tokenizer & diff
		detectMacOSAppearance: () => "dark",
		countTokens: (text) => {
			if (Array.isArray(text)) return text.reduce((s, t) => s + Math.ceil(String(t).length / 4), 0);
			return typeof text === "string" ? Math.ceil(text.length / 4) : 0;
		},
		diffLines: () => [],
		diffLineRuns: () => [],
		diffWords: () => [],
		editDiffString: () => ({ diff: "", firstChangedLine: undefined }),
		summarizeCode: (code) => code,
		copyToClipboard: () => false,
		executeShell: (options, onChunk) => new FallbackShell({ cwd: options?.cwd }).run(options, onChunk),
		glob: fallbackGlob,
		listWorkspace: fallbackListWorkspace,
		// Sixel & image encoding
		encodeSixel: () => "",
		snapcompactSupportedChars: (_font, chars) => String(chars ?? ""),
		parseKey: jsParseKey,
		parseKittySequence: jsParseKittySequence,
		notebookToEditableText: () => "",
		htmlToMarkdown: (html) => html ?? "",
		pdfToMarkdown: () => "",
		rasterizeSvg: () => null,
		readImageFromClipboard: () => null,
		renderSnapcompactPng: () => null,
		decodeSixelToPng: () => Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGP4z8AAAAMBAQDJ/pLvAAAAAElFTkSuQmCC", "base64"),
		deviceCheckGenerateToken: () => null,
		execReplace: () => "",
		extractInlineSloppyRegions: () => [],
		invalidateFsScanCache: () => {},
	};

	const dummyInstanceHandler = {
		get(target, prop) {
			if (prop in target) return target[prop];
			if (prop === "then") return undefined;
			return () => null;
		},
	};

	const dummyClass = class {
		static fromPid() { return null; }
		status() { return 0; }
		stop() {}
		close() {}
		constructor() {
			return new Proxy(this, dummyInstanceHandler);
		}
	};

	return new Proxy(fallback, {
		get(target, prop) {
			if (prop in target) return target[prop];
			if (typeof prop === "string" && /^[A-Z]/.test(prop)) {
				return dummyClass;
			}
			if (prop === "then") return undefined;
			if (typeof prop === "string" && /^(is|has|supports|matches)/.test(prop)) {
				return () => false;
			}
			if (typeof prop === "string" && /^(list|diff)/.test(prop)) {
				return () => [];
			}
			return () => null;
		},
	});
}

