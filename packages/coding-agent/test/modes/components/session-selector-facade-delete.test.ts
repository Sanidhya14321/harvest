import { afterEach, beforeAll, describe, expect, it, vi } from "bun:test";
import { SessionSelectorComponent } from "@harvest/pi-coding-agent/modes/components/session-selector";
import { initTheme } from "@harvest/pi-coding-agent/modes/theme/theme";
import type { SessionInfo } from "@harvest/pi-coding-agent/session/session-listing";
import { SessionTabs } from "../../../src/session/session-tabs";

beforeAll(() => {
	initTheme();
});

afterEach(() => {
	vi.restoreAllMocks();
});

function session(id: string, title: string): SessionInfo {
	return {
		path: `/tmp/${id}.jsonl`,
		id,
		cwd: "/tmp",
		title,
		created: new Date("2024-01-01T00:00:00Z"),
		modified: new Date("2024-01-02T00:00:00Z"),
		messageCount: 1,
		size: 0,
		firstMessage: `${title} first message`,
		allMessagesText: `${title} first message`,
	};
}

/**
 * Facade double shaped exactly like `SessionManagementFacade` for the delete
 * path: `delete(sessionId, opts?)` settles/seals/deletes/tombstones (throws
 * on busy without `stopFirst`), plus `isDeleted(sessionId)`. Backed by an
 * in-memory live set + busy set so the picker flows exercise the real
 * production contract without importing production `InteractiveMode`.
 */
function makeFacadeDouble(initialIds: string[]) {
	const live = new Set(initialIds);
	const busy = new Set<string>();
	const tombstones = new Set<string>();
	const deleted: string[] = [];
	const stops: string[] = [];
	return {
		async delete(sessionId: string, opts?: { stopFirst?: boolean }): Promise<void> {
			if (!live.has(sessionId)) throw new Error(`Session ${sessionId} is not live`);
			const isBusy = busy.has(sessionId);
			if (isBusy && !opts?.stopFirst) {
				throw new Error(`Session ${sessionId} is busy; stop it first or delete with stop-and-delete`);
			}
			if (isBusy && opts?.stopFirst) {
				stops.push(sessionId);
				busy.delete(sessionId);
			}
			live.delete(sessionId);
			deleted.push(sessionId);
			tombstones.add(sessionId);
		},
		isDeleted(sessionId: string): boolean {
			return tombstones.has(sessionId);
		},
		markBusy(id: string): void {
			busy.add(id);
		},
		markIdle(id: string): void {
			busy.delete(id);
		},
		isLive(id: string): boolean {
			return live.has(id);
		},
		deleted,
		stops,
	};
}

/**
 * Owner-shaped adapter (mirrors selector-controller): resolve path→UUID via
 * live snapshot, then tabs.idForPath, then SessionInfo.id; recheck busy at
 * execution time; route through facade.delete; return true on success.
 */
function makeOwnerAdapter(
	facade: ReturnType<typeof makeFacadeDouble>,
	tabs: SessionTabs,
	liveByPath: Map<string, string>,
	isBusy: (info: SessionInfo) => boolean,
) {
	const resolveId = (target: SessionInfo): string => {
		const snapshotId = liveByPath.get(target.path);
		if (snapshotId) return snapshotId;
		return tabs.idForPath(target.path) ?? target.id;
	};
	return {
		isSessionBusy: isBusy,
		onDelete: async (target: SessionInfo): Promise<boolean> => {
			if (isBusy(target)) {
				throw new Error("Session became busy; stop the run first (Esc), then delete the session.");
			}
			await facade.delete(resolveId(target));
			return true;
		},
		onStopAndDelete: async (target: SessionInfo): Promise<boolean> => {
			await facade.delete(resolveId(target), { stopFirst: true });
			return true;
		},
	};
}

function makePicker(
	sessions: SessionInfo[],
	adapter: {
		isSessionBusy: (s: SessionInfo) => boolean;
		onDelete: (s: SessionInfo) => Promise<boolean>;
		onStopAndDelete: (s: SessionInfo) => Promise<boolean>;
	},
): SessionSelectorComponent {
	return new SessionSelectorComponent(
		sessions,
		() => {},
		() => {},
		() => {},
		{
			onDelete: adapter.onDelete,
			isSessionBusy: adapter.isSessionBusy,
			onStopAndDelete: adapter.onStopAndDelete,
		},
	);
}

function text(selector: SessionSelectorComponent): string {
	return Bun.stripANSI(selector.render(120).join("\n"));
}

describe("selector facade-backed delete flows", () => {
	it("deletes an idle session through the facade double", async () => {
		const tabs = new SessionTabs();
		tabs.open("/tmp/session-a.jsonl", "Alpha");
		tabs.noteId("/tmp/session-a.jsonl", "session-a");
		tabs.open("/tmp/session-b.jsonl", "Beta");
		tabs.noteId("/tmp/session-b.jsonl", "session-b");
		const facade = makeFacadeDouble(["session-a", "session-b"]);
		const liveByPath = new Map([
			["/tmp/session-a.jsonl", "session-a"],
			["/tmp/session-b.jsonl", "session-b"],
		]);
		const adapter = makeOwnerAdapter(facade, tabs, liveByPath, () => false);
		const selector = makePicker([session("session-a", "Alpha"), session("session-b", "Beta")], adapter);

		selector.handleInput("\x1b[3~");
		expect(text(selector)).toContain("Delete session?");
		selector.handleInput("\n");
		await Bun.sleep(0);

		expect(facade.deleted).toEqual(["session-a"]);
		expect(facade.isDeleted("session-a")).toBe(true);
		expect(text(selector)).not.toContain("Alpha");
		expect(text(selector)).toContain("Beta");
	});

	it("busy targets detour to stop-and-delete and settle through the facade", async () => {
		const tabs = new SessionTabs();
		tabs.open("/tmp/session-a.jsonl", "Alpha");
		tabs.noteId("/tmp/session-a.jsonl", "session-a");
		const facade = makeFacadeDouble(["session-a", "session-b"]);
		facade.markBusy("session-a");
		const liveByPath = new Map([["/tmp/session-a.jsonl", "session-a"]]);
		const adapter = makeOwnerAdapter(facade, tabs, liveByPath, s => s.id === "session-a");
		const selector = makePicker([session("session-a", "Alpha"), session("session-b", "Beta")], adapter);

		selector.handleInput("\x1b[3~");
		expect(text(selector)).toContain("Session is busy");
		expect(text(selector)).toContain("Stop & Delete");

		selector.handleInput("\n");
		await Bun.sleep(0);

		expect(facade.stops).toEqual(["session-a"]);
		expect(facade.deleted).toEqual(["session-a"]);
		expect(text(selector)).not.toContain("Alpha");
	});

	it("execution-time busy recheck refuses a session that started after confirmation", async () => {
		// Failure mode: the target idles at request time, starts a run while
		// the confirm dialog is open, then deletes mid-flight.
		const tabs = new SessionTabs();
		tabs.open("/tmp/session-a.jsonl", "Alpha");
		let busyNow = false;
		const facade = makeFacadeDouble(["session-a"]);
		const liveByPath = new Map([["/tmp/session-a.jsonl", "session-a"]]);
		const adapter = makeOwnerAdapter(facade, tabs, liveByPath, () => busyNow);
		const selector = makePicker([session("session-a", "Alpha")], adapter);

		selector.handleInput("\x1b[3~");
		expect(text(selector)).toContain("Delete session?");

		// Run starts after confirmation but before dispatch.
		busyNow = true;
		facade.markBusy("session-a");
		selector.handleInput("\n");
		await Bun.sleep(0);

		// Component-level recheck detours to stop-and-delete instead of
		// calling plain delete; nothing deleted yet.
		expect(facade.deleted).toEqual([]);
		expect(text(selector)).toContain("Session is busy");
	});

	it("resolves path→UUID via tabs when the live snapshot misses (owner contract)", async () => {
		const tabs = new SessionTabs();
		tabs.open("/tmp/session-a.jsonl", "Alpha");
		tabs.noteId("/tmp/session-a.jsonl", "uuid-noted-in-tabs");
		const facade = makeFacadeDouble(["uuid-noted-in-tabs"]);
		// No live snapshot for this path: resolution falls through to tabs.
		const liveByPath = new Map<string, string>();
		const adapter = makeOwnerAdapter(facade, tabs, liveByPath, () => false);
		// SessionInfo.id is stale; tabs carries the stable UUID.
		const stale: SessionInfo = { ...session("stale-id", "Alpha"), path: "/tmp/session-a.jsonl" };
		const selector = makePicker([stale], adapter);

		selector.handleInput("\x1b[3~");
		selector.handleInput("\n");
		await Bun.sleep(0);

		expect(facade.deleted).toEqual(["uuid-noted-in-tabs"]);
	});
});
