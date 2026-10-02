import { beforeAll, describe, expect, it } from "bun:test";
import { stripVTControlCharacters } from "node:util";
import {
	describeSelectedSession,
	mergeRegistrySnapshot,
	SessionTabStrip,
} from "../../../src/modes/components/session-tab-strip";
import { initTheme } from "../../../src/modes/theme/theme";
import { SessionTabs } from "../../../src/session/session-tabs";

beforeAll(async () => {
	await initTheme(false);
});

it("does not switch hidden or clipped workspace tabs and clears their hover target", () => {
	const tabs = new SessionTabs();
	tabs.open("/work/first.jsonl", "First task");
	tabs.open("/work/second.jsonl", "Second task");
	let selected = "";
	const strip = new SessionTabStrip(
		tabs,
		() => "/work/first.jsonl",
		() => "First task",
		async path => {
			selected = path;
		},
	);
	const row = Bun.stripANSI(strip.renderWorkspace(100, true)[0]);
	const col = row.indexOf("Second task");
	expect(strip.hoverWorkspace(0, col)).toBe(true);
	expect(strip.hoverWorkspace(0, col)).toBe(false);
	expect(strip.renderWorkspace(100, true, 0)).toEqual([]);
	expect(strip.clickWorkspace(0, col)).toBe(false);
	expect(strip.hoverWorkspace(0, col)).toBe(true);
	expect(selected).toBe("");
	strip.renderWorkspace(100, true);
	expect(strip.clickWorkspace(0, col)).toBe(true);
	expect(selected).toBe("/work/second.jsonl");
});

it("uses overflow arrows to reach sessions outside the visible window", () => {
	const tabs = new SessionTabs();
	for (let index = 1; index <= 12; index++) tabs.open(`/work/${index}.jsonl`, `Task ${index}`);
	let selected = "";
	const strip = new SessionTabStrip(
		tabs,
		() => "/work/6.jsonl",
		() => "Task 6",
		async path => {
			selected = path;
		},
	);
	const row = Bun.stripANSI(strip.renderWorkspace(200, true)[0]);
	expect(strip.clickWorkspace(0, row.indexOf("‹"))).toBe(true);
	expect(selected).toBe("/work/3.jsonl");
	expect(strip.clickWorkspace(0, row.indexOf("›"))).toBe(true);
	expect(selected).toBe("/work/9.jsonl");
});

it("shows open session titles and keeps control characters out of the tab strip", () => {
	const tabs = new SessionTabs();
	tabs.open("/work/first.jsonl", "First task");
	tabs.open("/work/second.jsonl", "Second\t\x1b[31mtask");
	const strip = new SessionTabStrip(
		tabs,
		() => "/work/second.jsonl",
		() => "Second task",
		async () => {},
	);
	const text = stripVTControlCharacters(strip.render(80).join("\n"));
	expect(text).toContain("First task");
	expect(text).toContain("Second task");
	expect(text).not.toContain("\t");
});

it("shows the latest saved title after a named tab becomes inactive", () => {
	const tabs = new SessionTabs();
	tabs.open("/work/first.jsonl", "Untitled task");
	tabs.open("/work/second.jsonl", "Second task");
	const strip = new SessionTabStrip(
		tabs,
		() => "/work/second.jsonl",
		() => "Second task",
		async () => {},
	);
	tabs.open("/work/first.jsonl", "Investigate parser");
	const text = stripVTControlCharacters(strip.renderWorkspace(100, true).join(""));
	expect(text).toContain("Investigate parser");
	expect(text).not.toContain("Untitled task");
});

it("shows each live session's title and distinct activity state after switching", () => {
	const tabs = new SessionTabs();
	tabs.open("/work/first.jsonl", "Old title");
	tabs.open("/work/second.jsonl", "Second task");
	const strip = new SessionTabStrip(
		tabs,
		() => "/work/second.jsonl",
		() => "Second task",
		async () => {},
		path =>
			path === "/work/first.jsonl"
				? { id: "first", path, title: "Generated title", status: "running", unread: false, selected: false }
				: { id: "second", path, title: "Second task", status: "waiting", unread: true, selected: true },
	);
	const text = stripVTControlCharacters(strip.renderWorkspace(100, true).join(""));
	expect(text).toContain("● Generated title");
	expect(text).toContain("? Second task");
	expect(text).not.toContain("Old title");
});

it("keeps the active tab visible when many sessions are open", () => {
	const tabs = new SessionTabs();
	for (let index = 1; index <= 12; index++) tabs.open(`/work/${index}.jsonl`, `Task ${index}`);
	const strip = new SessionTabStrip(
		tabs,
		() => "/work/12.jsonl",
		() => "Task 12",
		async () => {},
	);
	const text = stripVTControlCharacters(strip.render(60).join("\n"));
	expect(text).toContain("Task 12");
	expect(text).not.toContain("Task 1 ");
});

it("hides a lone tab and uses a session title instead of a filename", () => {
	const tabs = new SessionTabs();
	tabs.open("/work/2026-09-28T18-11-33.jsonl");
	const strip = new SessionTabStrip(
		tabs,
		() => "/work/2026-09-28T18-11-33.jsonl",
		() => undefined,
		async () => {},
	);
	expect(strip.render(80)).toEqual([]);
	tabs.open("/work/other.jsonl", "Other task");
	const text = stripVTControlCharacters(strip.render(80).join("\n"));
	expect(text).toContain("New session");
	expect(text).not.toContain("2026-09-28");
});

it("opens and switches workspace sessions from their visible tab hit targets", () => {
	const tabs = new SessionTabs();
	tabs.open("/work/first.jsonl", "First task");
	tabs.open("/work/second.jsonl", "Second task");
	let created = 0;
	let selected: string | undefined;
	const strip = new SessionTabStrip(
		tabs,
		() => "/work/first.jsonl",
		() => "First task",
		async path => {
			selected = path;
		},
	);
	strip.setOnNew(() => {
		created++;
	});
	const row = stripVTControlCharacters(strip.renderWorkspace(80, true).join(""));
	expect(strip.clickWorkspace(0, row.indexOf("Second task") + 2)).toBe(true);
	expect(selected).toBe("/work/second.jsonl");
	expect(strip.clickWorkspace(0, row.indexOf("New session") + 2)).toBe(true);
	expect(created).toBe(1);
	strip.renderWorkspace(80, false);
	tabs.close("/work/second.jsonl");
	strip.renderWorkspace(80, false);
	expect(strip.clickWorkspace(0, row.indexOf("New session") + 2)).toBe(false);
});

it("keeps the selected workspace tab visible in a narrow terminal", () => {
	const tabs = new SessionTabs();
	for (let index = 1; index <= 12; index++) tabs.open(`/work/${index}.jsonl`, `Task ${index}`);
	const strip = new SessionTabStrip(
		tabs,
		() => "/work/12.jsonl",
		() => "Task 12",
		async () => {},
	);
	const rows = strip.renderWorkspace(24, true).map(row => stripVTControlCharacters(row));
	expect(rows.join(" ")).toContain("12");
	expect(rows.every(row => Bun.stringWidth(row) <= 24)).toBe(true);
});

describe("describeSelectedSession", () => {
	const base = {
		sessionId: "abc",
		sessionFile: "/work/active.jsonl",
		sessionName: "Active task",
		path: "/work/active.jsonl",
		isStreaming: false,
		approvalOpen: false,
	};

	it("marks a streaming session running and an approval-blocked one waiting", () => {
		expect(describeSelectedSession({ ...base, isStreaming: true })?.status).toBe("running");
		expect(describeSelectedSession({ ...base, approvalOpen: true })?.status).toBe("waiting");
		expect(describeSelectedSession(base)?.status).toBe("idle");
	});

	it("resolves only the selected path and never carries unread", () => {
		expect(describeSelectedSession({ ...base, path: "/work/other.jsonl" })).toBeUndefined();
		expect(describeSelectedSession({ ...base, sessionFile: undefined })).toBeUndefined();
		expect(describeSelectedSession({ ...base, isStreaming: true })).toMatchObject({
			id: "abc",
			selected: true,
			unread: false,
		});
	});

	it("paints the running indicator on the selected tab through the strip feed", () => {
		const tabs = new SessionTabs();
		tabs.open("/work/active.jsonl", "Active task");
		tabs.open("/work/other.jsonl", "Other task");
		const strip = new SessionTabStrip(
			tabs,
			() => "/work/active.jsonl",
			() => "Active task",
			async () => {},
			path => describeSelectedSession({ ...base, path, isStreaming: path === "/work/active.jsonl" }),
		);
		const text = stripVTControlCharacters(strip.renderWorkspace(100, true).join(""));
		expect(text).toContain("● Active task");
		expect(text).toContain("Other task");
		expect(text).not.toContain("? Active task");
	});
});

describe("mergeRegistrySnapshot", () => {
	const live = {
		id: "bg",
		path: "/work/bg.jsonl",
		title: "Background title",
		status: "completed" as const,
		unread: true,
		selected: false,
	};
	const selected = {
		id: "main",
		path: "/work/main.jsonl",
		title: "Main title",
		status: "running" as const,
		unread: false,
		selected: true,
	};

	it("falls back to whichever side resolves", () => {
		expect(mergeRegistrySnapshot(undefined, selected, false)).toBe(selected);
		expect(mergeRegistrySnapshot(live, undefined, false)).toBe(live);
		expect(mergeRegistrySnapshot(undefined, undefined, false)).toBeUndefined();
	});

	it("lets the visible tab own selection and title, keeping background state", () => {
		const merged = mergeRegistrySnapshot(live, selected, false);
		expect(merged).toMatchObject({
			id: "bg",
			selected: true,
			title: "Main title",
			status: "completed",
			unread: true,
		});
	});

	it("forces waiting while an approval holds the editor", () => {
		const merged = mergeRegistrySnapshot(live, { ...selected, status: "idle" as const }, true);
		expect(merged?.status).toBe("waiting");
		const background = mergeRegistrySnapshot(live, { ...selected, selected: false }, true);
		expect(background?.status).toBe("completed");
	});
});
