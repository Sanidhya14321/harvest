import { afterEach, describe, expect, test } from "bun:test";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { encodeProjectPath, getMemoryRoot, resolveExistingMemoryRoot } from "@harvest/pi-coding-agent/memories";
import { getMemoriesDir, Snowflake } from "@harvest/pi-utils";

const tmpDirs: string[] = [];

afterEach(() => {
	while (tmpDirs.length > 0) {
		const dir = tmpDirs.pop();
		if (dir) fs.rmSync(dir, { recursive: true, force: true });
	}
});

function makeTemp(prefix: string): string {
	const dir = fs.mkdtempSync(path.join(os.tmpdir(), prefix));
	tmpDirs.push(dir);
	return dir;
}

/** Predecessor encoding (17.x): lossy folding + 64-bit Bun.hash. Fixture only. */
function encodePreviousProjectPath(cwd: string): string {
	const canonical = path.resolve(cwd).replaceAll("\\", "/");
	const digest = Bun.hash(canonical).toString(16);
	return `--${canonical.replace(/^[/\\]/, "").replace(/[/\\:]/g, "-")}-${digest}--`;
}

/** Oldest encoding: lossy folding, no digest. Fixture only. */
function encodeLegacyProjectPath(cwd: string): string {
	return `--${cwd.replace(/^[/\\]/, "").replace(/[/\\:]/g, "-")}--`;
}

describe("memories project-path encoding", () => {
	test("/a/b:c and /a/b/c map to different directories", () => {
		expect(encodeProjectPath("/a/b:c")).not.toBe(encodeProjectPath("/a/b/c"));
	});

	test("encoding is deterministic and filesystem-shaped", () => {
		const first = encodeProjectPath("/some/project");
		expect(encodeProjectPath("/some/project")).toBe(first);
		expect(first.startsWith("--")).toBe(true);
		expect(first.endsWith("--")).toBe(true);
		expect(first).not.toContain("/");
		expect(first).not.toContain("\\");
		expect(first).not.toContain(":");
	});

	test("getMemoryRoot migrates a previous-encoding directory preserving contents", () => {
		const agentDir = makeTemp("mem-enc-agent-");
		const cwd = makeTemp("mem-enc-cwd-");
		const previousDir = path.join(getMemoriesDir(agentDir), encodePreviousProjectPath(cwd));
		fs.mkdirSync(previousDir, { recursive: true });
		fs.writeFileSync(path.join(previousDir, "marker.txt"), "keep me");

		const root = getMemoryRoot(agentDir, cwd);
		expect(root).toBe(path.join(getMemoriesDir(agentDir), encodeProjectPath(cwd)));
		expect(fs.readFileSync(path.join(root, "marker.txt"), "utf8")).toBe("keep me");
		expect(fs.existsSync(previousDir)).toBe(false);
	});

	test("getMemoryRoot migrates a legacy-encoding directory preserving contents", () => {
		const agentDir = makeTemp("mem-enc-agent-");
		const cwd = makeTemp("mem-enc-cwd-");
		const legacyDir = path.join(getMemoriesDir(agentDir), encodeLegacyProjectPath(cwd));
		fs.mkdirSync(legacyDir, { recursive: true });
		fs.writeFileSync(path.join(legacyDir, "marker.txt"), "legacy");

		const root = getMemoryRoot(agentDir, cwd);
		expect(root).toBe(path.join(getMemoriesDir(agentDir), encodeProjectPath(cwd)));
		expect(fs.readFileSync(path.join(root, "marker.txt"), "utf8")).toBe("legacy");
		expect(fs.existsSync(legacyDir)).toBe(false);
	});

	test("resolveExistingMemoryRoot finds old directories without migrating", () => {
		const agentDir = makeTemp("mem-enc-agent-");
		const cwd = makeTemp("mem-enc-cwd-");
		const previousDir = path.join(getMemoriesDir(agentDir), encodePreviousProjectPath(cwd));
		fs.mkdirSync(previousDir, { recursive: true });

		expect(resolveExistingMemoryRoot(agentDir, cwd)).toBe(previousDir);
		expect(fs.existsSync(previousDir)).toBe(true);
	});

	test("getMemoryRoot prefers the current encoding when both exist", () => {
		const agentDir = makeTemp("mem-enc-agent-");
		const cwd = makeTemp("mem-enc-cwd-");
		const previousDir = path.join(getMemoriesDir(agentDir), encodePreviousProjectPath(cwd));
		const currentDir = path.join(getMemoriesDir(agentDir), encodeProjectPath(cwd));
		fs.mkdirSync(previousDir, { recursive: true });
		fs.mkdirSync(currentDir, { recursive: true });

		expect(getMemoryRoot(agentDir, cwd)).toBe(currentDir);
		expect(fs.existsSync(previousDir)).toBe(true);
	});

	test("memory roots stay unique across separator variants", () => {
		const agentDir = makeTemp("mem-enc-agent-");
		const first = getMemoryRoot(agentDir, `${os.tmpdir()}${path.sep}proj-${Snowflake.next()}:x`);
		const second = getMemoryRoot(agentDir, `${os.tmpdir()}${path.sep}proj-${Snowflake.next()}${path.sep}x`);
		expect(first).not.toBe(second);
	});
});
