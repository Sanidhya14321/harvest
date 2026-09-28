import { describe, expect, it } from "bun:test";
import { SessionTabs } from "../../src/session/session-tabs";

describe("session tabs", () => {
	it("cycles among open sessions and removes an inactive tab without changing the active path", () => {
		const tabs = new SessionTabs();
		tabs.open("C:/work/one.jsonl");
		tabs.open("C:/work/two.jsonl");
		tabs.open("C:/work/three.jsonl");
		expect(tabs.neighbor("C:/work/three.jsonl", 1)).toBe("C:/work/one.jsonl");
		expect(tabs.neighbor("C:/work/one.jsonl", -1)).toBe("C:/work/three.jsonl");
		expect(tabs.close("C:/work/two.jsonl")).toBe(true);
		expect(tabs.neighbor("C:/work/one.jsonl", 1)).toBe("C:/work/three.jsonl");
	});

	it("does not duplicate a path and bounds the number of open tabs", () => {
		const tabs = new SessionTabs(2);
		tabs.open("C:/work/one.jsonl");
		tabs.open("C:/work/one.jsonl");
		expect(tabs.paths).toHaveLength(1);
		tabs.open("C:/work/two.jsonl");
		expect(() => tabs.open("C:/work/three.jsonl")).toThrow("At most 2 session tabs");
		expect(tabs.paths).toEqual(["C:/work/one.jsonl", "C:/work/two.jsonl"]);
	});

	it("keeps tab history and reopens a closed tab without losing its label", () => {
		const tabs = new SessionTabs();
		tabs.open("C:/work/one.jsonl", "First task");
		tabs.visit("C:/work/one.jsonl");
		tabs.open("C:/work/two.jsonl", "Second task");
		tabs.visit("C:/work/two.jsonl");
		expect(tabs.historyTarget(-1)).toBe("C:/work/one.jsonl");
		tabs.commitHistoryMove(-1);
		expect(tabs.historyTarget(1)).toBe("C:/work/two.jsonl");
		expect(tabs.close("C:/work/two.jsonl")).toBe(true);
		expect(tabs.historyTarget(1)).toBeUndefined();
		expect(tabs.reopen()).toBe("C:/work/two.jsonl");
		expect(tabs.label("C:/work/two.jsonl")).toBe("Second task");
	});
});
