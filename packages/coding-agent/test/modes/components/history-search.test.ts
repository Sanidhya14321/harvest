import { afterEach, beforeEach, describe, expect, it, vi } from "bun:test";
import { stripVTControlCharacters } from "node:util";
import { visibleWidth } from "@harvest/pi-tui";
import { HistorySearchComponent } from "@harvest/pi-coding-agent/modes/components/history-search";
import { createTheme, getBuiltinThemes } from "@harvest/pi-coding-agent/modes/theme/loader";
import { setThemeInstance, theme, type Theme } from "@harvest/pi-coding-agent/modes/theme/theme";
import type { HistoryEntry, HistoryStorage } from "@harvest/pi-coding-agent/session/history-storage";

let previousTheme: Theme;
beforeEach(() => {
	previousTheme = theme;
	setThemeInstance(createTheme(getBuiltinThemes().harvest, { mode: "truecolor" }));
});
afterEach(() => {
	setThemeInstance(previousTheme);
});

const NOW_SECONDS = Math.floor(Date.now() / 1000);

function makeEntry(id: number, prompt: string, ageSeconds = 0): HistoryEntry {
	return { id, prompt, created_at: NOW_SECONDS - ageSeconds };
}

/** Minimal in-memory stand-in matching the two methods the component touches. */
function fakeStorage(entries: HistoryEntry[]): HistoryStorage {
	const tokenize = (q: string) =>
		q
			.toLowerCase()
			.split(/[^\p{L}\p{N}]+/u)
			.filter(Boolean);
	return {
		getRecent: (limit: number) => entries.slice(0, limit),
		search: (query: string, limit: number) => {
			const tokens = tokenize(query);
			return entries.filter(e => tokens.every(t => e.prompt.toLowerCase().includes(t))).slice(0, limit);
		},
	} as unknown as HistoryStorage;
}

function render(component: HistorySearchComponent, width = 80): { raw: string; plain: string } {
	const lines = component.render(width);
	const raw = lines.join("\n");
	return { raw, plain: Bun.stripANSI(raw) };
}

function type(component: HistorySearchComponent, text: string): void {
	for (const char of text) component.handleInput(char);
}

describe("HistorySearchComponent", () => {
	it("retains the navigated selection after shrinking and selects the complete prompt", () => {
		const entries = Array.from({ length: 20 }, (_, id) => makeEntry(id, `item ${id} full saved prompt`));
		const select = vi.fn();
		const component = new HistorySearchComponent(fakeStorage(entries), select, () => {});
		component.render(80);
		for (let i = 0; i < 12; i++) component.handleInput("\x1b[B");
		component.setMaxHeight(4);
		const small = component.render(30);
		expect(small.length).toBeLessThanOrEqual(4);
		expect(stripVTControlCharacters(small.join("\n"))).toContain("item 12");
		component.setMaxHeight(1);
		expect(stripVTControlCharacters(component.render(30).join("\n"))).toContain("item 12");
		component.handleInput("\r");
		expect(select).toHaveBeenCalledWith(entries[12]!.prompt);
	});

	it("keeps the query editable at one row and restores filtered results after expanding", () => {
		setThemeInstance(createTheme(getBuiltinThemes().harvest, { mode: "none", symbolPresetOverride: "ascii" }));
		const selected = vi.fn();
		const component = new HistorySearchComponent(
			fakeStorage([makeEntry(1, "wanted complete prompt"), makeEntry(2, "other")]),
			selected,
			() => {},
		);
		component.setMaxHeight(1);
		type(component, "wanted");
		const tiny = component.render(3);
		expect(tiny).toHaveLength(1);
		expect(visibleWidth(tiny[0]!)).toBe(3);
		component.setMaxHeight(8);
		const expanded = stripVTControlCharacters(component.render(40).join("\n"));
		expect(expanded).toContain("wanted complete prompt");
		expect(expanded).not.toContain("other");
		expect(expanded).not.toMatch(/[^\x20-\x7e\n]/);
		component.handleInput("\r");
		expect(selected).toHaveBeenCalledWith("wanted complete prompt");
	});
	it("paints the selected row with the selectedBg highlight bar and a relative timestamp", () => {
		const component = new HistorySearchComponent(
			fakeStorage([makeEntry(1, "deploy the release"), makeEntry(2, "older prompt", 7200)]),
			() => {},
			() => {},
		);

		const { raw, plain } = render(component);

		expect(plain).toContain("deploy the release");
		// First (default-selected) row carries the selection background.
		const selectedRow = raw.split("\n").find(line => line.includes("deploy the release"));
		expect(selectedRow).toContain(theme.getBgAnsi("selectedBg"));
		// Fresh entry renders the compact "now" age marker.
		expect(plain).toContain("now");
	});

	it("highlights the matched query tokens within results", () => {
		const component = new HistorySearchComponent(
			fakeStorage([makeEntry(1, "deploy the needle rollback"), makeEntry(2, "routine status update")]),
			() => {},
			() => {},
		);

		type(component, "needle");

		const { raw, plain } = render(component);
		expect(plain).toContain("deploy the needle rollback");
		expect(plain).not.toContain("routine status update");
		// The matched substring is wrapped in the accent color.
		expect(raw).toContain(theme.fg("accent", "needle"));
	});

	it("distinguishes an empty query from an unmatched query", () => {
		const empty = new HistorySearchComponent(
			fakeStorage([]),
			() => {},
			() => {},
		);
		expect(render(empty).plain).toContain("No history yet");

		const unmatched = new HistorySearchComponent(
			fakeStorage([makeEntry(1, "deploy the release")]),
			() => {},
			() => {},
		);
		type(unmatched, "zzzz");
		expect(render(unmatched).plain).toContain("No matching history");
	});
});
