import { afterEach, describe, expect, it } from "bun:test";
import * as fs from "node:fs/promises";
import * as path from "node:path";
import { TempDir } from "@harvest/pi-utils";
import { SessionManager } from "../src/session/session-manager";

const managers: SessionManager[] = [];
const directories: TempDir[] = [];
afterEach(async () => {
	for (const manager of managers.splice(0)) await manager.close().catch(() => {});
	for (const directory of directories.splice(0)) await directory.remove();
});
async function seed() {
	const directory = TempDir.createSync("@harvest-writer-ownership-");
	directories.push(directory);
	const manager = SessionManager.create(directory.path(), path.join(directory.path(), "sessions"));
	managers.push(manager);
	manager.appendMessage({ role: "user", content: "seed", timestamp: Date.now() });
	await manager.ensureOnDisk();
	return { manager, directory, file: manager.getSessionFile()! };
}
async function open(file: string, readOnly = false) {
	const manager = await SessionManager.open(file, undefined, undefined, { readOnly, suppressBreadcrumb: true });
	managers.push(manager);
	return manager;
}
describe("file-backed session writer ownership", () => {
	it("rejects a second writer while permitting a non-mutating transcript snapshot", async () => {
		const { manager, file } = await seed();
		await expect(open(file)).rejects.toThrow("already open for writing");
		const snapshot = await open(file, true);
		const original = await Bun.file(file).text();
		snapshot.appendMessage({ role: "user", content: "snapshot-only", timestamp: Date.now() });
		await snapshot.rewriteEntries();
		await expect(snapshot.dropSession(file)).rejects.toThrow("read-only");
		expect(await Bun.file(file).text()).toBe(original);
		manager.appendMessage({ role: "user", content: "owner committed", timestamp: Date.now() });
		await manager.close();
		const successor = await open(file);
		await successor.rewriteEntries();
		manager.appendMessage({ role: "user", content: "stale owner", timestamp: Date.now() });
		await expect(manager.rewriteEntries()).rejects.toThrow("already open for writing");
		expect(await Bun.file(file).text()).toContain("owner committed");
		expect(await Bun.file(file).text()).not.toContain("stale owner");
	});

	it("treats a directory junction or symlink alias as the same writer identity", async () => {
		const { directory, file } = await seed();
		const alias = path.join(directory.path(), "alias");
		await fs.symlink(path.dirname(file), alias, process.platform === "win32" ? "junction" : "dir");
		await expect(open(path.join(alias, path.basename(file)))).rejects.toThrow("already open for writing");
	});

	it("refuses a stale rollback snapshot after another owner changes the old session", async () => {
		const { manager, directory, file } = await seed();
		const snapshot = manager.captureState();
		await manager.newSession();
		const other = await open(file);
		other.appendMessage({ role: "user", content: "new owner history", timestamp: Date.now() });
		await other.close();
		expect(() => manager.restoreState(snapshot)).toThrow("Session changed");
		expect(await Bun.file(file).text()).toContain("new owner history");
		expect(manager.getSessionFile()).not.toBe(file);
		expect(manager.getCwd()).toBe(directory.path());
	});

	it("releases a failed open's lease so repaired history can be resumed", async () => {
		const { manager, file } = await seed();
		await manager.close();
		const valid = await Bun.file(file).text();
		await Bun.write(file, '{"type":"message","id":"bad"}\n');
		await expect(open(file)).rejects.toThrow("missing or malformed");
		await Bun.write(file, valid);
		const recovered = await open(file);
		recovered.appendMessage({ role: "user", content: "recovered", timestamp: Date.now() });
		await recovered.flush();
		expect(await Bun.file(file).text()).toContain("recovered");
	});

	it("recovers ownership after process death without losing the dead owner's completed append", async () => {
		const { manager, directory, file } = await seed();
		await manager.close();
		const ready = path.join(directory.path(), "ready");
		const child = Bun.spawn(
			[process.execPath, path.join(import.meta.dir, "fixtures/session-writer-holder.ts"), file, ready],
			{
				stdout: "ignore",
				stderr: "pipe",
				stdin: "ignore",
			},
		);
		try {
			const deadline = performance.now() + 5000;
			while (!(await Bun.file(ready).exists())) {
				if (child.exitCode !== null || performance.now() > deadline)
					throw new Error("Writer holder did not become ready");
				await Bun.sleep(20);
			}
			await expect(open(file)).rejects.toThrow("already open for writing");
			child.kill();
			await child.exited;
			const recovered = await open(file);
			await recovered.rewriteEntries();
			expect(await Bun.file(file).text()).toContain("child committed");
		} finally {
			if (child.exitCode === null) {
				child.kill();
				await child.exited;
			}
		}
	}, 10_000);
});
