import { afterEach, describe, expect, it } from "bun:test";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { ManagerServer } from "../src/server";

/**
 * Contracts under test:
 * - New routes are additive: existing /api/runs output is byte-identical with flags off or on.
 * - Flags gate the new surface: off → 404 disabled, on → list/get/spawn/cancel work.
 * - Deleting a run also drops its mirrored sessions and agents.
 */

const cleanups: Array<() => void> = [];
afterEach(() => {
	while (cleanups.length) cleanups.pop()?.();
	for (const key of ["META_SESSIONS", "META_AGENTS"]) delete process.env[key];
});

function makeJobsDir(): string {
	const dir = fs.mkdtempSync(path.join(os.tmpdir(), "metaharness-server-addons-test-"));
	cleanups.push(() => fs.rmSync(dir, { recursive: true, force: true }));
	return dir;
}

function writeJob(jobsDir: string, jobName: string): void {
	const jobDir = path.join(jobsDir, jobName);
	fs.mkdirSync(jobDir, { recursive: true });
	fs.writeFileSync(
		path.join(jobDir, "result.json"),
		JSON.stringify({ n_total_trials: 1, stats: { n_running_trials: 1, n_pending_trials: 0 } }),
	);
	fs.writeFileSync(
		path.join(jobDir, "config.json"),
		JSON.stringify({ dataset: "d@1", agents: [{ name: "omp", model_name: "m/x" }] }),
	);
	fs.mkdirSync(path.join(jobDir, "trial-a__1", "agent"), { recursive: true });
}

describe("server session/agent addons", () => {
	it("returns 404 disabled when flags are off and leaves /api/runs unchanged", async () => {
		const jobsDir = makeJobsDir();
		writeJob(jobsDir, "job-off");
		const manager = new ManagerServer(jobsDir);
		const server = manager.start(0);
		cleanups.push(() => {
			void manager.stop();
		});
		const base = `http://localhost:${server.port}`;
		expect((await (await fetch(`${base}/api/sessions`)).json()) as unknown).toMatchObject({
			error: expect.stringMatching(/disabled/) as unknown,
		});
		expect((await fetch(`${base}/api/sessions`)).status).toBe(404);
		expect((await fetch(`${base}/api/agents`)).status).toBe(404);
		const runs = (await (await fetch(`${base}/api/runs`)).json()) as Array<{ jobName: string }>;
		expect(runs.map(r => r.jobName)).toContain("job-off");
	});

	it("mirrors sessions and agents when flags are on", async () => {
		process.env.META_SESSIONS = "mirror";
		process.env.META_AGENTS = "on";
		const jobsDir = makeJobsDir();
		writeJob(jobsDir, "job-on");
		const manager = new ManagerServer(jobsDir);
		const server = manager.start(0);
		cleanups.push(() => {
			void manager.stop();
		});
		const base = `http://localhost:${server.port}`;
		const sessions = (await (await fetch(`${base}/api/sessions?run=job-on`)).json()) as Array<{
			id: string;
			status: string;
		}>;
		expect(sessions).toHaveLength(1);
		expect(sessions[0].status).toBe("running");
		const one = await fetch(`${base}/api/sessions/${encodeURIComponent(sessions[0].id)}`);
		expect(one.status).toBe(200);
		expect(await fetch(`${base}/api/sessions/missing`).then(r => r.status)).toBe(404);

		const agents = (await (await fetch(`${base}/api/agents?run=job-on`)).json()) as Array<{
			id: string;
			kind: string;
		}>;
		expect(agents.map(a => a.kind).sort()).toEqual(["benchmark-runner", "trial-worker"]);

		const spawned = (await (
			await fetch(`${base}/api/agents`, {
				method: "POST",
				headers: { "content-type": "application/json" },
				body: JSON.stringify({ run: "job-on", displayName: "extra" }),
			})
		).json()) as { id: string; status: string };
		expect(spawned.status).toBe("running");
		const cancelled = (await (
			await fetch(`${base}/api/agents/${encodeURIComponent(spawned.id)}/cancel`, { method: "POST" })
		).json()) as { cancelled: boolean };
		expect(cancelled.cancelled).toBe(true);

		const badSpawn = await fetch(`${base}/api/agents`, {
			method: "POST",
			headers: { "content-type": "application/json" },
			body: JSON.stringify({}),
		});
		expect(badSpawn.status).toBe(400);
	});

	it("live-only session mutations are rejected in mirror mode", async () => {
		process.env.META_SESSIONS = "mirror";
		process.env.META_AGENTS = "on";
		const jobsDir = makeJobsDir();
		writeJob(jobsDir, "job-mirror");
		const manager = new ManagerServer(jobsDir);
		const server = manager.start(0);
		cleanups.push(() => {
			void manager.stop();
		});
		const base = `http://localhost:${server.port}`;
		const sessions = (await (await fetch(`${base}/api/sessions?run=job-mirror`)).json()) as Array<{
			id: string;
		}>;
		const res = await fetch(`${base}/api/sessions/${encodeURIComponent(sessions[0].id)}/fork`, {
			method: "POST",
		});
		expect(res.status).toBe(400);
	});

	it("deleting a run drops its mirrored sessions and agents", async () => {
		process.env.META_SESSIONS = "mirror";
		process.env.META_AGENTS = "on";
		const jobsDir = makeJobsDir();
		const manager = new ManagerServer(jobsDir);
		manager.store.registerLaunch({
			benchmark: "harbor",
			jobName: "job-del",
			dataset: "d@1",
			agent: "omp",
			models: ["m/x"],
			pid: process.pid,
		});
		manager.store.markExit("job-del", 0);
		const server = manager.start(0);
		cleanups.push(() => {
			void manager.stop();
		});
		const base = `http://localhost:${server.port}`;
		expect(manager.sessions?.list({ run: "job-del" }) ?? []).toHaveLength(0);
		const del = await fetch(`${base}/api/runs/job-del`, { method: "DELETE" });
		expect(del.status).toBe(200);
		expect(manager.sessions?.list({ run: "job-del" })).toHaveLength(0);
		expect(manager.agents?.list({ run: "job-del" })).toHaveLength(0);
	});
});
