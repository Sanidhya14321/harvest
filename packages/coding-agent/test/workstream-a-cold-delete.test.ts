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

/** Storage double that fails deletion so failure semantics are observable. */
class FailingDeleteStorage extends FileSessionStorage {
	override async deleteSessionWithArtifacts(_sessionPath: string): Promise<void> {
		throw new Error("disk is read-only");
	}
}

describe("facade cold-UUID delete (A2)", () => {
	const dirs: string[] = [];
	afterEach(async () => {
		vi.restoreAllMocks();
		await Promise.all(dirs.splice(0).map(dir => fs.rm(dir, { recursive: true, force: true })));
	});

	async function makeHarness(storage = new FileSessionStorage()) {
		const root = await fs.mkdtemp(path.join(os.tmpdir(), "harvest-cold-delete-"));
		dirs.push(root);
		const sessionDir = path.join(root, "sessions");
		await fs.mkdir(sessionDir, { recursive: true });
		// Live session adopted into the registry.
		const liveManager = SessionManager.create(root, sessionDir, storage);
		await liveManager.ensureOnDisk();
		const liveFile = liveManager.getSessionFile();
		if (!liveFile) throw new Error("Live session file was not created");
		const liveId = liveManager.getSessionId();
		// Cold session: persisted on disk through real storage, never adopted.
		const coldManager = SessionManager.create(root, sessionDir, storage);
		await coldManager.ensureOnDisk();
		const coldFile = coldManager.getSessionFile();
		if (!coldFile) throw new Error("Cold session file was not created");
		const coldId = coldManager.getSessionId();
		const registry = new LiveSessionRegistry(stubSession(liveManager), async () => {
			throw new Error("no cold opens");
		});
		const tabs = new SessionTabs();
		tabs.ensureTabId(liveFile, liveId, "live");
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
		return { facade, registry, tabs, liveId, liveFile, coldId, coldFile };
	}

	it("deletes a cold session by UUID discovered through the picker listing", async () => {
		const { facade, registry, liveId, liveFile, coldId, coldFile } = await makeHarness();
		// Picker flow: the cold UUID is discoverable before deletion.
		const before = await facade.listAll();
		expect(before.some(handle => handle.id === coldId)).toBe(true);
		await facade.delete(coldId);
		expect(await Bun.file(coldFile).exists()).toBe(false);
		expect(facade.isDeleted(coldId)).toBe(true);
		// The live session is untouched and still listed; the cold one is gone.
		expect(registry.snapshots.some(snapshot => snapshot.id === liveId)).toBe(true);
		expect(await Bun.file(liveFile).exists()).toBe(true);
		const after = await facade.listAll();
		expect(after.some(handle => handle.id === coldId)).toBe(false);
		expect(after.some(handle => handle.id === liveId)).toBe(true);
	});

	it("rejects unknown IDs with nothing tombstoned", async () => {
		const { facade, liveId } = await makeHarness();
		await expect(facade.delete("00000000-0000-0000-0000-000000000000")).rejects.toThrow(/not found/);
		expect(facade.isDeleted("00000000-0000-0000-0000-000000000000")).toBe(false);
		// A typo never hides the live session from discovery.
		const handles = await facade.listAll();
		expect(handles.some(handle => handle.id === liveId)).toBe(true);
	});

	it("retains discoverability when storage deletion fails", async () => {
		const storage = new FailingDeleteStorage();
		const { facade, coldId, coldFile } = await makeHarness(storage);
		await expect(facade.delete(coldId)).rejects.toThrow(/remains discoverable/);
		expect(await Bun.file(coldFile).exists()).toBe(true);
		expect(facade.isDeleted(coldId)).toBe(false);
		const handles = await facade.listAll();
		expect(handles.some(handle => handle.id === coldId)).toBe(true);
	});

	it("rejects busy live targets without stopping them", async () => {
		const { facade, registry, liveId } = await makeHarness();
		const live = registry.sessions.find(s => s.sessionManager.getSessionId() === liveId);
		(live as unknown as { setStreaming: (value: boolean) => void }).setStreaming(true);
		await expect(facade.delete(liveId)).rejects.toThrow(/busy/);
		expect(facade.isDeleted(liveId)).toBe(false);
		expect(registry.snapshots.some(snapshot => snapshot.id === liveId)).toBe(true);
	});
});
