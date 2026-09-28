import { afterEach, beforeAll, beforeEach, describe, expect, it } from "bun:test";
import * as fs from "node:fs/promises";
import * as os from "node:os";
import * as path from "node:path";
import { Settings } from "@harvest/pi-coding-agent/config/settings";
import { SecuritySandbox } from "@harvest/pi-coding-agent/core/harvest/security";
import { EditTool } from "@harvest/pi-coding-agent/edit";
import { applyWorkspaceEdit } from "@harvest/pi-coding-agent/lsp/edits";
import type { ToolSession } from "@harvest/pi-coding-agent/tools";
import { WriteTool } from "@harvest/pi-coding-agent/tools/write";
import { fileToUri } from "@harvest/pi-coding-agent/lsp/utils";
import { removeWithRetries } from "@harvest/pi-utils";

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

function jailRejection(value: unknown): void {
	expect(String(value)).toMatch(/traversal|outside workspace/i);
}

describe("workspace jail", () => {
	let root: string;
	let ws: string;
	let outside: string;

	beforeAll(async () => {
		await Settings.init({ inMemory: true });
	});

	beforeEach(async () => {
		root = await fs.mkdtemp(path.join(os.tmpdir(), "ws-jail-"));
		ws = path.join(root, "ws");
		outside = path.join(root, "outside");
		await fs.mkdir(ws, { recursive: true });
		await fs.mkdir(outside, { recursive: true });
	});

	afterEach(async () => {
		await removeWithRetries(root);
	});

	async function linkOutside(name: string): Promise<string> {
		const linkPath = path.join(ws, name);
		// "junction" works on Windows without elevation and is ignored on POSIX.
		await fs.symlink(outside, linkPath, "junction");
		return linkPath;
	}

	it("(a) rejects a new file under a symlink that points outside the workspace", async () => {
		const linkPath = await linkOutside("evildir");
		const target = path.join(linkPath, "new.txt");

		const check = new SecuritySandbox(ws).assertPathJailed(target);
		expect(check.jailed).toBe(false);

		const tool = new WriteTool(createSession(ws));
		const failure = await tool.execute("jail-a", { path: target, content: "nope\n" }).catch(error => error);
		jailRejection(failure instanceof Error ? failure.message : failure);
		await expect(Bun.file(path.join(outside, "new.txt")).exists()).resolves.toBe(false);
	});

	it("(b) jails the edit delete op and the LSP delete op", async () => {
		const victim = path.join(outside, "victim.txt");
		await Bun.write(victim, "precious\n");

		// Edit tool (patch mode) delete of an absolute outside path.
		const editResult = await new EditTool(createSession(ws), "patch")
			.execute("jail-b-edit", { path: victim, edits: [{ op: "delete" }] })
			.then(
				result => result,
				error => error,
			);
		if (editResult instanceof Error) {
			jailRejection(editResult.message);
		} else {
			expect(editResult.isError).toBe(true);
			jailRejection(JSON.stringify(editResult.content));
		}
		await expect(Bun.file(victim).exists()).resolves.toBe(true);
		await expect(Bun.file(victim).text()).resolves.toBe("precious\n");

		// LSP workspace-edit delete op against the same outside file.
		const lspFailure = await applyWorkspaceEdit(
			{
				documentChanges: [{ kind: "delete", uri: fileToUri(victim) }],
			},
			ws,
		).then(
			() => null,
			error => error,
		);
		expect(lspFailure).not.toBeNull();
		jailRejection(lspFailure instanceof Error ? lspFailure.message : lspFailure);
		await expect(Bun.file(victim).exists()).resolves.toBe(true);
	});

	it("(c) rejects archive and sqlite writes through an outside symlink", async () => {
		const linkPath = await linkOutside("evilstore");

		const archiveTarget = `${path.join(linkPath, "bundle.zip")}:entry.txt`;
		const archiveFailure = await new WriteTool(createSession(ws))
			.execute("jail-c-archive", { path: archiveTarget, content: "hello\n" })
			.catch(error => error);
		jailRejection(archiveFailure instanceof Error ? archiveFailure.message : archiveFailure);
		await expect(Bun.file(path.join(outside, "bundle.zip")).exists()).resolves.toBe(false);

		const sqliteTarget = `${path.join(linkPath, "db.sqlite")}:users:1`;
		const sqliteFailure = await new WriteTool(createSession(ws))
			.execute("jail-c-sqlite", { path: sqliteTarget, content: '{"name":"x"}\n' })
			.catch(error => error);
		jailRejection(sqliteFailure instanceof Error ? sqliteFailure.message : sqliteFailure);
		await expect(Bun.file(path.join(outside, "db.sqlite")).exists()).resolves.toBe(false);
	});

	it("(d) rejects LSP text edits and creates against absolute outside URIs", async () => {
		const outsideFile = path.join(outside, "secret.txt");
		await Bun.write(outsideFile, "original\n");

		const editFailure = await applyWorkspaceEdit(
			{
				changes: {
					[fileToUri(outsideFile)]: [
						{
							range: { start: { line: 0, character: 0 }, end: { line: 0, character: 8 } },
							newText: "pwned",
						},
					],
				},
			},
			ws,
		).then(
			() => null,
			error => error,
		);
		expect(editFailure).not.toBeNull();
		jailRejection(editFailure instanceof Error ? editFailure.message : editFailure);
		await expect(Bun.file(outsideFile).text()).resolves.toBe("original\n");

		const createFailure = await applyWorkspaceEdit(
			{
				documentChanges: [{ kind: "create", uri: fileToUri(path.join(outside, "created.txt")) }],
			},
			ws,
		).then(
			() => null,
			error => error,
		);
		expect(createFailure).not.toBeNull();
		jailRejection(createFailure instanceof Error ? createFailure.message : createFailure);
		await expect(Bun.file(path.join(outside, "created.txt")).exists()).resolves.toBe(false);
	});

	it("keeps in-workspace writes working, including through an inside symlink", async () => {
		const plain = path.join(ws, "plain.txt");
		const plainResult = await new WriteTool(createSession(ws)).execute("jail-ok", {
			path: plain,
			content: "hello\n",
		});
		expect(plainResult.isError ?? false).toBe(false);
		await expect(Bun.file(plain).text()).resolves.toBe("hello\n");

		const inner = path.join(ws, "inner");
		await fs.mkdir(inner, { recursive: true });
		await fs.symlink(inner, path.join(ws, "insidelink"), "junction");
		const viaLink = path.join(ws, "insidelink", "linked.txt");
		const linkResult = await new WriteTool(createSession(ws)).execute("jail-ok-link", {
			path: viaLink,
			content: "linked\n",
		});
		expect(linkResult.isError ?? false).toBe(false);
		await expect(Bun.file(path.join(inner, "linked.txt")).text()).resolves.toBe("linked\n");
	});
});
