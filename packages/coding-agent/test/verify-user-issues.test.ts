import { describe, expect, it } from "bun:test";
import * as path from "node:path";
import { executeBash } from "../src/exec/bash-executor";
import { ReadTool } from "../src/tools/read";
import { GlobTool } from "../src/tools/glob";
import { GrepTool } from "../src/tools/grep";
import { Settings } from "../src/config/settings";
import type { ToolSession } from "../src/tools";

function makeSession(cwd: string): ToolSession {
	return {
		cwd,
		hasUI: false,
		getSessionFile: () => path.join(cwd, "session.jsonl"),
		getSessionSpawns: () => "*",
		getArtifactsDir: () => path.join(cwd, "session"),
		allocateOutputArtifact: async (toolType: string) => ({
			id: "a1",
			path: path.join(cwd, "session", `a1.${toolType}.log`),
		}),
		settings: Settings.isolated(),
	};
}

describe("User reported issues verification", () => {
	it("executes 'cd C:\\ && pwd' without throwing null promise error", async () => {
		const cmd = process.platform === "win32" ? "cd C:\\ && pwd" : "cd / && pwd";
		const result = await executeBash(cmd, { timeout: 10000 });
		expect(result.exitCode).toBe(0);
		expect(result.cancelled).toBe(false);
		expect(result.timedOut).toBeFalsy();
		expect(result.output.length).toBeGreaterThan(0);
	});

	it("reads root directory / without undefined matches error", async () => {
		const session = makeSession(process.cwd());
		const readTool = new ReadTool(session);
		const result = await readTool.execute("c1", { path: "/" });
		expect(result.content).toBeDefined();
		expect(result.content.length).toBeGreaterThan(0);
		const first = result.content[0];
		const text = first && "text" in first ? first.text : "";
		expect(text.length).toBeGreaterThan(0);
		expect(text).not.toContain("Cannot read directory: undefined is not an object");
	});

	it("reads Desktop Project-2 or repo directory without error", async () => {
		const testDir = "C:/Users/sanid/Desktop/test/Project-2";
		const targetPath = (await Bun.file(path.join(testDir, "package.json")).exists())
			? testDir
			: path.resolve(process.cwd(), "../..");
		const session = makeSession(targetPath);
		const readTool = new ReadTool(session);
		const result = await readTool.execute("c2", { path: targetPath });
		expect(result.content).toBeDefined();
		expect(result.content.length).toBeGreaterThan(0);
		const first = result.content[0];
		const text = first && "text" in first ? first.text : "";
		expect(text).toContain("package.json");
	});

	it("executes glob without 'undefined is not an object (evaluating match of result.matches)'", async () => {
		const testDir = "C:/Users/sanid/Desktop/test/Project-2";
		const targetPath = (await Bun.file(path.join(testDir, "package.json")).exists()) ? testDir : process.cwd();
		const session = makeSession(targetPath);
		const globTool = new GlobTool(session);
		const searchPattern = targetPath === testDir ? `${targetPath}/**/*.tsx` : `${targetPath}/**/*.ts`;
		const result = await globTool.execute("c3", {
			path: searchPattern,
		});
		expect(result.content).toBeDefined();
		const first = result.content[0];
		const text = first && "text" in first ? first.text : "";
		expect(text.length).toBeGreaterThan(0);
		expect(text).not.toContain("undefined is not an object");
	});

	it("executes grep without 'undefined is not an object (evaluating match of result.matches)'", async () => {
		const testDir = "C:/Users/sanid/Desktop/test/Project-2";
		const targetPath = (await Bun.file(path.join(testDir, "package.json")).exists()) ? testDir : process.cwd();
		const session = makeSession(targetPath);
		const grepTool = new GrepTool(session);
		const result = await grepTool.execute("c4", {
			path: targetPath,
			pattern: "export",
		});
		expect(result.content).toBeDefined();
		const first = result.content[0];
		const text = first && "text" in first ? first.text : "";
		expect(text).toContain("export");
		expect(text).not.toContain("undefined is not an object");
	});
});
