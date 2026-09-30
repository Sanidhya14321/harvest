import { describe, expect, test } from "bun:test";
import * as fs from "node:fs";
import * as path from "node:path";
import { readTerminalBreadcrumbEntry, writeTerminalBreadcrumb } from "@harvest/pi-coding-agent/session/session-paths";
import { FileSessionStorage } from "@harvest/pi-coding-agent/session/session-storage";
import { getTerminalSessionsDir, Snowflake, TempDir } from "@harvest/pi-utils";
import { restoreEnvValue } from "../helpers/settings-test-state";

const isWindows = process.platform === "win32";

describe("session private permissions", () => {
	test("writeText round-trips content through the 0600-at-create path", async () => {
		using tempDir = TempDir.createSync("@omp-session-perms-");
		const storage = new FileSessionStorage();
		const file = path.join(tempDir.path(), "sessions", "test.jsonl");
		await storage.writeText(file, '{"hello":1}\n');
		expect(await storage.readText(file)).toBe('{"hello":1}\n');
		if (!isWindows) {
			// Freshly created session files must be owner-only (umask can only
			// remove bits, so a 0600 create stays exactly 0600).
			expect(fs.statSync(file).mode & 0o777).toBe(0o600);
			expect(fs.statSync(path.dirname(file)).mode & 0o777).toBe(0o700);
		}
	});

	test("writeText on a pre-existing file preserves content and enforces private mode", async () => {
		using tempDir = TempDir.createSync("@omp-session-perms-");
		const storage = new FileSessionStorage();
		const file = path.join(tempDir.path(), "s.jsonl");
		await storage.writeText(file, "one\n");
		await storage.writeText(file, "two\n");
		expect(await storage.readText(file)).toBe("two\n");
		if (!isWindows) expect(fs.statSync(file).mode & 0o777).toBe(0o600);
	});

	test("terminal breadcrumb round-trips through a private file", async () => {
		using tempDir = TempDir.createSync("@omp-session-crumb-");
		const terminalId = `test-perms-${Snowflake.next()}`;
		const previousWtSession = process.env.WT_SESSION;
		restoreEnvValue("WT_SESSION", terminalId);
		const crumb = path.join(getTerminalSessionsDir(), terminalId);
		const sessionFile = path.join(tempDir.path(), "session.jsonl");
		try {
			await Bun.write(sessionFile, "{}\n");
			writeTerminalBreadcrumb(tempDir.path(), sessionFile);
			const entry = await readTerminalBreadcrumbEntry();
			expect(entry?.cwd).toBe(tempDir.path());
			expect(entry?.sessionFile).toBe(sessionFile);
			if (!isWindows) {
				expect(fs.statSync(crumb).mode & 0o777).toBe(0o600);
				expect(fs.statSync(getTerminalSessionsDir()).mode & 0o777).toBe(0o700);
			}
		} finally {
			restoreEnvValue("WT_SESSION", previousWtSession);
			fs.rmSync(crumb, { force: true });
		}
	});
});
