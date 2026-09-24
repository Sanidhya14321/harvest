/**
 * Shared on-disk staging for subprocess kernel runner scripts.
 *
 * Each language kernel (Python/Julia/Ruby) ships its runner as a compiled-in
 * text asset, then stages it under `os.tmpdir()` so the interpreter can load it
 * as a normal file. Staging is cached per language directory so repeated kernel
 * starts within a process avoid redundant writes.
 */
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";

// Memoized staged path per cache directory. The value is re-validated on every
// call: a tmpdir sweep (e.g. macOS `periodic daily clean_tmps`) or any external
// clear must self-heal within a long-lived process, not only across restarts.
const stagedPaths = new Map<string, string>();

/**
 * Stage `script` under `os.tmpdir()/<dirName>` and return the runner path.
 *
 * The staged path is memoized per `dirName` but re-checked with `fs.existsSync`
 * before reuse, so a runner deleted mid-session is re-written on the next call
 * instead of handing back a path to a missing file (issue #8140).
 *
 * @param dirName Cache subdirectory under the OS temp dir (unique per language).
 * @param ext Runner file extension without the dot (e.g. `py`).
 * @param script Runner source, hashed to key the cached file per version.
 */
export async function stageRunnerScript(dirName: string, ext: string, script: string): Promise<string> {
	const memoized = stagedPaths.get(dirName);
	if (memoized && fs.existsSync(memoized)) {
		try {
			const stat = await fs.promises.lstat(memoized);
			if (!stat.isSymbolicLink()) {
				const existing = await fs.promises.readFile(memoized, "utf8");
				if (existing === script) return memoized;
			}
		} catch {}
	}

	const uidSuffix = typeof process.getuid === "function" ? String(process.getuid()) : os.userInfo().username;
	const dir = path.join(os.tmpdir(), `omp-eval-${uidSuffix}`, dirName);
	await fs.promises.mkdir(dir, { recursive: true, mode: 0o700 });
	if (process.platform !== "win32") {
		try {
			await fs.promises.chmod(dir, 0o700);
		} catch {}
	}
	const stat = await fs.promises.lstat(dir);
	if (stat.isSymbolicLink()) {
		throw new Error(`Eval runner directory '${dir}' is a symbolic link`);
	}
	if (typeof process.getuid === "function" && stat.uid !== process.getuid()) {
		throw new Error(`Eval runner directory '${dir}' is not owned by current user`);
	}

	const hash = Bun.hash(script).toString(36);
	const target = path.join(dir, `runner-${hash}.${ext}`);

	let writeNeeded = true;
	try {
		const targetStat = await fs.promises.lstat(target);
		if (targetStat.isSymbolicLink()) {
			await fs.promises.unlink(target);
		} else {
			const existing = await fs.promises.readFile(target, "utf8");
			if (existing === script) {
				writeNeeded = false;
			}
		}
	} catch {
		writeNeeded = true;
	}

	if (writeNeeded) {
		const tmp = path.join(dir, `tmp-${Date.now()}-${Math.random().toString(36).slice(2)}.${ext}`);
		await fs.promises.writeFile(tmp, script, { mode: 0o600, flag: "wx" });
		await fs.promises.rename(tmp, target);
		if (process.platform !== "win32") {
			try {
				await fs.promises.chmod(target, 0o600);
			} catch {}
		}
	}

	stagedPaths.set(dirName, target);
	return target;
}
