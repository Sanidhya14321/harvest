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

function stubSession(manager: SessionManager, opts?: { streaming?: boolean }): AgentSession {
	let streaming = opts?.streaming ?? false;
	const abort = vi.fn(async () => {});
	const dispose = vi.fn(async () => {});
	const listeners = new Set<(event: AgentSessionEvent) => void>();
	return {
		get isStreaming() {
			return streaming;
		},
		setStreaming: (value: boolean) => {
			streaming = value;
		},
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
		dispose,
		abort,
	} as unknown as AgentSession;
}

describe("session delete seals the writer (S1)", () => {
	const dirs: string[] = [];
	afterEach(async () => {
		vi.restoreAllMocks();
		await Promise.all(dirs.splice(0).map(dir => fs.rm(dir, { recursive: true, force: true })));
	});

	async function makeHarness() {
		const root = await fs.mkdtemp(path.join(os.tmpdir(), "harvest-delete-seal-"));
		dirs.push(root);
		const sessionDir = path.join(root, "sessions");
		await fs.mkdir(sessionDir, { recursive: true });
		const storage = new FileSessionStorage();
		const manager = SessionManager.create(root, sessionDir, storage);
		await manager.ensureOnDisk();
		const file = manager.getSessionFile();
		if (!file) throw new Error("Session file was not created");
		const session = stubSession(manager);
		const registry = new LiveSessionRegistry(session, async () => {
			throw new Error("no cold opens");
		});
		const tabs = new SessionTabs();
		tabs.open(file, "Seal target");
		const id = manager.getSessionId();
		tabs.noteId(file, id);
		const facade = new SessionManagementFacade({
			live: registry,
			tabs,
			viewState: new SessionViewStateStore(),
			storage,
			createSession: async () => {
				throw new Error("no creation");
			},
			openSession: async () => {
				throw new Error("no cold opens");
			},
			directories: { cwd: root, sessionDir },
			legacyAgentDir: path.join(root, "agent"),
		});
		return { facade, manager, session, registry, tabs, file, id };
	}

	it("deletes an idle background session and late writes cannot recreate it", async () => {
		const { facade, manager, file, id } = await makeHarness();
		await facade.delete(id);
		expect(await Bun.file(file).exists()).toBe(false);
		expect(facade.isDeleted(id)).toBe(true);
		// Late rename, callback append, and persistence attempts are fenced.
		await manager.setSessionName("Late rename", "user");
		manager.appendCustomEntry("late-note", { text: "late callback" });
		await manager.ensureOnDisk();
		expect(await Bun.file(file).exists()).toBe(false);
	});

	it("stop-and-delete settlement failure rejects and keeps the session discoverable", async () => {
		const { facade, manager, file, id, registry } = await makeHarness();
		const live = registry.sessions.find(s => s.sessionManager.getSessionId() === id);
		if (!live) throw new Error("Target is not live");
		(live as unknown as { setStreaming: (value: boolean) => void }).setStreaming(true);
		await expect(facade.delete(id, { stopFirst: true })).rejects.toThrow(/did not settle/);
		expect(await Bun.file(file).exists()).toBe(true);
		expect(registry.snapshots.some(snapshot => snapshot.id === id)).toBe(true);
		expect(facade.isDeleted(id)).toBe(false);
		expect(manager).toBeDefined();
	});
});
