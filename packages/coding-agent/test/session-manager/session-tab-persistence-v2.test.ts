import { describe, expect, it } from "bun:test";
import { SessionTabs } from "../../src/session/session-tabs";
import {
	loadSessionTabs,
	saveSessionTabs,
	SESSION_TABS_VERSION,
	snapshotSessionTabs,
} from "../../src/session/session-tab-persistence";

function memoryStorage(initial?: string) {
	let body = initial;
	return {
		writeTextSync: (_path: string, content: string) => {
			body = content;
		},
		readText: async (_path: string) => {
			if (body === undefined) throw Object.assign(new Error("missing"), { code: "ENOENT" });
			return body;
		},
	};
}

describe("session tab ID persistence", () => {
	it("persists every tab's stable ID, not just the active one", () => {
		const tabs = new SessionTabs();
		tabs.open("/sessions/one.jsonl", "First", "session-one");
		tabs.open("/sessions/two.jsonl", "Second", "session-two");
		tabs.close("/sessions/one.jsonl");

		const snapshot = snapshotSessionTabs(tabs, {
			projectKey: "C:/work",
			activePath: "/sessions/two.jsonl",
			activeSessionId: "session-two",
		});
		expect(snapshot.version).toBe(SESSION_TABS_VERSION);
		expect(snapshot.tabs).toEqual([{ path: "/sessions/two.jsonl", label: "Second", sessionId: "session-two" }]);
		expect(snapshot.activeSessionId).toBe("session-two");
		// Closed entries keep their IDs for reopen-by-ID.
		expect(snapshot.recentlyClosed).toEqual([
			{ path: "/sessions/one.jsonl", label: "First", sessionId: "session-one" },
		]);
	});

	it("round-trips background-tab IDs through save/load", async () => {
		const tabs = new SessionTabs();
		tabs.open("/sessions/one.jsonl", "First", "session-one");
		tabs.open("/sessions/two.jsonl", "Second", "session-two");

		const storage = memoryStorage();
		saveSessionTabs(
			storage,
			"tabs.json",
			snapshotSessionTabs(tabs, { projectKey: "C:/work", activePath: "/sessions/one.jsonl" }),
		);
		const loaded = await loadSessionTabs(storage, "tabs.json", "C:/work");
		expect(loaded?.tabs).toEqual([
			{ path: "/sessions/one.jsonl", label: "First", sessionId: "session-one" },
			{ path: "/sessions/two.jsonl", label: "Second", sessionId: "session-two" },
		]);
	});

	it("loads v1 payloads tolerantly, normalizing forward without fabricating IDs", async () => {
		// Failure mode: the version bump orphans every pre-migration tabs.json
		// and drops the user's open tabs on restart.
		const v1 = JSON.stringify({
			version: 1,
			projectKey: "C:/work",
			tabs: [{ path: "/sessions/one.jsonl", label: "First" }],
			activePath: "/sessions/one.jsonl",
			recentlyClosed: [{ path: "/sessions/old.jsonl" }],
		});
		const loaded = await loadSessionTabs(memoryStorage(v1), "tabs.json", "C:/work");
		expect(loaded?.version).toBe(SESSION_TABS_VERSION);
		expect(loaded?.tabs).toEqual([{ path: "/sessions/one.jsonl", label: "First" }]);
		expect(loaded?.recentlyClosed).toEqual([{ path: "/sessions/old.jsonl" }]);
	});
});
