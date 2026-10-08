import { afterEach, beforeEach, describe, expect, test, vi } from "bun:test";
import { stripVTControlCharacters } from "node:util";
import { SessionSelectorComponent } from "@harvest/pi-coding-agent/modes/components/session-selector";
import { createTheme, getBuiltinThemes } from "@harvest/pi-coding-agent/modes/theme/loader";
import { setThemeInstance, theme } from "@harvest/pi-coding-agent/modes/theme/theme";
import type { SessionInfo } from "@harvest/pi-coding-agent/session/session-listing";
import { visibleWidth } from "@harvest/pi-tui";

let previousTheme = theme;
beforeEach(() => {
	previousTheme = theme;
	setThemeInstance(createTheme(getBuiltinThemes().harvest!, { mode: "truecolor", symbolPresetOverride: "ascii" }));
});
afterEach(() => setThemeInstance(previousTheme));

const sessions: SessionInfo[] = Array.from({ length: 12 }, (_, index) => ({
	id: `s${index}`, path: `/work/session-${index}.jsonl`, cwd: "/work", title: `Choice ${index}`,
	created: new Date("2026-01-01"), modified: new Date("2026-01-02"), size: 1024, messageCount: 2,
	firstMessage: `Prompt ${index}`, allMessagesText: `Prompt ${index}`, status: "complete",
}));
const plain = (lines: readonly string[]): string => stripVTControlCharacters(lines.join("\n"));

describe("session picker allocation", () => {
	test("one to four allocated rows keep the selected session reachable after navigation and resizing", () => {
		const selected = vi.fn();
		const selector = new SessionSelectorComponent(sessions, selected, () => {}, () => {}, { getTerminalRows: () => 40, fillHeight: true });
		for (let i = 0; i < 7; i++) selector.handleInput("\x1b[B");
		for (const height of [4, 3, 2, 1]) {
			selector.setMaxHeight(height);
			const lines = selector.render(24);
			expect(lines).toHaveLength(height);
			expect(plain(lines)).toContain("Choice 7");
			for (const line of lines) {
				expect(visibleWidth(line)).toBe(24);
				expect(stripVTControlCharacters(line)).not.toMatch(/[^\x00-\x7f]/);
			}
		}
		selector.handleInput("\r");
		expect(selected).toHaveBeenCalledWith(sessions[7]);
	});

	test("the compact physical row map rejects a footer click and returns the complete visible session", () => {
		const selected = vi.fn();
		const selector = new SessionSelectorComponent(sessions, selected, () => {}, () => {}, { fillHeight: true });
		selector.handleInput("\x1b[B");
		selector.setMaxHeight(4);
		const lines = selector.render(24);
		selector.handleInput(`\x1b[<0;4;${lines.length}M`);
		expect(selected).not.toHaveBeenCalled();
		const choiceLine = lines.findIndex(line => plain([line]).includes("Choice 1"));
		selector.handleInput(`\x1b[<0;4;${choiceLine + 1}M`);
		expect(selected).toHaveBeenCalledWith(sessions[1]);
	});

	test("widths one through three collapse scrollbar and inset while retaining the selected session", () => {
		const selected = vi.fn();
		const selector = new SessionSelectorComponent(sessions, selected, () => {}, () => {}, { pinnedIds: new Set(["s0"]) });
		selector.setMaxHeight(1);
		for (const width of [1, 2, 3]) {
			const lines = selector.render(width);
			expect(lines).toHaveLength(1);
			expect(visibleWidth(lines[0]!)).toBe(width);
			expect(plain(lines)).not.toMatch(/[^\x00-\x7f]/);
		}
		selector.setMaxHeight(8);
		expect(plain(selector.render(60))).toContain("Choice 0");
		selector.handleInput("\r");
		expect(selected).toHaveBeenCalledWith(sessions[0]);
	});

	test("short delete confirmations expose the action and preserve the explicit busy-session stop gate", async () => {
		const remove = vi.fn(async () => true);
		const stopAndDelete = vi.fn(async () => true);
		const selector = new SessionSelectorComponent(sessions, () => {}, () => {}, () => {}, { onDelete: remove, isSessionBusy: () => true, onStopAndDelete: stopAndDelete });
		selector.setMaxHeight(1);
		selector.handleInput("\x1b[3~");
		const lines = selector.render(24);
		expect(lines).toHaveLength(1);
		expect(plain(lines)).toContain("Stop & Delete");
		expect(remove).not.toHaveBeenCalled();
		expect(stopAndDelete).not.toHaveBeenCalled();
		selector.handleInput("\r");
		await Promise.resolve();
		expect(stopAndDelete).toHaveBeenCalledWith(sessions[0]);
		expect(remove).not.toHaveBeenCalled();
	});

	test("Ctrl+N creates a session even when there is no row available for the mouse action", () => {
		const create = vi.fn();
		const select = vi.fn();
		const selector = new SessionSelectorComponent(sessions, select, () => {}, () => {}, { onNewSession: create });
		selector.setMaxHeight(1);
		selector.render(24);
		selector.handleInput("\x0e");
		expect(create).toHaveBeenCalledTimes(1);
		expect(select).not.toHaveBeenCalled();
	});

	test("navigating an empty result then changing scope never leaves a negative selection", async () => {
		const select = vi.fn();
		const selector = new SessionSelectorComponent([], select, () => {}, () => {}, { allSessions: sessions });
		selector.handleInput("\x1b[B");
		selector.handleInput("\t");
		await Promise.resolve();
		selector.setMaxHeight(1);
		expect(plain(selector.render(24))).toContain("Choice 0");
		selector.handleInput("\r");
		expect(select).toHaveBeenCalledWith(sessions[0]);
	});
});
