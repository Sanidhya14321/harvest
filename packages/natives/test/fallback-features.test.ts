import { describe, expect, it } from "bun:test";
import * as os from "node:os";
import * as path from "node:path";
import * as fs from "node:fs/promises";
import { Shell, executeShell, listWorkspace, glob, grep, search, astGrep, astEdit, FileType } from "../native/index.js";

describe("Native Fallback Features", () => {
	it("executes commands and tracks working directory in FallbackShell", async () => {
		const shell = new Shell();
		expect(shell).toBeDefined();

		const chunks: string[] = [];
		const result = await shell.run({ command: "echo test-run" }, (_err, chunk) => {
			chunks.push(chunk);
		});

		expect(result.exitCode).toBe(0);
		expect(result.cancelled).toBe(false);
		expect(result.timedOut).toBe(false);
		expect(chunks.join("")).toContain("test-run");
	});

	it("executes compound cd and pwd command without crashing", async () => {
		const shell = new Shell();
		const chunks: string[] = [];
		const cmd = process.platform === "win32" ? "cd C:\\ && pwd" : "cd / && pwd";
		const result = await shell.run({ command: cmd }, (_err, chunk) => {
			chunks.push(chunk);
		});

		expect(result.exitCode).toBe(0);
		expect(result.cancelled).toBe(false);
		expect(result.timedOut).toBe(false);
		expect(typeof result.workingDir).toBe("string");
		expect(result.workingDir?.length).toBeGreaterThan(0);
	});

	it("executeShell returns a valid ShellRunResult promise", async () => {
		const chunks: string[] = [];
		const result = await executeShell({ command: "echo hello" }, (_err, chunk) => {
			chunks.push(chunk);
		});

		expect(result.exitCode).toBe(0);
		expect(result.cancelled).toBe(false);
		expect(result.timedOut).toBe(false);
		expect(chunks.join("")).toContain("hello");
	});

	it("listWorkspace returns entries array and agentsMdFiles", async () => {
		const tmp = await fs.mkdtemp(path.join(os.tmpdir(), "fallback-lw-"));
		try {
			await fs.writeFile(path.join(tmp, "hello.txt"), "world");
			await fs.mkdir(path.join(tmp, "sub"));
			await fs.writeFile(path.join(tmp, "sub", "AGENTS.md"), "rules");

			const result = await listWorkspace({
				path: tmp,
				maxDepth: 2,
				collectAgentsMd: true,
			});

			expect(Array.isArray(result.entries)).toBe(true);
			expect(result.entries.length).toBeGreaterThanOrEqual(2);
			expect(Array.isArray(result.agentsMdFiles)).toBe(true);
			expect(result.agentsMdFiles).toContain("sub/AGENTS.md");
			expect(result.truncated).toBe(false);

			const helloEntry = result.entries.find(e => e.path === "hello.txt");
			expect(helloEntry).toBeDefined();
			expect(helloEntry?.fileType).toBe(FileType.File);
		} finally {
			await fs.rm(tmp, { recursive: true, force: true });
		}
	});

	it("glob returns matches array with correct shape", async () => {
		const tmp = await fs.mkdtemp(path.join(os.tmpdir(), "fallback-glob-"));
		try {
			await fs.writeFile(path.join(tmp, "a.ts"), "const a = 1;");
			await fs.writeFile(path.join(tmp, "b.js"), "const b = 2;");

			const result = await glob({
				path: tmp,
				pattern: "**/*.ts",
			});

			expect(Array.isArray(result.matches)).toBe(true);
			expect(result.totalMatches).toBe(1);
			expect(result.matches[0].path).toBe("a.ts");
		} finally {
			await fs.rm(tmp, { recursive: true, force: true });
		}
	});

	it("grep returns matches array with correct shape", async () => {
		const tmp = await fs.mkdtemp(path.join(os.tmpdir(), "fallback-grep-"));
		try {
			await fs.writeFile(path.join(tmp, "test.txt"), "line 1\nFIND_ME target\nline 3");

			const result = await grep({
				path: tmp,
				pattern: "FIND_ME",
			});

			expect(Array.isArray(result.matches)).toBe(true);
			expect(result.totalMatches).toBe(1);
			expect(result.filesWithMatches).toBe(1);
			expect(result.filesSearched).toBeGreaterThanOrEqual(1);
			expect(result.matches[0].line).toContain("FIND_ME target");
			expect(result.matches[0].lineNumber).toBe(2);
		} finally {
			await fs.rm(tmp, { recursive: true, force: true });
		}
	});

	it("search, astGrep, and astEdit return non-null object shapes with arrays", async () => {
		const searchRes = search("some content", { pattern: "content" });
		expect(Array.isArray(searchRes.matches)).toBe(true);
		expect(searchRes.matchCount).toBe(0);

		const astGrepRes = await astGrep({ pattern: "x" } as any);
		expect(Array.isArray(astGrepRes.matches)).toBe(true);
		expect(astGrepRes.totalMatches).toBe(0);

		const astEditRes = await astEdit({ pattern: "x", rewrite: "y" } as any);
		expect(Array.isArray(astEditRes.changes)).toBe(true);
		expect(Array.isArray(astEditRes.fileChanges)).toBe(true);
		expect(astEditRes.totalReplacements).toBe(0);
	});
});
