/**
 * Harness agent service: isolated registry facade over benchmark workers.
 *
 * Uses its own `AgentRegistry` instance (never `AgentRegistry.global()`) so a
 * locally running `omp` CLI is unaffected. The coding-agent registry is loaded
 * lazily — mirror/sync paths never import it.
 *
 * Agents are logical by default (no process). `spawn` optionally starts a
 * detached child for ad-hoc workers; benchmark trial processes remain owned by
 * `ManagerServer` — this service only mirrors them.
 */
import * as fs from "node:fs";
import * as path from "node:path";
import type { AgentLifecycleManager } from "@harvest/pi-coding-agent/registry/agent-lifecycle";
import type { AgentRegistry } from "@harvest/pi-coding-agent/registry/agent-registry";
import { readBenchmarkSnapshot } from "../benchmarks";
import type { RunStore } from "../store";
import { AgentStore } from "./agent-store";
import type { HarnessAgent, HarnessAgentKind, HarnessAgentListFilter, HarnessAgentStatus } from "./types";

export type AgentsMode = "off" | "on";

export function agentsModeFromEnv(env: NodeJS.ProcessEnv = process.env): AgentsMode {
	const raw = (env.META_AGENTS ?? "off").toLowerCase();
	return raw === "on" || raw === "1" ? "on" : "off";
}

export function assertSafeAgentId(id: string): void {
	if (!id || id === "." || id === ".." || id.includes("/") || id.includes("\\")) {
		throw new Error(`invalid agent id: ${id}`);
	}
	if (id.length > 256) throw new Error(`invalid agent id: too long`);
}

function processAlive(pid: number | null): boolean {
	if (pid == null) return false;
	try {
		process.kill(pid, 0);
		return true;
	} catch {
		return false;
	}
}

type RegistryLike = Pick<AgentRegistry, "register" | "setStatus" | "detachSession" | "unregister" | "get">;

type LifecycleLike = Pick<AgentLifecycleManager, "adopt" | "release">;

export interface SpawnAgentRequest {
	run: string;
	trial?: string;
	displayName?: string;
	kind?: HarnessAgentKind;
	/** Optional command to run as a detached worker, e.g. ["bun", "worker.ts"]. */
	command?: string[];
	cwd?: string;
}

export class AgentService {
	readonly mode: AgentsMode;
	readonly jobsDir: string;
	readonly store: AgentStore;
	#registry: RegistryLike | null = null;
	#lifecycle: LifecycleLike | null = null;
	#children = new Map<string, number>();

	constructor(jobsDir: string, mode: AgentsMode = agentsModeFromEnv(), dbPath?: string) {
		this.jobsDir = jobsDir;
		this.mode = mode;
		this.store = new AgentStore(jobsDir, dbPath);
	}

	get enabled(): boolean {
		return this.mode === "on";
	}

	close(): void {
		this.store.close();
	}

	async #ensureRegistry(): Promise<{ registry: RegistryLike; lifecycle: LifecycleLike }> {
		if (this.#registry && this.#lifecycle) return { registry: this.#registry, lifecycle: this.#lifecycle };
		const [{ AgentRegistry: RegistryCtor }, { AgentLifecycleManager: LifecycleCtor }] = await Promise.all([
			import("@harvest/pi-coding-agent/registry/agent-registry"),
			import("@harvest/pi-coding-agent/registry/agent-lifecycle"),
		]);
		const registry = new RegistryCtor() as RegistryLike;
		const lifecycle = new LifecycleCtor(
			registry as unknown as AgentRegistry,
		) as unknown as LifecycleLike;
		this.#registry = registry;
		this.#lifecycle = lifecycle;
		return { registry, lifecycle };
	}

	/** Mirror running runs + running trials into agent rows. No-op when off. */
	syncFromRuns(runStore: RunStore): void {
		if (!this.enabled) return;
		const now = Date.now();
		const seen = new Set<string>();
		for (const run of runStore.listRuns()) {
			const runnerId = `runner:${run.jobName}`;
			seen.add(runnerId);
			const alive = run.status === "running" && (run.pid == null || processAlive(run.pid));
			const existingRunner = this.store.get(runnerId);
			this.store.upsert({
				id: runnerId,
				displayName: `runner ${run.jobName}`,
				kind: "benchmark-runner",
				run: run.jobName,
				trial: null,
				status: run.status === "running" ? (alive ? "running" : "running") : "idle",
				pid: run.pid,
				sessionId: null,
				sessionFile: null,
				createdAt: existingRunner?.createdAt ?? run.createdAt,
				lastActivity: now,
			});
			if (run.status !== "running") continue;
			const jobDir = path.join(this.jobsDir, run.jobName);
			const snapshot = readBenchmarkSnapshot(run.benchmark, jobDir);
			for (const trace of snapshot.traces) {
				if (trace.status !== "running") continue;
				const id = `trial:${run.jobName}:${trace.name}`;
				seen.add(id);
				const existing = this.store.get(id);
				this.store.upsert({
					id,
					displayName: `trial ${trace.name}`,
					kind: "trial-worker",
					run: run.jobName,
					trial: trace.name,
					status: "running",
					pid: null,
					sessionId: `${run.jobName}:${trace.name}`,
					sessionFile: null,
					createdAt: existing?.createdAt ?? now,
					lastActivity: now,
				});
			}
		}
		for (const agent of this.store.list()) {
			if (seen.has(agent.id)) continue;
			if (agent.status === "running" && agent.kind === "trial-worker") {
				this.store.upsert({ ...agent, status: "idle", lastActivity: now });
			}
		}
	}

	deleteRun(jobName: string): void {
		if (!this.enabled) return;
		for (const agent of this.store.list({ run: jobName })) this.store.remove(agent.id);
	}

	list(filter: HarnessAgentListFilter = {}): HarnessAgent[] {
		return this.store.list(filter);
	}

	get(id: string): HarnessAgent | null {
		return this.store.get(id);
	}

	/** Register a logical or detached-process worker. Requires agents enabled. */
	async spawn(req: SpawnAgentRequest): Promise<HarnessAgent> {
		if (!this.enabled) throw new Error("agents are disabled (set META_AGENTS=on)");
		const kind: HarnessAgentKind = req.kind ?? "trial-worker";
		const base = req.trial ? `trial:${req.run}:${req.trial}` : `worker:${req.run}:${Date.now()}`;
		const id = base;
		assertSafeAgentId(id.replace(/:/g, "-").slice(0, 200) || id);
		const now = Date.now();
		let pid: number | null = null;
		if (req.command && req.command.length > 0) {
			const logDir = path.join(this.jobsDir, "_manager", "logs");
			fs.mkdirSync(logDir, { recursive: true });
			const logFile = fs.openSync(path.join(logDir, `agent-${Date.now()}.log`), "w");
			const proc = Bun.spawn(req.command, {
				cwd: req.cwd ?? this.jobsDir,
				stdout: logFile,
				stderr: logFile,
				env: { ...process.env },
				detached: true,
			});
			pid = proc.pid;
			this.#children.set(id, pid);
			proc.exited.then(() => {
				try {
					fs.closeSync(logFile);
				} catch {}
				this.#children.delete(id);
				const current = this.store.get(id);
				if (current && current.status === "running") {
					this.store.upsert({ ...current, status: "idle", pid: null, lastActivity: Date.now() });
				}
			});
		}
		const agent: HarnessAgent = {
			id,
			displayName: req.displayName ?? (req.trial ? `trial ${req.trial}` : `worker ${req.run}`),
			kind,
			run: req.run,
			trial: req.trial ?? null,
			status: "running",
			pid,
			sessionId: req.trial ? `${req.run}:${req.trial}` : null,
			sessionFile: null,
			createdAt: now,
			lastActivity: now,
		};
		this.store.upsert(agent);
		try {
			const { registry, lifecycle } = await this.#ensureRegistry();
			registry.register({
				id,
				displayName: agent.displayName,
				kind: "sub",
				session: null,
				sessionFile: null,
				status: "running",
			});
			lifecycle.adopt(id, { idleTtlMs: 0 });
		} catch {
			// Registry mirror is best-effort; the store row is authoritative.
		}
		return agent;
	}

	/** Cancel a worker: SIGTERM, escalate to SIGKILL after 5s. Returns false when unknown. */
	async cancel(id: string): Promise<boolean> {
		const agent = this.store.get(id);
		if (!agent) return false;
		const pid = agent.pid ?? this.#children.get(id) ?? null;
		if (pid != null && processAlive(pid)) {
			try {
				process.kill(pid, "SIGTERM");
			} catch {}
			setTimeout(() => {
				try {
					if (processAlive(pid)) process.kill(pid, "SIGKILL");
				} catch {}
			}, 5000).unref?.();
		}
		this.#children.delete(id);
		const next: HarnessAgentStatus = "aborted";
		this.store.upsert({ ...agent, status: next, pid: null, lastActivity: Date.now() });
		try {
			const { registry, lifecycle } = await this.#ensureRegistry();
			registry.detachSession(id);
			await lifecycle.release(id, undefined, { tombstone: true });
		} catch {}
		return true;
	}

	/** Park a running/idle worker (keeps ref + sessionFile for revival). */
	async park(id: string): Promise<HarnessAgent> {
		const agent = this.store.get(id);
		if (!agent) throw new Error(`agent ${id} not found`);
		const next: HarnessAgent = { ...agent, status: "parked", lastActivity: Date.now() };
		this.store.upsert(next);
		try {
			const { registry } = await this.#ensureRegistry();
			registry.detachSession(id);
			registry.setStatus(id, "parked");
		} catch {}
		return next;
	}

	/** Revive a parked worker to idle. */
	async revive(id: string): Promise<HarnessAgent> {
		const agent = this.store.get(id);
		if (!agent) throw new Error(`agent ${id} not found`);
		if (agent.status !== "parked") throw new Error(`agent ${id} is ${agent.status}, not parked`);
		const next: HarnessAgent = { ...agent, status: "idle", lastActivity: Date.now() };
		this.store.upsert(next);
		try {
			const { registry } = await this.#ensureRegistry();
			registry.setStatus(id, "idle");
		} catch {}
		return next;
	}
}
