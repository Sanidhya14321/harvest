import { afterEach, describe, expect, it, vi } from "bun:test";
import * as fs from "node:fs/promises";
import * as os from "node:os";
import * as path from "node:path";
import type { AgentSession } from "../src/session/agent-session";
import type { AgentSessionEvent } from "../src/session/agent-session-events";
import { LiveSessionRegistry } from "../src/session/live-session-registry";
import { SessionManagementFacade } from "../src/session/session-management-facade";
import { SessionManager } from "../src/session/session-manager";
import { FileSessionStorage } from "../src/session/session-storage";
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
		setSessionName: (title: string, source?: string) => manager.setSessionName(title, source as "auto" | "user"),
		dispose: async () => {
			await manager.close().catch(() => {});
		},
		abort: async () => {},
	} as unknown as AgentSession;
}

describe("facade archived reopen gate (T07)", () => {
	const dirs: string[] = [];
	afterEach(async () => {
		vi.restoreAllMocks();
		await Promise.all(dirs.splice(0).map(dir => fs.rm(dir, { recursive: true, force: true })));
	});

	async function makeHarness() {
		const root = await fs.mkdtemp(path.join(os.tmpdir(), "harvest-reopen-gate-"));
		dirs.push(root);
		const sessionDir = path.join(root, "sessions");
		await fs.mkdir(sessionDir, { recursive: true });
		const storage = new FileSessionStorage();
		const manager = SessionManager.create(root, sessionDir, storage);
		await manager.ensureOnDisk();
		const session = stubSession(manager);
		const registry = new LiveSessionRegistry(session, async () => {
			throw new Error("no cold opens");
		});
		const tabs = new SessionTabs();
		const file = manager.getSessionFile();
		if (!file) throw new Error("Session file was not created");
		const id = manager.getSessionId();
		tabs.ensureTabId(file, id, "Gated");
		const coldByPath = new Map<string, AgentSession>();
		const facade = new SessionManagementFacade({
			live: registry,
			tabs,
			viewState: new SessionViewStateStore(),
			storage,
			createSession: async () => {
				throw new Error("no creation");
			},
			openSession: async (sessionPath: string) => {
				const found = coldByPath.get(sessionPath);
				if (!found) throw new Error(`No cold session for ${sessionPath}`);
				return found;
			},
			directories: { cwd: root, sessionDir },
			legacyAgentDir: path.join(root, "agent"),
		});
		return { facade, registry, tabs, manager, session, id, file, coldByPath, storage, root, sessionDir };
	}

	it("warm reopen of an archived session is refused until Restore, then reopens truthfully", async () => {
		const { facade, registry, tabs, id, file } = await makeHarness();
		await facade.archive(id);
		// Warm runtime still live, but reopen is refused through the same gate.
		expect(registry.snapshots.some(snapshot => snapshot.id === id)).toBe(true);
		await expect(facade.reopen(id)).rejects.toThrow(/archived; restore it first/);
		await expect(facade.select(id)).resolves.toEqual({ switched: true });
		// Restore then reopen: truthful warm select of the requested session.
		await facade.restore(id);
		const reopened = await facade.reopen(id);
		expect(reopened.switched).toBe(true);
		expect(registry.snapshots.find(snapshot => snapshot.selected)?.id).toBe(id);
		expect(tabs.pathForId(id)).toBe(file);
	});

	it("cold reopen of an archived session is refused until Restore, then reopens truthfully", async () => {
		const { facade, registry, tabs, session, id, file, coldByPath, sessionDir } = await makeHarness();
		await facade.archive(id);
		// Detach the runtime (releasing its write lease, like the idle-release
		// path does) so the reopen takes the cold path through the persisted file.
		registry.detach(id);
		await session.dispose();
		const reopenedManager = await SessionManager.open(file, sessionDir, new FileSessionStorage());
		const coldSession = stubSession(reopenedManager);
		coldByPath.set(file, coldSession);

		await expect(facade.reopen(id)).rejects.toThrow(/archived; restore it first/);
		expect(facade.isDeleted(id)).toBe(false);
		await facade.restore(id);
		const reopened = await facade.reopen(id);
		expect(reopened.switched).toBe(true);
		expect(registry.snapshots.some(snapshot => snapshot.id === id)).toBe(true);
		expect(tabs.pathForId(id)).toBe(file);
	});

	it("reopenLast on an archived tab preserves the usable tab and names Restore", async () => {
		const { facade, tabs, id, file } = await makeHarness();
		// Close first (remembered), then archive the still-live runtime: the
		// remembered tab now points at an archived session.
		await facade.close(id);
		expect(tabs.paths).toHaveLength(0);
		await facade.archive(id);
		await expect(facade.reopenLast()).rejects.toThrow(/archived; restore it first/);
		// Failed selection preserves a usable tab: still open, still bound.
		expect(tabs.paths).toContain(file);
		expect(tabs.idForPath(file)).toBe(id);
		// Restore then reopen completes truthfully (the tab stayed open, so
		// reopen — not another reopenLast — is the follow-up).
		await facade.restore(id);
		const reopened = await facade.reopen(id);
		expect(reopened.switched).toBe(true);
	});

	it("unknown IDs reopen truthfully without disturbing open tabs", async () => {
		const { facade, tabs, id } = await makeHarness();
		const before = tabs.paths.slice();
		const result = await facade.reopen("00000000-0000-0000-0000-000000000000");
		expect(result.switched).toBe(false);
		expect(tabs.paths).toEqual(before);
		expect((await facade.listAll()).some(handle => handle.id === id)).toBe(true);
	});
});
