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

/** Storage double that refuses to publish the merged archive. */
class FailingArchivePublishStorage extends FileSessionStorage {
	override async writeTextAtomic(fpath: string, content: string, options?: WriteTextAtomicOptions): Promise<void> {
		if (fpath.endsWith("archived-sessions.json")) throw new Error("archive publish refused");
		return super.writeTextAtomic(fpath, content, options);
	}
}

describe("facade legacy archive migration (A3)", () => {
	const dirs: string[] = [];
	afterEach(async () => {
		vi.restoreAllMocks();
		await Promise.all(dirs.splice(0).map(dir => fs.rm(dir, { recursive: true, force: true })));
	});

	async function makeProject(root: string, name: string, storage: SessionStorage) {
		const cwd = path.join(root, name);
		const sessionDir = path.join(cwd, "sessions");
		await fs.mkdir(sessionDir, { recursive: true });
		const manager = SessionManager.create(cwd, sessionDir, storage);
		await manager.ensureOnDisk();
		const registry = new LiveSessionRegistry(stubSession(manager), async () => {
			throw new Error("no cold opens");
		});
		return { cwd, sessionDir, id: manager.getSessionId(), registry };
	}

	function makeFacade(
		project: { cwd: string; sessionDir: string; registry: LiveSessionRegistry },
		legacyAgentDir: string,
		storage: SessionStorage,
	) {
		return new SessionManagementFacade({
			live: project.registry,
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

	it("attributes legacy IDs per project and never discards other-project IDs", async () => {
		const root = await fs.mkdtemp(path.join(os.tmpdir(), "harvest-archive-migrate-"));
		dirs.push(root);
		const storage = new FileSessionStorage();
		const legacyAgentDir = path.join(root, "agent");
		await fs.mkdir(legacyAgentDir, { recursive: true });
		const projectA = await makeProject(root, "proj-a", storage);
		const projectB = await makeProject(root, "proj-b", storage);
		const legacyPath = path.join(legacyAgentDir, "session-archive.json");
		await Bun.write(legacyPath, JSON.stringify([projectA.id, projectB.id, "ghost-unknown"]));

		const facadeA = makeFacade(projectA, legacyAgentDir, storage);
		await expect(facadeA.archivedIds()).resolves.toContain(projectA.id);
		expect(await facadeA.archivedIds().then(ids => ids.has(projectB.id))).toBe(false);
		// Published atomically to A's store before the legacy source was touched.
		const publishedA = (await Bun.file(path.join(projectA.sessionDir, "archived-sessions.json")).json()) as {
			version: number;
			ids: string[];
		};
		expect(publishedA.version).toBe(1);
		expect(publishedA.ids).toContain(projectA.id);
		// Other-project IDs stay in the legacy file; A's ID is consumed.
		const remainderA = (await Bun.file(legacyPath).json()) as string[];
		expect(remainderA.sort()).toEqual([projectB.id, "ghost-unknown"].sort());

		const facadeB = makeFacade(projectB, legacyAgentDir, storage);
		await expect(facadeB.archivedIds()).resolves.toContain(projectB.id);
		const remainderB = (await Bun.file(legacyPath).json()) as string[];
		expect(remainderB).toEqual(["ghost-unknown"]);
	});

	it("coordinates concurrent loads without losing IDs", async () => {
		const root = await fs.mkdtemp(path.join(os.tmpdir(), "harvest-archive-race-"));
		dirs.push(root);
		const storage = new FileSessionStorage();
		const legacyAgentDir = path.join(root, "agent");
		await fs.mkdir(legacyAgentDir, { recursive: true });
		const projectA = await makeProject(root, "proj-a", storage);
		const legacyPath = path.join(legacyAgentDir, "session-archive.json");
		await Bun.write(legacyPath, JSON.stringify([projectA.id]));

		const facade1 = makeFacade(projectA, legacyAgentDir, storage);
		const facade2 = makeFacade(projectA, legacyAgentDir, storage);
		const [first, second] = await Promise.all([facade1.archivedIds(), facade2.archivedIds()]);
		expect(first.has(projectA.id)).toBe(true);
		expect(second.has(projectA.id)).toBe(true);
		// Fully consumed: the legacy source is removed exactly once.
		expect(await Bun.file(legacyPath).exists()).toBe(false);
		const published = (await Bun.file(path.join(projectA.sessionDir, "archived-sessions.json")).json()) as {
			ids: string[];
		};
		expect(published.ids).toEqual([projectA.id]);
	});

	it("preserves the legacy source when publishing fails and retries on the next load", async () => {
		const root = await fs.mkdtemp(path.join(os.tmpdir(), "harvest-archive-fail-"));
		dirs.push(root);
		const legacyAgentDir = path.join(root, "agent");
		await fs.mkdir(legacyAgentDir, { recursive: true });
		const goodStorage = new FileSessionStorage();
		const projectA = await makeProject(root, "proj-a", goodStorage);
		const legacyPath = path.join(legacyAgentDir, "session-archive.json");
		const legacyBody = JSON.stringify([projectA.id]);
		await Bun.write(legacyPath, legacyBody);

		const failing = new FailingArchivePublishStorage();
		const failed = makeFacade(projectA, legacyAgentDir, failing);
		await expect(failed.archivedIds().then(ids => ids.size)).resolves.toBe(0);
		// Source preserved verbatim; nothing published.
		expect(await Bun.file(legacyPath).text()).toBe(legacyBody);
		expect(await Bun.file(path.join(projectA.sessionDir, "archived-sessions.json")).exists()).toBe(false);

		// Next load (fresh facade, healthy storage) retries and succeeds.
		const retried = makeFacade(projectA, legacyAgentDir, goodStorage);
		await expect(retried.archivedIds()).resolves.toContain(projectA.id);
		expect(await Bun.file(legacyPath).exists()).toBe(false);
	});
});
