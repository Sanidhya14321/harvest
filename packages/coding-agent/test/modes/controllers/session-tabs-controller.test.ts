import { afterEach, expect, it, vi } from "bun:test";
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
	let active = "/work/two.jsonl";
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
	} as unknown as InteractiveModeContext;
	const controller = new SelectorController(ctx);
	controller.sessionTabs.open("/work/one.jsonl", "First task");
	controller.sessionTabs.visit("/work/one.jsonl");
	controller.sessionTabs.visit("/work/two.jsonl");

	expect(await controller.handleSessionTabsCommand("back")).toContain("did not complete");
	expect(controller.sessionTabs.historyTarget(-1)).toBe("/work/one.jsonl");
	allowSwitch = true;
	expect(await controller.handleSessionTabsCommand("back")).toContain("Switched to session tab");
	expect(active).toBe("/work/one.jsonl");
	expect(controller.sessionTabs.historyTarget(1)).toBe("/work/two.jsonl");
});

it("tracks new and forked sessions while removing a moved or dropped path", () => {
	let active = "/work/first.jsonl";
	const ctx = {
		sessionManager: {
			getSessionFile: () => active,
			getSessionName: () => "Task",
		},
		ui: { requestRender: vi.fn() },
	} as unknown as InteractiveModeContext;
	const controller = new SelectorController(ctx);
	active = "/work/fork.jsonl";
	controller.recordSessionTransition("/work/first.jsonl");
	expect(controller.sessionTabs.paths).toEqual(["/work/first.jsonl", "/work/fork.jsonl"]);
	active = "/moved/fork.jsonl";
	controller.recordSessionTransition("/work/fork.jsonl", true);
	expect(controller.sessionTabs.paths).toEqual(["/work/first.jsonl", "/moved/fork.jsonl"]);
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
