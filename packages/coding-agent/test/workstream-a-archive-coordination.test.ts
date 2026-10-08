import { afterEach, describe, expect, it, vi } from "bun:test";
import * as fs from "node:fs/promises";
import * as os from "node:os";
import * as path from "node:path";
import type { AgentSession } from "../src/session/agent-session";
import type { AgentSessionEvent } from "../src/session/agent-session-events";
import { LiveSessionRegistry } from "../src/session/live-session-registry";
import { SessionManagementFacade } from "../src/session/session-management-facade";
import { SessionManager } from "../src/session/session-manager";
import { FileSessionStorage, type SessionStorage, type WriteTextAtomicOptions } from "../src/session/session-storage";
import { SessionTabs } from "../src/session/session-tabs";
import { SessionViewStateStore } from "../src/session/session-view-state";

function stubSession(manager: SessionManager): AgentSession {
	const listeners = new Set<(event: AgentSessionEvent) => void>();
	return {
		isStreaming: false,
		isBashRunning: false,
		isEvalRunning: false,
		isCompacting: false,
		hasPendingAsyncWork: () => false,
		sessionManager: manager,
		subscribe: (listener: (event: AgentSessionEvent) => void) => {
			listeners.add(listener);
			return () => listeners.delete(listener);
		},
		dispose: async () => {},
		abort: async () => {},
	} as unknown as AgentSession;
}

/** Storage double refusing the archive publish that carries a target ID, once. */
class FailTargetArchiveStorage extends FileSessionStorage {
	failuresLeft: number;
	targetId: string;
	constructor(targetId: string, failures: number) {
		super();
		this.targetId = targetId;
		this.failuresLeft = failures;
	}
	override async writeTextAtomic(fpath: string, content: string, options?: WriteTextAtomicOptions): Promise<void> {
		if (fpath.endsWith("archived-sessions.json") && this.failuresLeft > 0 && content.includes(this.targetId)) {
			this.failuresLeft--;
			throw new Error("archive publish refused");
		}
		return super.writeTextAtomic(fpath, content, options);
	}
}

describe("facade archive coordination across owners (T06)", () => {
	const dirs: string[] = [];
	afterEach(async () => {
		vi.restoreAllMocks();
		await Promise.all(dirs.splice(0).map(dir => fs.rm(dir, { recursive: true, force: true })));
	});

	async function makeProject(root: string, name: string) {
		const cwd = path.join(root, name);
		const sessionDir = path.join(cwd, "sessions");
		await fs.mkdir(sessionDir, { recursive: true });
		return { cwd, sessionDir };
	}

	async function makeLive(project: { cwd: string; sessionDir: string }, storage: SessionStorage) {
		const manager = SessionManager.create(project.cwd, project.sessionDir, storage);
		await manager.ensureOnDisk();
		const session = stubSession(manager);
		const registry = new LiveSessionRegistry(session, async () => {
			throw new Error("no cold opens");
		});
		return { manager, registry, id: manager.getSessionId() };
	}

	function makeFacade(
		project: { cwd: string; sessionDir: string },
		registry: LiveSessionRegistry,
		storage: SessionStorage,
		legacyAgentDir: string,
	) {
		return new SessionManagementFacade({
			live: registry,
			tabs: new SessionTabs(),
			viewState: new SessionViewStateStore(),
			storage,
			createSession: async () => {
				throw new Error("no creation");
			},
			openSession: async () => {
				throw new Error("no cold opens");
			},
			directories: { cwd: project.cwd, sessionDir: project.sessionDir },
			legacyAgentDir,
		});
	}

	async function readDiskIds(sessionDir: string): Promise<string[]> {
		const raw = (await Bun.file(path.join(sessionDir, "archived-sessions.json")).json()) as { ids: string[] };
		return raw.ids;
	}

	it("two owners archiving concurrently keep both additions", async () => {
		const root = await fs.mkdtemp(path.join(os.tmpdir(), "harvest-archive-coord-"));
		dirs.push(root);
		const storage = new FileSessionStorage();
		const project = await makeProject(root, "proj");
		const legacyAgentDir = path.join(root, "agent");
		await fs.mkdir(legacyAgentDir, { recursive: true });
		const liveA = await makeLive(project, storage);
		const liveB = await makeLive(project, storage);
		const facadeA = makeFacade(project, liveA.registry, storage, legacyAgentDir);
		const facadeB = makeFacade(project, liveB.registry, storage, legacyAgentDir);

		await Promise.all([facadeA.archive(liveA.id), facadeB.archive(liveB.id)]);
		const [idsA, idsB] = await Promise.all([facadeA.archivedIds(), facadeB.archivedIds()]);
		expect([...idsA].sort()).toEqual([liveA.id, liveB.id].sort());
		expect([...idsB].sort()).toEqual([liveA.id, liveB.id].sort());
		expect((await readDiskIds(project.sessionDir)).sort()).toEqual([liveA.id, liveB.id].sort());
	});

	it("a concurrent restore is never resurrected by a sibling addition", async () => {
		const root = await fs.mkdtemp(path.join(os.tmpdir(), "harvest-archive-restore-"));
		dirs.push(root);
		const storage = new FileSessionStorage();
		const project = await makeProject(root, "proj");
		const legacyAgentDir = path.join(root, "agent");
		await fs.mkdir(legacyAgentDir, { recursive: true });
		const liveA = await makeLive(project, storage);
		const liveB = await makeLive(project, storage);
		const liveC = await makeLive(project, storage);
		const facadeA = makeFacade(project, liveA.registry, storage, legacyAgentDir);
		const facadeB = makeFacade(project, liveB.registry, storage, legacyAgentDir);

		await facadeA.archive(liveA.id);
		// Restore of A races an unrelated addition of C: the union must not resurrect A.
		await Promise.all([facadeB.restore(liveA.id), facadeA.archive(liveC.id)]);
		const [idsA, idsB] = await Promise.all([facadeA.archivedIds(), facadeB.archivedIds()]);
		expect(idsA.has(liveA.id)).toBe(false);
		expect(idsB.has(liveA.id)).toBe(false);
		expect(idsA.has(liveC.id)).toBe(true);
		expect(idsB.has(liveC.id)).toBe(true);
		expect((await readDiskIds(project.sessionDir)).sort()).toEqual([liveC.id].sort());
	});

	it("failed publishes preserve input and leave a truthful cache; retry succeeds", async () => {
		const root = await fs.mkdtemp(path.join(os.tmpdir(), "harvest-archive-fail-"));
		dirs.push(root);
		const backing = new FileSessionStorage();
		const project = await makeProject(root, "proj");
		const legacyAgentDir = path.join(root, "agent");
		await fs.mkdir(legacyAgentDir, { recursive: true });
		// Seed live sessions first so the failing wrapper can target A's ID.
		const seedA = await makeLive(project, backing);
		const storage = new FailTargetArchiveStorage(seedA.id, 1);
		const liveA = seedA;
		const facadeA = makeFacade(project, liveA.registry, storage, legacyAgentDir);
		const liveB = await makeLive(project, storage);
		const facadeB = makeFacade(project, liveB.registry, storage, legacyAgentDir);

		await facadeB.archive(liveB.id);
		await expect(facadeA.archive(liveA.id)).rejects.toThrow(/archive publish refused/);
		// Input preserved (B's entry intact on disk), failed write not cached.
		expect(await readDiskIds(project.sessionDir)).toEqual([liveB.id]);
		expect((await facadeA.archivedIds()).has(liveA.id)).toBe(false);
		expect((await facadeA.archivedIds()).has(liveB.id)).toBe(true);
		// Retry on the healthy path succeeds and both owners converge.
		await facadeA.archive(liveA.id);
		expect((await readDiskIds(project.sessionDir)).sort()).toEqual([liveA.id, liveB.id].sort());
		expect((await facadeB.archivedIds()).has(liveA.id)).toBe(true);
	});

	it("a corrupt archive file is reconstructed instead of clobbering siblings", async () => {
		const root = await fs.mkdtemp(path.join(os.tmpdir(), "harvest-archive-corrupt-"));
		dirs.push(root);
		const storage = new FileSessionStorage();
		const project = await makeProject(root, "proj");
		const legacyAgentDir = path.join(root, "agent");
		await fs.mkdir(legacyAgentDir, { recursive: true });
		const liveA = await makeLive(project, storage);
		const liveB = await makeLive(project, storage);
		const facadeA = makeFacade(project, liveA.registry, storage, legacyAgentDir);
		const facadeB = makeFacade(project, liveB.registry, storage, legacyAgentDir);

		await facadeA.archive(liveA.id);
		await Bun.write(path.join(project.sessionDir, "archived-sessions.json"), "{corrupt");
		await Promise.all([facadeA.archive(liveB.id), facadeB.restore(liveA.id)]);
		const disk = await readDiskIds(project.sessionDir);
		expect(disk).toEqual([liveB.id]);
		expect((await facadeA.archivedIds()).has(liveB.id)).toBe(true);
	});

	it("archives never leak across projects", async () => {
		const root = await fs.mkdtemp(path.join(os.tmpdir(), "harvest-archive-xproj-"));
		dirs.push(root);
		const storage = new FileSessionStorage();
		const legacyAgentDir = path.join(root, "agent");
		await fs.mkdir(legacyAgentDir, { recursive: true });
		const projectA = await makeProject(root, "proj-a");
		const projectB = await makeProject(root, "proj-b");
		const liveA = await makeLive(projectA, storage);
		const liveB = await makeLive(projectB, storage);
		const facadeA = makeFacade(projectA, liveA.registry, storage, legacyAgentDir);
		const facadeB = makeFacade(projectB, liveB.registry, storage, legacyAgentDir);

		await facadeA.archive(liveA.id);
		expect((await facadeB.archivedIds()).has(liveA.id)).toBe(false);
		expect((await facadeA.archivedIds()).has(liveB.id)).toBe(false);
	});
});
