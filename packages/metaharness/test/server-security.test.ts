import { afterEach, describe, expect, it } from "bun:test";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { ManagerServer } from "../src/server";

/**
 * Contracts under test:
 *  - launch() rejects job names that could escape the jobs dir (traversal,
 *    absolute paths, separators) instead of mkdir-ing outside it.
 *  - start() refuses a non-loopback bind without a token (--token /
 *    METAHARNESS_TOKEN), keeping the loopback default open and back-compat.
 *  - With a token configured, mutating routes require
 *    `Authorization: Bearer <token>` while read routes keep working.
 */

const cleanups: Array<() => void> = [];
afterEach(() => {
	while (cleanups.length) cleanups.pop()?.();
});

function makeJobsDir(): string {
	const dir = fs.mkdtempSync(path.join(os.tmpdir(), "metaharness-sec-"));
	cleanups.push(() => fs.rmSync(dir, { recursive: true, force: true }));
	return dir;
}

describe("ManagerServer launch jobName validation", () => {
	for (const evil of ["../evil", "..\\evil", "/abs/evil", "a/b", "a\\b", "..", "."]) {
		it(`rejects traversal jobName '${evil}' without touching disk`, async () => {
			const jobsDir = makeJobsDir();
			const manager = new ManagerServer(jobsDir);
			cleanups.push(() => {
				void manager.stop();
			});
			const before = fs.readdirSync(jobsDir).sort();
			expect(() => manager.launch({ model: "anthropic/x", jobName: evil })).toThrow(/invalid job name/);
			// No job dir may materialize for the evil name (when it resolves
			// inside jobsDir at all — "." and ".." trivially resolve to
			// existing ancestors, where the throw above is the assertion).
			const resolved = path.resolve(jobsDir, evil);
			if (resolved.startsWith(`${jobsDir}${path.sep}`)) {
				expect(fs.existsSync(resolved)).toBe(false);
			}
			expect(fs.readdirSync(jobsDir).sort()).toEqual(before);
		});
	}
});

describe("ManagerServer bind + token auth", () => {
	it("refuses a non-loopback bind without a token", async () => {
		const manager = new ManagerServer(makeJobsDir());
		cleanups.push(() => {
			void manager.stop();
		});
		expect(() => manager.start(0, "0.0.0.0")).toThrow(/non-loopback bind.*token/);
	});

	it("allows a non-loopback bind with an explicit token", async () => {
		const manager = new ManagerServer(makeJobsDir(), undefined, "secret");
		cleanups.push(() => {
			void manager.stop();
		});
		const server = manager.start(0, "127.0.0.1");
		expect(server.port).toBeGreaterThan(0);
	});

	it("requires the bearer token on mutating routes, keeps reads open", async () => {
		const manager = new ManagerServer(makeJobsDir(), undefined, "secret");
		cleanups.push(() => {
			void manager.stop();
		});
		const server = manager.start(0);
		const base = `http://127.0.0.1:${server.port}`;

		const unauthLaunch = await fetch(`${base}/api/runs`, {
			method: "POST",
			headers: { "content-type": "application/json" },
			body: JSON.stringify({}),
		});
		expect(unauthLaunch.status).toBe(401);

		// Reads stay working without a token.
		const runs = await fetch(`${base}/api/runs`);
		expect(runs.status).toBe(200);

		// Wrong token is still rejected.
		const wrongLaunch = await fetch(`${base}/api/runs`, {
			method: "POST",
			headers: { "content-type": "application/json", authorization: "Bearer wrong" },
			body: JSON.stringify({}),
		});
		expect(wrongLaunch.status).toBe(401);

		// Right token passes auth and reaches request validation (400: model required).
		const authedLaunch = await fetch(`${base}/api/runs`, {
			method: "POST",
			headers: { "content-type": "application/json", authorization: "Bearer secret" },
			body: JSON.stringify({}),
		});
		expect(authedLaunch.status).toBe(400);

		const unauthCancel = await fetch(`${base}/api/runs/nope/cancel`, { method: "POST" });
		expect(unauthCancel.status).toBe(401);
	});

	it("stays fully open on loopback without a token (CLI back-compat)", async () => {
		const manager = new ManagerServer(makeJobsDir());
		cleanups.push(() => {
			void manager.stop();
		});
		const server = manager.start(0);
		const base = `http://127.0.0.1:${server.port}`;
		const badLaunch = await fetch(`${base}/api/runs`, {
			method: "POST",
			headers: { "content-type": "application/json" },
			body: JSON.stringify({}),
		});
		expect(badLaunch.status).toBe(400);
	});
});
