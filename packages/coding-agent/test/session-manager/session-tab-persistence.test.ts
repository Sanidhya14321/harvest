import { describe, expect, it } from "bun:test";
import { SessionTabs } from "../../src/session/session-tabs";
import {
	loadSessionTabs,
	saveSessionTabs,
	snapshotSessionTabs,
	MAX_PERSISTED_CLOSED_TABS,
	MAX_RESTORED_TABS,
} from "../../src/session/session-tab-persistence";

function memoryStorage(initial?: string) {
	let body = initial;
	return {
		written: () => body,
		writeTextSync: (_path: string, content: string) => {
			body = content;
		},
		readText: async (_path: string) => {
			if (body === undefined) throw Object.assign(new Error("missing"), { code: "ENOENT" });
			return body;
		},
	};
}

function tabbed(): SessionTabs {
	const tabs = new SessionTabs();
	tabs.open("/sessions/one.jsonl", "First");
	tabs.open("/sessions/two.jsonl", "Second");
	tabs.visit("/sessions/two.jsonl");
	tabs.close("/sessions/one.jsonl");
	return tabs;
}

describe("session tab persistence", () => {
	it("round-trips open tabs, labels, the active reference, and recently closed", async () => {
		const storage = memoryStorage();
		const tabs = tabbed();
		saveSessionTabs(
			storage,
			"tabs.json",
			snapshotSessionTabs(tabs, {
				projectKey: "C:/work",
				activePath: "/sessions/two.jsonl",
				activeSessionId: "session-two",
			}),
		);
		const loaded = await loadSessionTabs(storage, "tabs.json", "C:/work");
		expect(loaded?.tabs).toEqual([{ path: "/sessions/two.jsonl", label: "Second", sessionId: "session-two" }]);
		expect(loaded?.activePath).toBe("/sessions/two.jsonl");
		expect(loaded?.recentlyClosed).toEqual([{ path: "/sessions/one.jsonl", label: "First" }]);
	});

	it("returns undefined for a missing file without throwing", async () => {
		expect(await loadSessionTabs(memoryStorage(), "tabs.json", "C:/work")).toBeUndefined();
	});

	it("discards corrupt, wrong-version, and foreign-project payloads", async () => {
		expect(await loadSessionTabs(memoryStorage("not json"), "tabs.json", "C:/work")).toBeUndefined();
		expect(
			await loadSessionTabs(
				memoryStorage(JSON.stringify({ version: 999, projectKey: "C:/work", tabs: [] })),
				"tabs.json",
				"C:/work",
			),
		).toBeUndefined();
		expect(
			await loadSessionTabs(
				memoryStorage(JSON.stringify({ version: 1, projectKey: "C:/other", tabs: [] })),
				"tabs.json",
				"C:/work",
			),
		).toBeUndefined();
		const filtered = await loadSessionTabs(
			memoryStorage(
				JSON.stringify({
					version: 1,
					projectKey: "C:/work",
					tabs: [{ path: "" }, { label: "no path" }, { path: "/sessions/ok.jsonl" }],
				}),
			),
			"tabs.json",
			"C:/work",
		);
		expect(filtered?.tabs).toEqual([{ path: "/sessions/ok.jsonl" }]);
	});

	it("bounds restored tabs and recently closed entries", async () => {
		const tabs = new SessionTabs();
		for (let index = 0; index < MAX_RESTORED_TABS + 5; index++) tabs.open(`/sessions/${index}.jsonl`);
		for (let index = 0; index < MAX_PERSISTED_CLOSED_TABS + 5; index++) {
			tabs.open(`/sessions/closed-${index}.jsonl`);
			tabs.close(`/sessions/closed-${index}.jsonl`);
		}
		const storage = memoryStorage();
		saveSessionTabs(storage, "tabs.json", snapshotSessionTabs(tabs, { projectKey: "C:/work" }));
		const loaded = await loadSessionTabs(storage, "tabs.json", "C:/work");
		expect(loaded?.tabs).toHaveLength(MAX_RESTORED_TABS);
		expect(loaded?.recentlyClosed).toHaveLength(MAX_PERSISTED_CLOSED_TABS);
	});

	it("never lets a persistence failure break the caller", () => {
		const storage = memoryStorage();
		const failing = {
			...storage,
			writeTextSync: () => {
				throw new Error("disk full");
			},
		};
		expect(() =>
			saveSessionTabs(failing, "tabs.json", snapshotSessionTabs(new SessionTabs(), { projectKey: "C:/work" })),
		).not.toThrow();
	});
});
