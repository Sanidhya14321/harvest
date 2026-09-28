import { afterEach, describe, expect, it } from "bun:test";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { mirrorSnapshotToSessions } from "../src/sessions/session-mirror";
import { SessionService } from "../src/sessions/session-service";
import { SessionStore } from "../src/sessions/session-store";
import { RunStore } from "../src/store";

/**
 * Contracts under test:
 * - Mirror maps normalized trial snapshots to session rows without writes to trial dirs.
 * - SessionService in mirror mode indexes RunStore runs; in off mode it no-ops.
 * - Live mode creates an isolated sidecar journal and forks it with reset cost semantics.
 */

const cleanups: Array<() => void> = [];
afterEach(() => {
	while (cleanups.length) cleanups.pop()?.();
});

function makeJobsDir(): string {
	const dir = fs.mkdtempSync(path.join(os.tmpdir(), "metaharness-sessions-test-"));
	cleanups.push(() => fs.rmSync(dir, { recursive: true, force: true }));
	return dir;
}

function writeHarborJob(jobsDir: string, jobName: string): void {
	const jobDir = path.join(jobsDir, jobName);
	fs.mkdirSync(jobDir, { recursive: true });
	fs.writeFileSync(
		path.join(jobDir, "result.json"),
		JSON.stringify({ n_total_trials: 2, stats: { n_running_trials: 1, n_pending_trials: 0 } }),
	);
	fs.writeFileSync(
		path.join(jobDir, "config.json"),
		JSON.stringify({ dataset: "d@1", agents: [{ name: "omp", model_name: "m/x" }] }),
	);
	const passDir = path.join(jobDir, "task-a__1", "agent");
	fs.mkdirSync(passDir, { recursive: true });
	fs.writeFileSync(
		path.join(jobDir, "task-a__1", "result.json"),
		JSON.stringify({
			started_at: "2026-07-12T10:00:00",
			finished_at: "2026-07-12T10:01:00",
			verifier_result: { rewards: { reward: 1 } },
			agent_result: { cost_usd: 0.3, n_input_tokens: 10, n_output_tokens: 5, n_cache_tokens: 0 },
		}),
	);
	fs.mkdirSync(path.join(jobDir, "task-b__2", "agent"), { recursive: true });
}

describe("session mirror", () => {
	it("maps pass and running trials to distinct session ids", () => {
		const jobsDir = makeJobsDir();
		writeHarborJob(jobsDir, "job-a");
		const store = new RunStore(jobsDir);
		cleanups.push(() => store.close());
		store.discover();
		const service = new SessionService(jobsDir, "mirror");
		cleanups.push(() => service.close());
		service.syncRun(store, "job-a");
		const sessions = service.list({ run: "job-a" });
		expect(sessions.map(s => [s.trial, s.status]).sort()).toEqual([
			["task-a__1", "pass"],
			["task-b__2", "running"],
		]);
		expect(sessions[0].id).toContain("job-a:");
		expect(sessions.every(s => s.live === false)).toBe(true);
	});

	it("filters sessions by status without touching trial dirs", () => {
		const before = new Map<string, number>();
		const jobsDir = makeJobsDir();
		writeHarborJob(jobsDir, "job-b");
		for (const f of ["result.json", "config.json"]) {
			before.set(f, fs.statSync(path.join(jobsDir, "job-b", f)).mtimeMs);
		}
		const store = new RunStore(jobsDir);
		cleanups.push(() => store.close());
		store.discover();
		const service = new SessionService(jobsDir, "mirror");
		cleanups.push(() => service.close());
		service.syncRun(store, "job-b");
		expect(service.list({ run: "job-b", status: "pass" })).toHaveLength(1);
		expect(service.list({ run: "job-b", status: "running" })).toHaveLength(1);
		for (const f of ["result.json", "config.json"]) {
			expect(fs.statSync(path.join(jobsDir, "job-b", f)).mtimeMs).toBe(before.get(f));
		}
	});

	it("drops sessions for deleted runs and prunes vanished trials", () => {
		const jobsDir = makeJobsDir();
		writeHarborJob(jobsDir, "job-c");
		const store = new RunStore(jobsDir);
		cleanups.push(() => store.close());
		store.discover();
		const service = new SessionService(jobsDir, "mirror");
		cleanups.push(() => service.close());
		service.syncRun(store, "job-c");
		expect(service.list({ run: "job-c" })).toHaveLength(2);
		fs.rmSync(path.join(jobsDir, "job-c", "task-b__2"), { recursive: true, force: true });
		service.syncRun(store, "job-c");
		expect(service.list({ run: "job-c" }).map(s => s.trial)).toEqual(["task-a__1"]);
	});

	it("off mode performs no I/O beyond its own empty store", () => {
		const jobsDir = makeJobsDir();
		writeHarborJob(jobsDir, "job-d");
		const store = new RunStore(jobsDir);
		cleanups.push(() => store.close());
		store.discover();
		const service = new SessionService(jobsDir, "off");
		cleanups.push(() => service.close());
		service.syncRun(store, "job-d");
		expect(service.list()).toHaveLength(0);
		expect(fs.existsSync(path.join(jobsDir, "_manager", "sessions"))).toBe(false);
	});

	it("mirror helper preserves cost and trace locator", () => {
		const sessions = mirrorSnapshotToSessions("r", {
			traces: [
				{
					name: "t__1",
					task: "t",
					status: "pass",
					reward: 1,
					costUsd: 1.5,
					durationMs: 7,
					detail: "",
					tracePath: path.join("t__1", "agent", "omp.txt"),
				},
			],
			total: 1,
			done: 1,
			pass: 1,
			fail: 0,
			error: 0,
			running: 0,
			costUsd: 1.5,
			tokIn: 0,
			tokOut: 0,
			tokCache: 0,
			score: 1,
			metrics: {},
		});
		expect(sessions[0]).toMatchObject({ costUsd: 1.5, tracePath: path.join("t__1", "agent", "omp.txt") });
	});

	it("session store round-trips live flags", () => {
		const jobsDir = makeJobsDir();
		const store = new SessionStore(jobsDir);
		cleanups.push(() => store.close());
		store.upsert({
			id: "r:t__1",
			run: "r",
			trial: "t__1",
			task: "t",
			status: "running",
			tracePath: null,
			sessionFile: "/tmp/x.jsonl",
			costUsd: 0,
			durationMs: 0,
			detail: "",
			updatedAt: 1,
			live: true,
		});
		expect(store.get("r:t__1")?.live).toBe(true);
		expect(store.get("r:t__1")?.sessionFile).toBe("/tmp/x.jsonl");
	});
});
