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
	const dispose = vi.fn(async () => {
		await manager.close().catch(() => {});
	});
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
		dispose,
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

describe("facade three-session close order (T02)", () => {
	const dirs: string[] = [];
	afterEach(async () => {
		vi.restoreAllMocks();
		await Promise.all(dirs.splice(0).map(dir => fs.rm(dir, { recursive: true, force: true })));
	});

	async function makeHarness() {
		const root = await fs.mkdtemp(path.join(os.tmpdir(), "harvest-close-order-"));
		dirs.push(root);
		const sessionDir = path.join(root, "sessions");
		await fs.mkdir(sessionDir, { recursive: true });
		const storage = new FileSessionStorage();
		const first = SessionManager.create(root, sessionDir, storage);
		await first.ensureOnDisk();
		const registry = new LiveSessionRegistry(stubSession(first), async () => {
			throw new Error("no cold opens");
		});
		const tabs = new SessionTabs();
		const viewState = new SessionViewStateStore();
		const pending: AgentSession[] = [];
		const facade = new SessionManagementFacade({
			live: registry,
			tabs,
			viewState,
			storage,
			createSession: async () => {
				const manager = SessionManager.create(root, sessionDir, storage);
				await manager.ensureOnDisk();
				const session = stubSession(manager);
				pending.push(session);
				registry.adopt(session);
				return session;
			},
			openSession: async () => {
				throw new Error("no cold opens");
			},
			directories: { cwd: root, sessionDir },
			legacyAgentDir: path.join(root, "agent"),
		});
		const firstId = first.getSessionId();
		const firstFile = first.getSessionFile();
		if (!firstFile) throw new Error("First session file was not created");
		tabs.ensureTabId(firstFile, firstId, "First");
		const second = await facade.create();
		const third = await facade.create();
		return { facade, registry, tabs, viewState, firstId, firstFile, second, third };
	}

	it("closes the middle tab onto the right neighbor; runtimes continue and drafts restore", async () => {
		const { facade, registry, tabs, viewState, firstId, second, third } = await makeHarness();
		const order = tabs.paths.slice();
		expect(order).toHaveLength(3);

		// Save a draft under the middle session, then close-then-select right.
		const editor = fakeEditor("middle draft");
		viewState.saveDraft(second.id, editor);
		const next = tabs.closeNeighbor(tabs.pathForId(second.id) ?? "");
		expect(next).toBe(tabs.pathForId(third.id));
		const nextId = tabs.idForPath(next ?? "");
		const result = await facade.closeAndSelect(second.id, nextId);
		expect(result.switched).toBe(true);
		expect(result.lastTab).toBe(false);

		// Closed tab hidden, both runtimes still live and undisposed.
		expect(tabs.paths).toHaveLength(2);
		expect(tabs.pathForId(second.id)).toBeUndefined();
		expect(registry.snapshots.some(snapshot => snapshot.id === second.id)).toBe(true);
		expect(registry.snapshots.some(snapshot => snapshot.id === third.id)).toBe(true);
		expect(registry.snapshots.some(snapshot => snapshot.id === firstId)).toBe(true);

		// The middle session's draft restores exactly; selection followed right.
		const restore = fakeEditor();
		viewState.restoreDraft(second.id, restore);
		expect(restore.current()).toBe("middle draft");
		expect(registry.snapshots.find(snapshot => snapshot.selected)?.id).toBe(third.id);
	});

	it("closing an inactive tab preserves the selection; final close reports lastTab with runtimes intact", async () => {
		const { facade, registry, tabs, firstId, second, third } = await makeHarness();
		// Selection sits on the third tab; close the inactive first tab.
		expect(registry.snapshots.find(snapshot => snapshot.selected)?.id).toBe(third.id);
		await facade.close(firstId);
		expect(registry.snapshots.find(snapshot => snapshot.selected)?.id).toBe(third.id);
		expect(registry.snapshots.some(snapshot => snapshot.id === firstId)).toBe(true);

		// Close the middle (second) tab via close-then-select onto third.
		const afterInactive = await facade.closeAndSelect(second.id, third.id);
		expect(afterInactive).toEqual({ switched: true, lastTab: false });
		// Final close: the view empties but the runtime keeps running (detached Home is the owner's call).
		const final = await facade.close(third.id);
		expect(final.lastTab).toBe(true);
		expect(tabs.paths).toHaveLength(0);
		expect(registry.snapshots.some(snapshot => snapshot.id === third.id)).toBe(true);
	});

	it("keyboard next/prev cycles with wrap while close-then-select never wraps", async () => {
		const { tabs, firstId, second, third } = await makeHarness();
		const first = tabs.pathForId(firstId) ?? "";
		const middle = tabs.pathForId(second.id) ?? "";
		const last = tabs.pathForId(third.id) ?? "";
		// Keyboard cycling wraps at both ends.
		expect(tabs.neighbor(last, 1)).toBe(first);
		expect(tabs.neighbor(first, -1)).toBe(last);
		// Close targets never wrap: rightmost falls back left.
		expect(tabs.closeNeighbor(last)).toBe(middle);
		expect(tabs.closeNeighbor(middle)).toBe(last);
	});

	it("concurrent close-then-select and select settle without deadlock", async () => {
		const { facade, tabs, second, third } = await makeHarness();
		const middle = tabs.pathForId(second.id) ?? "";
		const last = tabs.pathForId(third.id) ?? "";
		const [closed, selected] = await Promise.all([
			facade.closeAndSelect(second.id, tabs.idForPath(last)),
			facade.select(third.id),
		]);
		expect(closed.switched || selected.switched).toBe(true);
		expect(tabs.paths).not.toContain(middle);
	});
});
