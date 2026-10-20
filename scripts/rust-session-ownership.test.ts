import { expect, test } from "bun:test";
import * as fs from "node:fs/promises";
import * as os from "node:os";
import * as path from "node:path";
import { tryAcquireFileLock } from "@harvest/pi-utils/file-lock";

test("legacy ownership blocks Rust writes until handoff, then permits durable completion", async () => {
	const binary = process.env.HARVEST_RUST_OMP;
	if (!binary) throw new Error("Set HARVEST_RUST_OMP to the built Rust executable for this integration contract");
	const root = await fs.mkdtemp(path.join(os.tmpdir(), "harvest-ownership-"));
	const session = path.join(root, "session.jsonl");
	let requests = 0;
	const server = Bun.serve({
		hostname: "127.0.0.1",
		port: 0,
		fetch: () => {
			requests++;
			return Response.json({ choices: [{ message: { role: "assistant", content: "handoff complete" } }] });
		},
	});
	const lease = tryAcquireFileLock(session);
	expect(lease?.acquired).toBe(true);
	try {
		const args = [
			binary,
			"run",
			"--workspace",
			root,
			"--session",
			session,
			"--prompt",
			"ownership probe",
			"--base-url",
			`http://127.0.0.1:${server.port}/v1`,
			"--model",
			"fixture",
			"--api-key",
			"fixture",
		];
		const blocked = Bun.spawn(args, { stdout: "pipe", stderr: "pipe" });
		const blockedError = await new Response(blocked.stderr).text();
		const blockedOutput = await new Response(blocked.stdout).text();
		expect({ code: await blocked.exited, error: blockedError, output: blockedOutput }).toMatchObject({ code: 1 });
		expect(blockedError).toContain("another writer owns this session");
		expect(requests).toBe(0);
		expect(await Bun.file(session).exists()).toBe(false);
		lease?.release();
		const accepted = Bun.spawn(args, { stdout: "pipe", stderr: "pipe" });
		const output = await new Response(accepted.stdout).text();
		const error = await new Response(accepted.stderr).text();
		expect({ exit: await accepted.exited, error }).toEqual({ exit: 0, error: "" });
		expect(output).toContain("handoff complete");
		expect(requests).toBe(1);
		const rows = Bun.JSONL.parse(await Bun.file(session).text()) as { message?: { role?: string } }[];
		expect(rows.filter(row => row.message).map(row => row.message?.role)).toEqual(["user", "assistant"]);
		const successor = tryAcquireFileLock(session);
		expect(successor?.acquired).toBe(true);
		successor?.release();
	} finally {
		lease?.release();
		server.stop(true);
		await fs.rm(root, { recursive: true, force: true });
	}
});
