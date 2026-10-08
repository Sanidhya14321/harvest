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
import {
	backfillMissingTabIds,
	loadSessionTabs,
	saveSessionTabs,
	sessionTabsFile,
	snapshotSessionTabs,
} from "../src/session/session-tab-persistence";
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
		dispose: async () => {},
		abort: async () => {},
	} as unknown as AgentSession;
}

describe("session tab UUID persistence (A4)", () => {
	const dirs: string[] = [];
	afterEach(async () => {
		vi.restoreAllMocks();
		await Promise.all(dirs.splice(0).map(dir => fs.rm(dir, { recursive: true, force: true })));
	});

	async function makeDirs() {
		const root = await fs.mkdtemp(path.join(os.tmpdir(), "harvest-tab-uuid-"));
		dirs.push(root);
		const sessionDir = path.join(root, "sessions");
		await fs.mkdir(sessionDir, { recursive: true });
		return { root, sessionDir, storage: new FileSessionStorage() };
	}

	it("ensureTabId binds open tabs without disturbing order", () => {
		const tabs = new SessionTabs();
		tabs.open("/s/one.jsonl", "One");
		tabs.open("/s/two.jsonl", "Two");
		tabs.ensureTabId("/s/two.jsonl", "uuid-two", "Two renamed");
		tabs.ensureTabId("/s/three.jsonl", "uuid-three");
		expect(tabs.paths).toEqual(["/s/one.jsonl", "/s/two.jsonl", "/s/three.jsonl"]);
		expect(tabs.idForPath("/s/two.jsonl")).toBe("uuid-two");
		expect(tabs.idForPath("/s/three.jsonl")).toBe("uuid-three");
		expect(tabs.pathForId("uuid-two")).toBe("/s/two.jsonl");
		expect(tabs.entries).toContainEqual({ path: "/s/two.jsonl", label: "Two renamed", sessionId: "uuid-two" });
	});

	it("persists UUIDs across restarts and recovers closed tabs", async () => {
		const { sessionDir, storage } = await makeDirs();
		const projectKey = "proj";
		const tabs = new SessionTabs();
		tabs.ensureTabId("/s/one.jsonl", "uuid-one", "One");
		tabs.ensureTabId("/s/two.jsonl", "uuid-two");
		tabs.visit("/s/one.jsonl");
		tabs.visit("/s/two.jsonl");
		tabs.close("/s/one.jsonl", true);
		const file = sessionTabsFile(sessionDir);
		saveSessionTabs(
			storage,
			file,
			snapshotSessionTabs(tabs, { projectKey, activePath: "/s/two.jsonl", activeSessionId: "uuid-two" }),
		);

		// Restart: a fresh tab set restores paths, IDs, the active ID, and closed history.
		const restored = new SessionTabs();
		const snapshot = await loadSessionTabs(storage, file, projectKey);
		expect(snapshot).toBeDefined();
		for (const entry of snapshot!.tabs) {
			if (entry.sessionId) restored.ensureTabId(entry.path, entry.sessionId, entry.label);
			else restored.open(entry.path, entry.label);
		}
		for (const entry of snapshot!.recentlyClosed) restored.rememberClosed(entry.path, entry.label, entry.sessionId);
		expect(restored.idForPath("/s/two.jsonl")).toBe("uuid-two");
		expect(snapshot!.activeSessionId).toBe("uuid-two");
		expect(restored.recentlyClosed).toContainEqual({ path: "/s/one.jsonl", label: "One", sessionId: "uuid-one" });
	});

	it("rejects foreign-project and corrupt payloads", async () => {
		const { sessionDir, storage } = await makeDirs();
		const tabs = new SessionTabs();
		tabs.ensureTabId("/s/one.jsonl", "uuid-one");
		const file = sessionTabsFile(sessionDir);
		saveSessionTabs(storage, file, snapshotSessionTabs(tabs, { projectKey: "proj" }));
		expect(await loadSessionTabs(storage, file, "other-proj")).toBeUndefined();
		await Bun.write(file, "{not json");
		expect(await loadSessionTabs(storage, file, "proj")).toBeUndefined();
	});

	it("backfills legacy path-only payloads without fabricating IDs", () => {
		const tabs = new SessionTabs();
		tabs.ensureTabId("/s/known.jsonl", "uuid-known");
		const legacy = {
			version: 2 as const,
			projectKey: "proj",
			tabs: [{ path: "/s/known.jsonl" }, { path: "/s/unknown.jsonl" }],
			activePath: "/s/known.jsonl",
			activeSessionId: undefined,
			recentlyClosed: [{ path: "/s/unknown.jsonl" }],
		};
		const filled = backfillMissingTabIds(legacy, p => tabs.idForPath(p));
		expect(filled.tabs).toContainEqual({ path: "/s/known.jsonl", sessionId: "uuid-known" });
		expect(filled.tabs).toContainEqual({ path: "/s/unknown.jsonl" });
		expect(filled.activeSessionId).toBe("uuid-known");
		expect(filled.recentlyClosed).toEqual([{ path: "/s/unknown.jsonl" }]);
	});

	it("facade transitions keep the path→UUID binding: create, select, close, reopen", async () => {
		const { root, sessionDir, storage } = await makeDirs();
		const managerA = SessionManager.create(root, sessionDir, storage);
		await managerA.ensureOnDisk();
		const fileA = managerA.getSessionFile();
		if (!fileA) throw new Error("Session file was not created");
		const registry = new LiveSessionRegistry(stubSession(managerA), async () => {
			throw new Error("no cold opens");
		});
		const tabs = new SessionTabs();
		const coldByPath = new Map<string, AgentSession>();
		// Create a second live session through the facade: tab carries its UUID.
		const managerB = SessionManager.create(root, sessionDir, storage);
		await managerB.ensureOnDisk();
		const sessionB = stubSession(managerB);
		const idB = managerB.getSessionId();
		const fileB = managerB.getSessionFile();
		if (!fileB) throw new Error("Second session file was not created");
		const facade = new SessionManagementFacade({
			live: registry,
			tabs,
			viewState: new SessionViewStateStore(),
			storage,
			createSession: async () => {
				registry.adopt(sessionB);
				return sessionB;
			},
			openSession: async (sessionPath: string) => {
				const found = coldByPath.get(sessionPath);
				if (!found) throw new Error(`No cold session for ${sessionPath}`);
				return found;
			},
			directories: { cwd: root, sessionDir },
			legacyAgentDir: path.join(root, "agent"),
		});
		const created = await facade.create();
		expect(created.id).toBe(idB);
		expect(tabs.idForPath(fileB)).toBe(idB);

		// Warm select re-binds the ID, close remembers it, reopenLast restores it.
		expect((await facade.select(idB)).switched).toBe(true);
		expect(tabs.idForPath(fileB)).toBe(idB);
		coldByPath.set(fileB, sessionB);
		expect((await facade.close(idB)).lastTab).toBe(true);
		// Detach the runtime so reopenLast takes the cold path through the
		// persisted file and the openSession seam, then re-binds the UUID.
		registry.detach(idB);
		const reopened = await facade.reopenLast();
		expect(reopened.sessionId).toBe(idB);
		expect(reopened.switched).toBe(true);
		expect(tabs.idForPath(fileB)).toBe(idB);
	});
});
