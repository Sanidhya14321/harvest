import { afterEach, describe, expect, it } from "bun:test";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { AgentService } from "../src/agents/agent-service";
import { AgentStore } from "../src/agents/agent-store";
import { RunStore } from "../src/store";

/**
 * Contracts under test:
 * - syncFromRuns mirrors running runs and running trials, demotes finished trial-workers to idle.
 * - spawn/cancel/park/revive drive the store lifecycle; cancel kills a real child pid.
 * - Off mode disables mutation entry points.
 */

const cleanups: Array<() => void> = [];
afterEach(() => {
	while (cleanups.length) cleanups.pop()?.();
});

function makeJobsDir(): string {
	const dir = fs.mkdtempSync(path.join(os.tmpdir(), "metaharness-agents-test-"));
	cleanups.push(() => fs.rmSync(dir, { recursive: true, force: true }));
	return dir;
}

function writeRunningHarborJob(jobsDir: string, jobName: string): void {
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
	fs.mkdirSync(path.join(jobDir, "live-trial__1", "agent"), { recursive: true });
	const doneDir = path.join(jobDir, "done-trial__2");
	fs.mkdirSync(path.join(doneDir, "agent"), { recursive: true });
	fs.writeFileSync(
		path.join(doneDir, "result.json"),
		JSON.stringify({
			started_at: "2026-07-12T10:00:00",
			finished_at: "2026-07-12T10:01:00",
			verifier_result: { rewards: { reward: 1 } },
			agent_result: { cost_usd: 0.1 },
		}),
	);
}

describe("agent service", () => {
	it("mirrors one runner per run and one worker per running trial", () => {
		const jobsDir = makeJobsDir();
		writeRunningHarborJob(jobsDir, "job-a");
		const runs = new RunStore(jobsDir);
		cleanups.push(() => runs.close());
		runs.discover();
		const agents = new AgentService(jobsDir, "on");
		cleanups.push(() => agents.close());
		agents.syncFromRuns(runs);
		const rows = agents.list({ run: "job-a" });
		expect(rows.map(r => r.kind).sort()).toEqual(["benchmark-runner", "trial-worker"]);
		expect(rows.find(r => r.kind === "trial-worker")?.trial).toBe("live-trial__1");
		expect(rows.every(r => r.status === "running")).toBe(true);
	});

	it("demotes a finished trial-worker to idle on the next sync", () => {
		const jobsDir = makeJobsDir();
		writeRunningHarborJob(jobsDir, "job-b");
		const runs = new RunStore(jobsDir);
		cleanups.push(() => runs.close());
		runs.discover();
		const agents = new AgentService(jobsDir, "on");
		cleanups.push(() => agents.close());
		agents.syncFromRuns(runs);
		expect(agents.list({ run: "job-b", status: "running" })).toHaveLength(2);
		fs.writeFileSync(
			path.join(jobsDir, "job-b", "live-trial__1", "result.json"),
			JSON.stringify({
				started_at: "2026-07-12T10:00:00",
				finished_at: "2026-07-12T10:02:00",
				verifier_result: { rewards: { reward: 0 } },
			}),
		);
		agents.syncFromRuns(runs);
		const idle = agents.list({ run: "job-b", status: "idle" });
		expect(idle.map(a => a.trial)).toContain("live-trial__1");
	});

	it("spawns a logical worker then parks and revives it", async () => {
		const jobsDir = makeJobsDir();
		const agents = new AgentService(jobsDir, "on");
		cleanups.push(() => agents.close());
		const spawned = await agents.spawn({ run: "job-c", trial: "t__1", displayName: "t worker" });
		expect(spawned.status).toBe("running");
		expect(agents.get(spawned.id)?.status).toBe("running");
		const parked = await agents.park(spawned.id);
		expect(parked.status).toBe("parked");
		const revived = await agents.revive(spawned.id);
		expect(revived.status).toBe("idle");
	});

	it("cancel marks a spawned child aborted and reaps its pid", async () => {
		const jobsDir = makeJobsDir();
		const agents = new AgentService(jobsDir, "on");
		cleanups.push(() => agents.close());
		const spawned = await agents.spawn({
			run: "job-d",
			displayName: "sleeper",
			command: ["bun", "-e", "await new Promise(() => {})"],
		});
		expect(spawned.pid).not.toBeNull();
		expect(await agents.cancel(spawned.id)).toBe(true);
		expect(agents.get(spawned.id)?.status).toBe("aborted");
		expect(await agents.cancel("missing")).toBe(false);
	});

	it("deleteRun removes only that run's agents", () => {
		const jobsDir = makeJobsDir();
		const agents = new AgentService(jobsDir, "on");
		cleanups.push(() => agents.close());
		agents.store.upsert({
			id: "a1",
			displayName: "a1",
			kind: "benchmark-runner",
			run: "r1",
			trial: null,
			status: "running",
			pid: null,
			sessionId: null,
			sessionFile: null,
			createdAt: 1,
			lastActivity: 1,
		});
		agents.store.upsert({
			id: "a2",
			displayName: "a2",
			kind: "benchmark-runner",
			run: "r2",
			trial: null,
			status: "running",
			pid: null,
			sessionId: null,
			sessionFile: null,
			createdAt: 1,
			lastActivity: 1,
		});
		agents.deleteRun("r1");
		expect(agents.get("a1")).toBeNull();
		expect(agents.get("a2")).not.toBeNull();
	});

	it("off mode refuses spawn", async () => {
		const jobsDir = makeJobsDir();
		const agents = new AgentService(jobsDir, "off");
		cleanups.push(() => agents.close());
		await expect(agents.spawn({ run: "r" })).rejects.toThrow(/disabled/);
	});

	it("agent store round-trips pid and session linkage", () => {
		const jobsDir = makeJobsDir();
		const store = new AgentStore(jobsDir);
		cleanups.push(() => store.close());
		store.upsert({
			id: "w1",
			displayName: "w1",
			kind: "trial-worker",
			run: "r",
			trial: "t",
			status: "running",
			pid: 1234,
			sessionId: "r:t",
			sessionFile: "/tmp/s.jsonl",
			createdAt: 5,
			lastActivity: 6,
		});
		expect(store.get("w1")).toMatchObject({ pid: 1234, sessionId: "r:t", sessionFile: "/tmp/s.jsonl" });
	});
});
