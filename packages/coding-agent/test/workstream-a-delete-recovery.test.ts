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
import { SessionViewStateStore, type DraftEditor } from "../src/session/session-view-state";

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

function fakeEditor(text = ""): DraftEditor & { current: () => string } {
	let value = text;
	return {
		getText: () => value,
		setText: next => {
			value = next;
		},
		pendingImages: [],
		pendingImageLinks: [],
		current: () => value,
	};
}

/** Storage double failing deletion a bounded number of times, then healthy. */
class FailOnceDeleteStorage extends FileSessionStorage {
	failuresLeft: number;
	constructor(failures: number) {
		super();
		this.failuresLeft = failures;
	}
	override async deleteSessionWithArtifacts(sessionPath: string): Promise<void> {
		if (this.failuresLeft > 0) {
			this.failuresLeft--;
			throw new Error("injected unlink failure");
		}
		return super.deleteSessionWithArtifacts(sessionPath);
	}
}

/** Artifact-boundary partial: the session file is gone, artifact cleanup fails. */
class ArtifactBoundaryStorage extends FileSessionStorage {
	override async deleteSessionWithArtifacts(sessionPath: string): Promise<void> {
		await this.unlink(sessionPath);
		throw new Error(`Session file deleted but failed to remove artifacts directory ${sessionPath.slice(0, -6)}`);
	}
}

async function sessionFileId(file: string): Promise<string | undefined> {
	const text = await Bun.file(file).text();
	for (const line of text.split("\n")) {
		const trimmed = line.trim();
		if (!trimmed.startsWith("{")) continue;
		try {
			const entry = JSON.parse(trimmed) as { type?: string; id?: string };
			if (entry.type === "session" && typeof entry.id === "string") return entry.id;
		} catch {
			// Title slot line: skip.
		}
	}
	return undefined;
}

describe("facade delete failure recovery (T05)", () => {
	const dirs: string[] = [];
	afterEach(async () => {
		vi.restoreAllMocks();
		await Promise.all(dirs.splice(0).map(dir => fs.rm(dir, { recursive: true, force: true })));
	});

	async function makeHarness(storage: FileSessionStorage) {
		const root = await fs.mkdtemp(path.join(os.tmpdir(), "harvest-delete-recovery-"));
		dirs.push(root);
		const sessionDir = path.join(root, "sessions");
		await fs.mkdir(sessionDir, { recursive: true });
		const manager = SessionManager.create(root, sessionDir, storage);
		await manager.ensureOnDisk();
		await manager.setSessionName("Original", "user");
		const file = manager.getSessionFile();
		if (!file) throw new Error("Session file was not created");
		const id = manager.getSessionId();
		const session = stubSession(manager);
		const registry = new LiveSessionRegistry(session, async () => {
			throw new Error("no cold opens");
		});
		const tabs = new SessionTabs();
		const viewState = new SessionViewStateStore();
		tabs.ensureTabId(file, id, "Original");
		const facade = new SessionManagementFacade({
			live: registry,
			tabs,
			viewState,
			storage,
			createSession: async () => {
				throw new Error("no creation");
			},
			openSession: async (sessionPath: string) => {
				const reopened = await SessionManager.open(sessionPath, sessionDir, storage);
				return stubSession(reopened);
			},
			directories: { cwd: root, sessionDir },
			legacyAgentDir: path.join(root, "agent"),
		});
		return { facade, registry, tabs, viewState, manager, session, id, file, storage, sessionDir };
	}

	it("injected unlink failure recovers a usable same-UUID runtime; retry succeeds; old generation fenced", async () => {
		const harness = await makeHarness(new FailOnceDeleteStorage(1));
		const { facade, registry, tabs, viewState, manager: oldManager, session: oldSession, id, file } = harness;
		viewState.saveDraft(id, fakeEditor("unsent work"));
		viewState.saveReadingAnchor(id, { block: 3, row: 7 });

		await expect(facade.delete(id)).rejects.toThrow(/Failed to delete session.*Recovered a usable runtime/);
		// Nothing tombstoned, nothing hidden: discoverable under the same UUID.
		expect(facade.isDeleted(id)).toBe(false);
		expect(tabs.pathForId(id)).toBe(file);
		expect(await sessionFileId(file)).toBe(id);
		const live = registry.sessions.find(s => s.sessionManager.getSessionId() === id);
		expect(live).toBeDefined();
		expect(live).not.toBe(oldSession);

		// View state preserved: draft and anchor survive the failed delete.
		expect(viewState.hasDraft(id)).toBe(true);
		const restore = fakeEditor();
		viewState.restoreDraft(id, restore);
		expect(restore.current()).toBe("unsent work");
		expect(viewState.readingAnchor(id)).toEqual({ block: 3, row: 7 });

		// Old sealed generation is fenced: renames are dropped, on-disk title untouched.
		await expect(oldManager.setSessionName("Hacked", "user")).resolves.toBe(false);
		expect(oldManager.getSessionName()).toBe("Original");

		// Recovered runtime is durable: rename + flush persist, then retry deletes cleanly.
		const recovered = registry.sessions.find(s => s.sessionManager.getSessionId() === id);
		if (!recovered) throw new Error("Recovered runtime is missing");
		await recovered.setSessionName("Recovered", "user");
		await recovered.sessionManager.flush();
		expect(recovered.sessionManager.getSessionName()).toBe("Recovered");
		await facade.delete(id);
		expect(facade.isDeleted(id)).toBe(true);
		expect(await Bun.file(file).exists()).toBe(false);
		expect(registry.snapshots.some(snapshot => snapshot.id === id)).toBe(false);
		expect(viewState.hasDraft(id)).toBe(false);
		expect(viewState.readingAnchor(id)).toBeUndefined();
	});

	it("deleting a never-materialized live session succeeds idempotently without sealing it into oblivion", async () => {
		const root = await fs.mkdtemp(path.join(os.tmpdir(), "harvest-delete-unmaterialized-"));
		dirs.push(root);
		const sessionDir = path.join(root, "sessions");
		await fs.mkdir(sessionDir, { recursive: true });
		const storage = new FileSessionStorage();
		// No ensureOnDisk: lazy persistence never wrote the file.
		const manager = SessionManager.create(root, sessionDir, storage);
		const file = manager.getSessionFile();
		const id = manager.getSessionId();
		const session = stubSession(manager);
		const registry = new LiveSessionRegistry(session, async () => {
			throw new Error("no cold opens");
		});
		const tabs = new SessionTabs();
		if (file) tabs.ensureTabId(file, id, "Unmaterialized");
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
		if (file) expect(await Bun.file(file).exists()).toBe(false);
		await facade.delete(id);
		expect(facade.isDeleted(id)).toBe(true);
		expect(registry.snapshots.some(snapshot => snapshot.id === id)).toBe(false);
	});

	it("artifact-boundary partial (file gone, artifacts failed) reconstructs the same UUID and stays retryable", async () => {
		const harness = await makeHarness(new ArtifactBoundaryStorage());
		const { facade, registry, id, file } = harness;
		await expect(facade.delete(id)).rejects.toThrow(/Failed to delete session/);
		// Reconstructed under the same identity with the preserved transcript.
		expect(await Bun.file(file).exists()).toBe(true);
		expect(await sessionFileId(file)).toBe(id);
		expect(facade.isDeleted(id)).toBe(false);
		expect(registry.sessions.some(s => s.sessionManager.getSessionId() === id)).toBe(true);
		const handles = await facade.listAll();
		expect(handles.some(handle => handle.id === id)).toBe(true);
	});
});
