import { afterEach, describe, expect, it } from "bun:test";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { getMemoriesDir } from "@harvest/pi-utils";
import { encodeProjectPath, getMemoryRoot, resolveExistingMemoryRoot } from "../../src/memories/index";

/**
 * Project-memory root encoding (CODE_REVIEW_3 P3): the 17.x scheme folded
 * `[/\\:]` to `-` with a 64-bit non-crypto digest, so `/a/b:c` and `/a/b/c`
 * shared one directory and mixed cross-project memory. The current encoding
 * escapes separator classes distinctly plus a SHA-256 prefix; the old
 * schemes survive only as frozen migration sources below.
 */

const roots: string[] = [];
afterEach(async () => {
	await Promise.all(roots.splice(0).map(root => fs.promises.rm(root, { recursive: true, force: true })));
});

async function scratch(): Promise<string> {
	const root = await fs.promises.mkdtemp(path.join(os.tmpdir(), "harvest-memory-path-"));
	roots.push(root);
	return root;
}

/** 17.x hashed scheme, frozen for migration fixtures (see encodeHashedProjectPath). */
function hashed17Name(cwd: string): string {
	const canonical = path.resolve(cwd).replaceAll("\\", "/");
	const digest = Bun.hash(canonical).toString(16);
	return `--${canonical.replace(/^[/\\]/, "").replace(/[/\\:]/g, "-")}-${digest}--`;
}

/** Pre-17.x legacy scheme, frozen for migration fixtures (see encodeLegacyProjectPath). */
function legacyName(cwd: string): string {
	return `--${cwd.replace(/^[/\\]/, "").replace(/[/\\:]/g, "-")}--`;
}

describe("project memory path encoding", () => {
	it("keeps separator-folding collisions apart", () => {
		expect(encodeProjectPath("/a/b:c")).not.toBe(encodeProjectPath("/a/b/c"));
		expect(encodeProjectPath("/a/b-c")).not.toBe(encodeProjectPath("/a/b/c"));
	});

	it("is deterministic and canonicalizes the same directory identically", () => {
		expect(encodeProjectPath("/a/b/c")).toBe(encodeProjectPath("/a/b/c"));
		expect(encodeProjectPath("/a/b/c/")).toBe(encodeProjectPath("/a/b/c"));
	});

	it("migrates a 17.x directory without losing memory", async () => {
		const agentDir = await scratch();
		const projCwd = path.join(agentDir, "proj");
		await fs.promises.mkdir(projCwd, { recursive: true });
		const base = getMemoriesDir(agentDir);
		await fs.promises.mkdir(base, { recursive: true });
		const oldDir = path.join(base, hashed17Name(projCwd));
		await fs.promises.mkdir(oldDir, { recursive: true });
		await Bun.write(path.join(oldDir, "MEMORY.md"), "old memory");

		const root = getMemoryRoot(agentDir, projCwd);
		expect(root).toBe(path.join(base, encodeProjectPath(projCwd)));
		expect(await Bun.file(path.join(root, "MEMORY.md")).text()).toBe("old memory");
		expect(fs.existsSync(oldDir)).toBe(false);
	});

	it("migrates a legacy directory and prefers the current encoding when both exist", async () => {
		const agentDir = await scratch();
		const projCwd = path.join(agentDir, "proj");
		await fs.promises.mkdir(projCwd, { recursive: true });
		const base = getMemoriesDir(agentDir);
		await fs.promises.mkdir(base, { recursive: true });
		const legacyDir = path.join(base, legacyName(projCwd));
		await fs.promises.mkdir(legacyDir, { recursive: true });
		await Bun.write(path.join(legacyDir, "MEMORY.md"), "legacy memory");

		const root = getMemoryRoot(agentDir, projCwd);
		expect(root).toBe(path.join(base, encodeProjectPath(projCwd)));
		expect(await Bun.file(path.join(root, "MEMORY.md")).text()).toBe("legacy memory");

		// A second project already on the current encoding keeps its root;
		// stale predecessors are never merged across roots.
		const proj2 = path.join(agentDir, "proj2");
		await fs.promises.mkdir(proj2, { recursive: true });
		const current2 = path.join(base, encodeProjectPath(proj2));
		const stale2 = path.join(base, hashed17Name(proj2));
		await fs.promises.mkdir(current2, { recursive: true });
		await fs.promises.mkdir(stale2, { recursive: true });
		expect(getMemoryRoot(agentDir, proj2)).toBe(current2);
		expect(fs.existsSync(stale2)).toBe(true);
	});

	it("locates roots written by any encoding without migrating", async () => {
		const agentDir = await scratch();
		const projCwd = path.join(agentDir, "proj");
		await fs.promises.mkdir(projCwd, { recursive: true });
		const base = getMemoriesDir(agentDir);
		await fs.promises.mkdir(base, { recursive: true });
		const oldDir = path.join(base, hashed17Name(projCwd));
		await fs.promises.mkdir(oldDir, { recursive: true });

		expect(resolveExistingMemoryRoot(agentDir, projCwd)).toBe(oldDir);
		expect(fs.existsSync(oldDir)).toBe(true);
		expect(resolveExistingMemoryRoot(agentDir, path.join(agentDir, "absent"))).toBeUndefined();
	});
});
