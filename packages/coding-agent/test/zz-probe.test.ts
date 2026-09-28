import { test } from "bun:test";
import * as fs from "node:fs/promises";
import * as os from "node:os";
import * as path from "node:path";
import { Settings } from "@harvest/pi-coding-agent/config/settings";
import type { ToolSession } from "@harvest/pi-coding-agent/tools";
import { WriteTool } from "@harvest/pi-coding-agent/tools/write";

function createSession(cwd: string): ToolSession {
	return {
		cwd,
		hasUI: false,
		getSessionFile: () => path.join(cwd, "session.jsonl"),
		getSessionSpawns: () => "*",
		getArtifactsDir: () => path.join(cwd, "artifacts"),
		allocateOutputArtifact: async () => ({ id: "artifact-1", path: path.join(cwd, "artifact-1.log") }),
		settings: Settings.isolated(),
		enableLsp: false,
	} as unknown as ToolSession;
}

test("probe archive write outcome", async () => {
	await Settings.init({ inMemory: true });
	const root = await fs.mkdtemp(path.join(os.tmpdir(), "probe2-"));
	const ws = path.join(root, "ws");
	const outside = path.join(root, "outside");
	await fs.mkdir(ws, { recursive: true });
	await fs.mkdir(outside, { recursive: true });
	await fs.symlink(outside, path.join(ws, "evilstore"), "junction");
	const target = `${path.join(ws, "evilstore", "bundle.zip")}:entry.txt`;
	console.log(`TARGET: ${target}`);
	try {
		const result = await new WriteTool(createSession(ws)).execute("probe", { path: target, content: "hello\n" });
		console.log(`RESULT: ${JSON.stringify(result).slice(0, 400)}`);
	} catch (error) {
		console.log(`THREW: ${error instanceof Error ? error.message : String(error)}`);
	}
	console.log(`OUTSIDE BUNDLE EXISTS: ${await Bun.file(path.join(outside, "bundle.zip")).exists()}`);
	const sTarget = `${path.join(ws, "evilstore", "db.sqlite")}:users:1`;
	try {
		const result = await new WriteTool(createSession(ws)).execute("probe2", {
			path: sTarget,
			content: '{"name":"x"}\n',
		});
		console.log(`SQLITE RESULT: ${JSON.stringify(result).slice(0, 400)}`);
	} catch (error) {
		console.log(`SQLITE THREW: ${error instanceof Error ? error.message : String(error)}`);
	}
	console.log(`OUTSIDE DB EXISTS: ${await Bun.file(path.join(outside, "db.sqlite")).exists()}`);
});
