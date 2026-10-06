import { afterEach, describe, expect, it, vi } from "bun:test";
import * as fs from "node:fs/promises";
import * as os from "node:os";
import * as path from "node:path";
import { SelectorController } from "../../../src/modes/controllers/selector-controller";
import type { InteractiveModeContext } from "../../../src/modes/types";

const roots: string[] = [];
afterEach(async () => {
	await Promise.all(roots.splice(0).map(root => fs.rm(root, { recursive: true, force: true })));
});

it("closes the active tab by switching first, then reopens the saved session", async () => {
	const root = await fs.mkdtemp(path.join(os.tmpdir(), "harvest-session-tabs-"));
	roots.push(root);
	const first = path.join(root, "first.jsonl");
	const second = path.join(root, "second.jsonl");
	await Bun.write(first, "first");
	await Bun.write(second, "second");
	let active = first;
	const handleResumeSession = vi.fn(async (target: string) => {
		active = target;
	});
	const ctx = {
		sessionManager: {
			getSessionFile: () => active,
			getSessionName: () => "First task",
		},
		handleResumeSession,
		ui: { requestRender: vi.fn() },
		clearHomeDetached: () => {},
		enterHomeDetached: () => {},
		isHomeDetached: () => false,
	} as unknown as InteractiveModeContext;
	const controller = new SelectorController(ctx);
	controller.sessionTabs.open(second, "Second task");

	expect(await controller.handleSessionTabsCommand("close")).toBe("Closed session tab 1.");
	expect(active).toBe(second);
	expect(controller.sessionTabs.paths).toEqual([second]);
	expect(await controller.handleSessionTabsCommand("reopen")).toBe("Reopened session tab 2.");
	expect(active).toBe(first);
	expect(controller.sessionTabs.paths).toEqual([second, first]);
	expect(handleResumeSession).toHaveBeenCalledTimes(2);
});

it("moves through tab history only after the session switch succeeds", async () => {
	const root = await fs.mkdtemp(path.join(os.tmpdir(), "harvest-session-tabs-"));
	roots.push(root);
	const first = path.join(root, "one.jsonl");
	const second = path.join(root, "two.jsonl");
	await Bun.write(first, "first");
	await Bun.write(second, "second");
	let active = second;
	let allowSwitch = false;
	const ctx = {
		sessionManager: {
			getSessionFile: () => active,
			getSessionName: () => "Second task",
		},
		handleResumeSession: async (target: string) => {
			if (allowSwitch) active = target;
		},
		ui: { requestRender: vi.fn() },
		clearHomeDetached: () => {},
		enterHomeDetached: () => {},
		isHomeDetached: () => false,
	} as unknown as InteractiveModeContext;
	const controller = new SelectorController(ctx);
	controller.sessionTabs.open(first, "First task");
	controller.sessionTabs.visit(first);
	controller.sessionTabs.visit(second);

	expect(await controller.handleSessionTabsCommand("back")).toContain("did not complete");
	expect(controller.sessionTabs.historyTarget(-1)).toBe(first);
	allowSwitch = true;
	expect(await controller.handleSessionTabsCommand("back")).toContain("Switched to session tab");
	expect(active).toBe(first);
	expect(controller.sessionTabs.historyTarget(1)).toBe(second);
});

it("does not restart the active session when its tab number is selected", async () => {
	const root = await fs.mkdtemp(path.join(os.tmpdir(), "harvest-session-tabs-"));
	roots.push(root);
	const active = path.join(root, "active.jsonl");
	await Bun.write(active, "active");
	const handleResumeSession = vi.fn(async () => {});
	const ctx = {
		sessionManager: { getSessionFile: () => active, getSessionName: () => "Active" },
		handleResumeSession,
		ui: { requestRender: vi.fn() },
	} as unknown as InteractiveModeContext;
	const controller = new SelectorController(ctx);
	expect(await controller.handleSessionTabsCommand("switch 1")).toBe("Already viewing session tab 1.");
	expect(handleResumeSession).not.toHaveBeenCalled();
});

it("removes a tab whose session file disappeared before navigation", async () => {
	const root = await fs.mkdtemp(path.join(os.tmpdir(), "harvest-session-tabs-"));
	roots.push(root);
	const active = path.join(root, "active.jsonl");
	const missing = path.join(root, "missing.jsonl");
	await Bun.write(active, "active");
	const handleResumeSession = vi.fn(async () => {});
	const ctx = {
		sessionManager: { getSessionFile: () => active, getSessionName: () => "Active" },
		handleResumeSession,
		ui: { requestRender: vi.fn() },
	} as unknown as InteractiveModeContext;
	const controller = new SelectorController(ctx);
	controller.sessionTabs.open(missing);
	expect(await controller.handleSessionTabsCommand("next")).toContain("tab was removed");
	expect(controller.sessionTabs.paths).toEqual([active]);
	expect(handleResumeSession).not.toHaveBeenCalled();
});

it("keeps the active session when a clicked tab has been deleted", async () => {
	const root = await fs.mkdtemp(path.join(os.tmpdir(), "harvest-session-tabs-"));
	roots.push(root);
	const active = path.join(root, "active.jsonl");
	const missing = path.join(root, "deleted.jsonl");
	await Bun.write(active, "active");
	const showError = vi.fn();
	const ctx = {
		sessionManager: { getSessionFile: () => active, getSessionName: () => "Active" },
		showError,
		ui: { requestRender: vi.fn() },
	} as unknown as InteractiveModeContext;
	const controller = new SelectorController(ctx);
	controller.sessionTabs.open(missing);
	expect(await controller.handleResumeSession(missing)).toBe(false);
	expect(controller.sessionTabs.paths).toEqual([active]);
	expect(showError).toHaveBeenCalledWith(expect.stringContaining("no longer available"));
});

it("tracks persisted sessions while removing a moved path", async () => {
	const root = await fs.mkdtemp(path.join(os.tmpdir(), "harvest-session-tabs-"));
	roots.push(root);
	const first = path.join(root, "first.jsonl");
	const fork = path.join(root, "fork.jsonl");
	const moved = path.join(root, "moved.jsonl");
	await Bun.write(first, "first");
	await Bun.write(fork, "fork");
	let active = first;
	const ctx = {
		sessionManager: {
			getSessionFile: () => active,
			getSessionName: () => "Task",
		},
		ui: { requestRender: vi.fn() },
	} as unknown as InteractiveModeContext;
	const controller = new SelectorController(ctx);
	active = fork;
	await controller.recordSessionTransition(first);
	expect(controller.sessionTabs.paths).toEqual([first, fork]);
	active = moved;
	await controller.recordSessionTransition(fork, true);
	expect(controller.sessionTabs.paths).toEqual([first, moved]);
});

it("does not leave an unsaved startup session as a ghost tab after creating a new session", async () => {
	const root = await fs.mkdtemp(path.join(os.tmpdir(), "harvest-session-tabs-"));
	roots.push(root);
	const startup = path.join(root, "startup.jsonl");
	const next = path.join(root, "next.jsonl");
	await Bun.write(next, "next");
	let active = startup;
	const ctx = {
		sessionManager: { getSessionFile: () => active, getSessionName: () => undefined },
		ui: { requestRender: vi.fn() },
	} as unknown as InteractiveModeContext;
	const controller = new SelectorController(ctx);
	expect(await controller.handleSessionTabsCommand("list")).toContain("New session");
	active = next;
	await controller.recordSessionTransition(startup);
	expect(controller.sessionTabs.paths).toEqual([next]);
	expect(controller.sessionTabs.historyTarget(-1)).toBeUndefined();
});

it("forgets a closed tab whose session file was deleted", async () => {
	let active = "/work/active.jsonl";
	const ctx = {
		sessionManager: { getSessionFile: () => active, getSessionName: () => "Active" },
		handleResumeSession: async (target: string) => {
			active = target;
		},
		ui: { requestRender: vi.fn() },
	} as unknown as InteractiveModeContext;
	const controller = new SelectorController(ctx);
	controller.sessionTabs.open("/work/deleted.jsonl");
	controller.sessionTabs.close("/work/deleted.jsonl");
	expect(await controller.handleSessionTabsCommand("reopen")).toContain("no longer available");
	expect(await controller.handleSessionTabsCommand("reopen")).toBe("No recently closed session tab.");
});

describe("tab persistence wiring", () => {
	it("writes tab references when tabs mutate", async () => {
		const root = await fs.mkdtemp(path.join(os.tmpdir(), "harvest-session-tabs-"));
		roots.push(root);
		const first = path.join(root, "first.jsonl");
		const second = path.join(root, "second.jsonl");
		await Bun.write(first, "first");
		await Bun.write(second, "second");
		const active = second;
		const ctx = {
			sessionManager: {
				getSessionFile: () => active,
				getSessionName: () => "Second task",
				getSessionDir: () => root,
				getSessionId: () => "session-two",
				getCwd: () => root,
			},
			ui: { requestRender: vi.fn() },
		} as unknown as InteractiveModeContext;
		const controller = new SelectorController(ctx);
		await controller.recordSessionTransition(first);
		const tabsFile = path.join(root, "tabs.json");
		const persisted = (await Bun.file(tabsFile).json()) as {
			version: number;
			tabs: { path: string; sessionId?: string }[];
			activePath: string;
			activeSessionId?: string;
		};
		expect(persisted.version).toBe(2);
		expect(persisted.tabs.map(tab => tab.path).sort()).toEqual([first, second].sort());
		expect(persisted.activePath).toBe(second);
		expect(persisted.activeSessionId).toBe("session-two");
		expect(await controller.handleSessionTabsCommand("close 2")).toContain("Closed session tab");
		const afterClose = (await Bun.file(tabsFile).json()) as { tabs: { path: string }[] };
		expect(afterClose.tabs.map(tab => tab.path)).toEqual([second]);
	});
});
