import { describe, expect, it } from "bun:test";
import { resolveNavigationTarget, SessionTabs } from "../../src/session/session-tabs";

describe("SessionTabs stable IDs", () => {
	it("carries {id,path} entries and resolves both directions", () => {
		const tabs = new SessionTabs();
		tabs.open("/work/one.jsonl", "First", "session-one");
		tabs.open("/work/two.jsonl", "Second");
		tabs.noteId("/work/two.jsonl", "session-two");

		expect(tabs.entries).toEqual([
			{ path: "/work/one.jsonl", label: "First", sessionId: "session-one" },
			{ path: "/work/two.jsonl", label: "Second", sessionId: "session-two" },
		]);
		expect(tabs.idForPath("/work/one.jsonl")).toBe("session-one");
		expect(tabs.pathForId("session-two")).toBe("/work/two.jsonl");
		expect(tabs.pathForId("session-missing")).toBeUndefined();
	});

	it("drops the ID with the tab and restores it on reopen", () => {
		const tabs = new SessionTabs();
		tabs.open("/work/one.jsonl", "First", "session-one");
		expect(tabs.close("/work/one.jsonl")).toBe(true);
		expect(tabs.idForPath("/work/one.jsonl")).toBeUndefined();
		expect(tabs.recentlyClosed).toEqual([{ path: "/work/one.jsonl", label: "First", sessionId: "session-one" }]);
		expect(tabs.reopen()).toBe("/work/one.jsonl");
		expect(tabs.idForPath("/work/one.jsonl")).toBe("session-one");
	});

	it("tabs stay usable while callers migrate to selectById", () => {
		const tabs = new SessionTabs();
		tabs.open("/work/one.jsonl");
		// Path-keyed navigation keeps working when no ID was ever noted.
		expect(tabs.entries).toEqual([{ path: "/work/one.jsonl" }]);
		expect(tabs.neighbor("/work/one.jsonl", 1)).toBeUndefined();
	});
});

describe("closeLastTabToHome", () => {
	it("closes the final tab into Home instead of refusing, remembering every tab", () => {
		const tabs = new SessionTabs();
		tabs.open("/work/one.jsonl", "First", "session-one");
		tabs.open("/work/two.jsonl", "Second", "session-two");

		const outcome = tabs.closeLastTabToHome();
		expect(outcome).toEqual({ closed: ["/work/one.jsonl", "/work/two.jsonl"], home: true });
		expect(tabs.paths).toEqual([]);
		// Every hidden tab is reopenable with its ID intact.
		expect(tabs.recentlyClosed).toHaveLength(2);
		expect(tabs.reopen()).toBe("/work/two.jsonl");
		expect(tabs.idForPath("/work/two.jsonl")).toBe("session-two");
	});
});

describe("resolveNavigationTarget", () => {
	const warm = [
		{ id: "session-one", path: "/work/one.jsonl" },
		{ id: "session-two", path: "/work/two.jsonl" },
	];

	it("prefers the stable session ID even when the path moved", () => {
		// Failure mode: navigation re-reads a stale path from disk while the
		// warm runtime already lives under a new file.
		const target = resolveNavigationTarget({ id: "session-two", path: "/work/stale.jsonl" }, warm, () => true);
		expect(target).toEqual({ kind: "warm-id", sessionId: "session-two", path: "/work/two.jsonl" });
	});

	it("falls back to a warm path match, then a cold open, then missing", () => {
		expect(resolveNavigationTarget({ path: "/work/one.jsonl" }, warm, () => false)).toEqual({
			kind: "warm-path",
			sessionId: "session-one",
			path: "/work/one.jsonl",
		});
		expect(resolveNavigationTarget({ path: "/work/cold.jsonl" }, warm, () => true)).toEqual({
			kind: "cold",
			sessionId: undefined,
			path: "/work/cold.jsonl",
		});
		expect(resolveNavigationTarget({ path: "/work/gone.jsonl" }, warm, () => false)).toBeUndefined();
	});

	it("resolves keyboard and mouse paths identically (warm-first parity)", () => {
		// Failure mode: keyboard next/prev and strip clicks resolve the same
		// tab differently (one warm, one cold), landing on different runtimes.
		const tabs = new SessionTabs();
		tabs.open("/work/one.jsonl", "First", "session-one");
		tabs.open("/work/two.jsonl", "Second", "session-two");
		// Keyboard: neighbor path without an ID.
		const keyboardPath = tabs.neighbor("/work/one.jsonl", 1)!;
		const viaKeyboard = resolveNavigationTarget({ path: keyboardPath }, warm, () => true);
		// Mouse: same tab path with the snapshot ID the strip feed provides.
		const viaMouse = resolveNavigationTarget(
			{ id: tabs.idForPath(keyboardPath), path: keyboardPath },
			warm,
			() => true,
		);
		expect(viaKeyboard?.sessionId).toBe("session-two");
		expect(viaMouse?.sessionId).toBe("session-two");
		// Both paths land on the same runtime and file; only the resolution
		// evidence (`kind`) differs, since only the mouse path knew the ID.
		expect(viaKeyboard).toMatchObject({ sessionId: viaMouse?.sessionId, path: viaMouse?.path });
		expect(viaKeyboard?.sessionId).toBe("session-two");
	});
});
