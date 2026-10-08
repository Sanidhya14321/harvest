import { describe, expect, it } from "bun:test";
import { SessionTabs, translateStripHitToLocal } from "@harvest/pi-coding-agent/session/session-tabs";

function threeTabs(): SessionTabs {
	const tabs = new SessionTabs();
	tabs.open("/s/first.jsonl", "First");
	tabs.open("/s/second.jsonl", "Second");
	tabs.open("/s/third.jsonl", "Third");
	return tabs;
}

describe("session tab close neighbor (S7 helper)", () => {
	it("closing a middle tab selects the right neighbor", () => {
		expect(threeTabs().closeNeighbor("/s/second.jsonl")).toBe("/s/third.jsonl");
	});

	it("closing the rightmost tab selects the left neighbor and never wraps", () => {
		expect(threeTabs().closeNeighbor("/s/third.jsonl")).toBe("/s/second.jsonl");
	});

	it("closing the leftmost tab selects the tab to its right", () => {
		expect(threeTabs().closeNeighbor("/s/first.jsonl")).toBe("/s/second.jsonl");
	});

	it("a lone tab has no close neighbor (owner enters detached Home)", () => {
		const tabs = new SessionTabs();
		tabs.open("/s/only.jsonl");
		expect(tabs.closeNeighbor("/s/only.jsonl")).toBeUndefined();
	});

	it("an unknown path has no close neighbor", () => {
		expect(threeTabs().closeNeighbor("/s/ghost.jsonl")).toBeUndefined();
	});

	it("keyboard next/prev keeps wrapping while close never does", () => {
		const tabs = threeTabs();
		expect(tabs.neighbor("/s/third.jsonl", 1)).toBe("/s/first.jsonl");
		expect(tabs.neighbor("/s/first.jsonl", -1)).toBe("/s/third.jsonl");
		expect(tabs.closeNeighbor("/s/third.jsonl")).toBe("/s/second.jsonl");
		expect(tabs.closeNeighbor("/s/first.jsonl")).toBe("/s/second.jsonl");
	});
});

describe("strip hit translation (S1 helper)", () => {
	it("translates screen hits into strip-local coordinates", () => {
		expect(translateStripHitToLocal(12, 30, { row: 10, col: 4 })).toEqual({ row: 2, col: 26 });
		expect(translateStripHitToLocal(10, 4, { row: 10, col: 4 })).toEqual({ row: 0, col: 0 });
	});

	it("drops hits above or left of the strip origin (never screen-row-zero)", () => {
		expect(translateStripHitToLocal(9, 30, { row: 10, col: 4 })).toBeUndefined();
		expect(translateStripHitToLocal(12, 3, { row: 10, col: 4 })).toBeUndefined();
		expect(translateStripHitToLocal(0, 0, { row: 10, col: 4 })).toBeUndefined();
	});

	it("rejects non-integer coordinates", () => {
		expect(translateStripHitToLocal(10.5, 4, { row: 10, col: 4 })).toBeUndefined();
		expect(translateStripHitToLocal(10, Number.NaN, { row: 10, col: 4 })).toBeUndefined();
	});
});
