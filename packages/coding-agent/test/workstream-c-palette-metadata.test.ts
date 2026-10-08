/**
 * Workstream C — palette metadata completeness, intent classification, and
 * budget sequences.
 *
 * Contracts (component internals + item metadata + budget math only; dispatch
 * semantics owned elsewhere and unchanged):
 * - Every registry kind (builtin/extension/skill/file/template/custom/mcp)
 *   projects to an actionable item: `/name` handler key, kind group,
 *   description hint, argument metadata (draft-vs-run intent + argHint).
 * - Unknown argument metadata fails safe to `draft` for every
 *   non-builtin/non-session kind; builtin/session keep the explicit contract.
 * - `resolvePaletteSelection` routing is unchanged (draft → draft text,
 *   otherwise dispatch text).
 * - Budget math accounts chrome (top/prompt/bottom) plus per-item description
 *   rows; open + resize sequences keep the selection visible.
 */
import { afterEach, beforeAll, describe, expect, it } from "bun:test";
import { KeybindingsManager } from "@harvest/pi-coding-agent/config/keybindings";
import {
	CommandPaletteComponent,
	mergePaletteItems,
	paletteListBudget,
	PALETTE_CHROME_ROWS,
	paletteOverlayRows,
	projectPendingSlashCommands,
	resolvePaletteSelection,
	TAB_MANAGEMENT_PALETTE_SOURCES,
	type PaletteCommandSource,
} from "@harvest/pi-coding-agent/modes/components/command-palette";
import { initTheme, setSymbolPreset, theme } from "@harvest/pi-coding-agent/modes/theme/theme";
import { setKeybindings } from "@harvest/pi-tui";

beforeAll(() => {
	initTheme();
});

afterEach(() => {
	setKeybindings(KeybindingsManager.inMemory());
});

function byId(ids: ReturnType<typeof mergePaletteItems>): Map<string, (typeof ids)[number]> {
	return new Map(ids.map(item => [item.id, item]));
}

describe("palette metadata completeness", () => {
	it("carries actionable metadata for every registry kind", () => {
		// Failure mode: an item without a handler key, kind, or intent cannot
		// be routed by the owner (builtin dispatch vs production submission).
		const sources: PaletteCommandSource[] = [
			{ name: "status", description: "builtin status", group: "builtin", allowArgs: false },
			{ name: "ext:deploy", description: "extension command", group: "extension", allowArgs: true },
			{ name: "skill:review", description: "skill prompt", group: "skill", allowArgs: false },
			{ name: "notes.md", description: "file command", group: "file" },
			{ name: "review", description: "prompt template", group: "template" },
			{ name: "mine", description: "custom command", group: "custom" },
			{ name: "server/tool", description: "mcp command", group: "mcp" },
		];
		const items = mergePaletteItems(sources);
		expect(items.length).toBe(sources.length);
		for (const item of items) {
			expect(item.id.startsWith("/")).toBe(true);
			expect(item.title).toBe(item.id);
			expect(item.group).toBeDefined();
			expect(typeof item.hint === "string" || item.hint === undefined).toBe(true);
		}
		const map = byId(items);
		expect(map.get("/status")?.group).toBe("builtin");
		expect(map.get("/ext:deploy")?.group).toBe("extension");
		expect(map.get("/skill:review")?.group).toBe("skill");
		expect(map.get("/notes.md")?.group).toBe("file");
		expect(map.get("/review")?.group).toBe("template");
		expect(map.get("/mine")?.group).toBe("custom");
		expect(map.get("/server/tool")?.group).toBe("mcp");
	});

	it("preserves static argument hints alongside intent", () => {
		// Failure mode: hint-loss — the palette advertises a runnable command
		// with no indication it takes arguments.
		const items = mergePaletteItems([
			{ name: "deploy", description: "deploy", group: "extension", allowArgs: true, inlineHint: "[env]" },
			{ name: "tab close", description: "close", group: "session", allowArgs: true, inlineHint: "[number]" },
		]);
		const map = byId(items);
		expect(map.get("/deploy")?.argHint).toBe("[env]");
		expect(map.get("/deploy")?.intent).toBe("draft");
		expect(map.get("/tab close")?.argHint).toBe("[number]");
	});
});

describe("palette intent classification", () => {
	it("drafts explicit argument-taking commands and runs explicit arg-free ones", () => {
		const items = mergePaletteItems([
			{ name: "a", description: "takes args", group: "extension", allowArgs: true },
			{ name: "b", description: "has subcommands", group: "skill", hasSubcommands: true },
			{ name: "c", description: "arg-free", group: "extension", allowArgs: false },
			{ name: "d", description: "builtin run", group: "builtin", allowArgs: false },
		]);
		const map = byId(items);
		expect(map.get("/a")?.intent).toBe("draft");
		expect(map.get("/b")?.intent).toBe("draft");
		expect(map.get("/c")?.intent ?? "run").toBe("run");
		expect(map.get("/d")?.intent ?? "run").toBe("run");
	});

	it("fails safe to draft on unknown metadata for every non-builtin/non-session kind", () => {
		// Failure mode: an extension/skill/custom/MCP/file/template command
		// with no recorded arg metadata bare-executes an incomplete command.
		const items = mergePaletteItems([
			{ name: "e1", description: "ext unknown", group: "extension" },
			{ name: "s1", description: "skill unknown", group: "skill" },
			{ name: "f1", description: "file unknown", group: "file" },
			{ name: "t1", description: "template unknown", group: "template" },
			{ name: "c1", description: "custom unknown", group: "custom" },
			{ name: "m1", description: "mcp unknown", group: "mcp" },
			{ name: "b1", description: "builtin unknown", group: "builtin" },
		]);
		const map = byId(items);
		for (const id of ["/e1", "/s1", "/f1", "/t1", "/c1", "/m1"]) {
			expect(map.get(id)?.intent).toBe("draft");
		}
		// Builtin keeps the explicit contract: unknown means runnable.
		expect(map.get("/b1")?.intent ?? "run").toBe("run");
	});

	it("keeps tab management verbs dispatchable with draft-vs-run intent intact", () => {
		const items = mergePaletteItems([...TAB_MANAGEMENT_PALETTE_SOURCES]);
		const map = byId(items);
		expect(map.get("/tab close")?.intent).toBe("draft");
		expect(map.get("/tab reopen")?.intent ?? "run").toBe("run");
		expect(map.get("/tab archive")?.intent).toBe("draft");
		expect(map.get("/tab restore")?.intent).toBe("draft");
	});

	it("does not change selection routing semantics", () => {
		// Failure mode: a metadata change accidentally reroutes dispatch
		// (draft items executing, run items drafting).
		expect(resolvePaletteSelection({ id: "/foo", title: "/foo", intent: "draft" })).toEqual({
			kind: "draft",
			text: "/foo ",
		});
		expect(resolvePaletteSelection({ id: "/foo", title: "/foo", group: "extension" })).toEqual({
			kind: "dispatch",
			text: "/foo",
		});
		expect(resolvePaletteSelection({ id: "bar", title: "bar" })).toEqual({ kind: "dispatch", text: "/bar" });
	});
});

describe("pending projection preserves unknown metadata (C1)", () => {
	const isBuiltin = (name: string): boolean => name === "status" || name === "tab close";

	it("never stamps false from absent metadata — unknown stays undefined", () => {
		// Failure mode: the owner projected `hasSubcommands: <boolean expr>`
		// (false when no factory), defeating the leaf unknown→draft fallback
		// so incomplete extension commands bare-executed.
		const sources = projectPendingSlashCommands(
			[
				{ name: "deploy", description: "deploy things" },
				{ name: "skill:review", description: "review skill" },
				{ name: "status", description: "builtin" },
			],
			isBuiltin,
		);
		expect(sources.map(source => source.name)).toEqual(["deploy", "skill:review"]);
		for (const source of sources) {
			expect(source.allowArgs).toBeUndefined();
			expect(source.hasSubcommands).toBeUndefined();
		}
		expect(sources[0]?.group).toBe("extension");
		expect(sources[1]?.group).toBe("skill");
	});

	it("passes authoritative facts through: explicit allowArgs plus factory-only subcommands", () => {
		const factory = (): string[] => [];
		const sources = projectPendingSlashCommands(
			[
				{ name: "a", allowArgs: true },
				{ name: "b", allowArgs: false },
				{ name: "c", getArgumentCompletions: factory },
				{ name: "d", getArgumentCompletions: undefined },
			],
			() => false,
		);
		const byName = new Map(sources.map(source => [source.name, source]));
		expect(byName.get("a")?.allowArgs).toBe(true);
		expect(byName.get("b")?.allowArgs).toBe(false);
		expect(byName.get("b")?.hasSubcommands).toBeUndefined();
		expect(byName.get("c")?.hasSubcommands).toBe(true);
		expect(byName.get("d")?.hasSubcommands).toBeUndefined();
	});

	it("projected sources classify end-to-end: unknown drafts, explicit arg-free runs", () => {
		const factory = (): string[] => [];
		const items = mergePaletteItems(
			projectPendingSlashCommands(
				[
					{ name: "unknown-cmd", description: "no metadata" },
					{ name: "runnable", description: "explicitly arg-free", allowArgs: false },
					{ name: "takes-args", description: "explicit args", allowArgs: true },
					{ name: "subbed", description: "factory", getArgumentCompletions: factory },
				],
				() => false,
			),
		);
		const map = byId(items);
		expect(map.get("/unknown-cmd")?.intent).toBe("draft");
		expect(map.get("/runnable")?.intent ?? "run").toBe("run");
		expect(map.get("/takes-args")?.intent).toBe("draft");
		expect(map.get("/subbed")?.intent).toBe("draft");
	});

	it("selected-item callbacks fire once: unknown selects draft, known selects dispatch", () => {
		// Failure mode: double-Enter dispatches twice, or Escape after Enter
		// re-fires close and restores a stale draft over the selection.
		setKeybindings(KeybindingsManager.inMemory());
		const items = mergePaletteItems([
			{ name: "unknown-cmd", description: "no metadata", group: "extension" },
			{ name: "runnable", description: "arg-free", group: "extension", allowArgs: false },
		]);
		const palette = new CommandPaletteComponent();
		palette.setItems(items);
		const picked: string[] = [];
		let closes = 0;
		palette.onSelect = item => picked.push(item.id);
		palette.onClose = () => closes++;
		palette.handleInput("\n");
		palette.handleInput("\n");
		palette.handleInput("\x1b");
		expect(picked).toEqual(["/unknown-cmd"]);
		expect(closes).toBe(0);
		expect(palette.isSettled()).toBe(true);
		expect(resolvePaletteSelection({ id: "/unknown-cmd", title: "/unknown-cmd", intent: "draft" })).toEqual({
			kind: "draft",
			text: "/unknown-cmd ",
		});

		const run = new CommandPaletteComponent();
		run.setItems(items);
		run.setQuery("runnable");
		const dispatched: string[] = [];
		run.onSelect = item => dispatched.push(item.id);
		run.handleInput("\n");
		expect(dispatched).toEqual(["/runnable"]);
		expect(resolvePaletteSelection({ id: "/runnable", title: "/runnable", group: "extension" })).toEqual({
			kind: "dispatch",
			text: "/runnable",
		});
	});

	it("escape closes once and repeated escape never re-fires", () => {
		setKeybindings(KeybindingsManager.inMemory());
		const palette = new CommandPaletteComponent();
		palette.setItems([{ id: "/a", title: "/a" }]);
		let closes = 0;
		palette.onClose = () => closes++;
		palette.handleInput("\x1b");
		palette.handleInput("\x1b");
		expect(closes).toBe(1);
		expect(palette.isSettled()).toBe(true);
	});
});

describe("palette resize + ascii + edge rendering (C2)", () => {
	function hinted(count: number) {
		return Array.from({ length: count }, (_, i) => ({
			id: `/cmd-${i}`,
			title: `/cmd-${i}`,
			hint: `Does thing ${i}`,
		}));
	}

	function budgetForTerminal(rows: number): number {
		return paletteListBudget(paletteOverlayRows(rows));
	}

	it("keeps selection stable across 100x45 → 80x10 → 60x8 → 24x4 → expand", () => {
		// Failure mode: the old min-8 overlay floor overstated tiny viewports
		// so the list budget overflowed and hid every action; single-tier
		// rendering hid the prompt or the selection at 24x4.
		setKeybindings(KeybindingsManager.inMemory());
		const palette = new CommandPaletteComponent();
		palette.setItems(hinted(25));
		palette.setMaxVisible(budgetForTerminal(45));
		for (let i = 0; i < 20; i++) palette.moveSelection(1);
		expect(palette.selectedItem()?.id).toBe("/cmd-20");
		for (const rows of [10, 8, 4]) {
			palette.setMaxVisible(budgetForTerminal(rows));
			expect(palette.visibleItems().map(item => item.id)).toContain("/cmd-20");
			const text = Bun.stripANSI(palette.render(rows === 4 ? 24 : rows === 8 ? 60 : 80).join("\n"));
			// Prompt row (query) and the selected action are always visible.
			expect(text).toContain("/cmd-20");
		}
		// Expand again: the selection survives and the full chrome returns.
		palette.setMaxVisible(budgetForTerminal(45));
		expect(palette.selectedItem()?.id).toBe("/cmd-20");
		const expanded = Bun.stripANSI(palette.render(100).join("\n"));
		expect(expanded).toContain("/cmd-20");
		expect(expanded).toContain("Does thing 20");
	});

	it("renders title/input only at 24x4 without overflow or missing selection", () => {
		setKeybindings(KeybindingsManager.inMemory());
		const palette = new CommandPaletteComponent();
		palette.setItems(hinted(10));
		palette.setMaxVisible(budgetForTerminal(4));
		const rendered = palette.render(24);
		// Ultra-compact: prompt row + at most maxListRows titles, no borders.
		expect(rendered.length).toBeLessThanOrEqual(1 + budgetForTerminal(4));
		const text = Bun.stripANSI(rendered.join("\n"));
		expect(text).toContain("/cmd-0");
		for (const row of rendered) {
			expect(Bun.stripANSI(row).length).toBeLessThanOrEqual(24);
		}
	});

	it("renders empty search, long descriptions, and paste commits truthfully", () => {
		setKeybindings(KeybindingsManager.inMemory());
		const palette = new CommandPaletteComponent();
		palette.setItems(hinted(5));
		palette.setQuery("zzz-no-match");
		palette.setMaxVisible(budgetForTerminal(24));
		expect(Bun.stripANSI(palette.render(80).join("\n"))).toContain("No matching commands");

		const long = new CommandPaletteComponent();
		const desc = `Does the thing with ${"very ".repeat(40)}long detail`;
		long.setItems([{ id: "/deploy", title: "/deploy", hint: desc, intent: "draft" }]);
		long.setMaxVisible(budgetForTerminal(24));
		for (const row of long.render(60)) {
			expect(Bun.stripANSI(row).length).toBeLessThanOrEqual(60);
		}
		expect(Bun.stripANSI(long.render(60).join("\n"))).toContain("/deploy");

		const paste = new CommandPaletteComponent();
		paste.setItems(hinted(5));
		paste.handleInput("/cmd-1");
		expect(paste.query).toBe("/cmd-1");
		expect(paste.selectedItem()?.id).toBe("/cmd-1");
	});

	it("uses the central cursor symbol — ASCII preset renders > with no unicode cursor", async () => {
		setKeybindings(KeybindingsManager.inMemory());
		await setSymbolPreset("ascii");
		try {
			expect(theme.nav.cursor).toBe(">");
			const palette = new CommandPaletteComponent();
			palette.setItems(hinted(3));
			palette.setMaxVisible(budgetForTerminal(24));
			const text = palette.render(80).join("\n");
			expect(text).not.toContain("❯");
			expect(text).not.toContain("›");
			expect(Bun.stripANSI(text)).toContain("> /cmd-0");
		} finally {
			await initTheme(false);
		}
	});
});

describe("palette budget sequences", () => {
	function hinted(count: number) {
		return Array.from({ length: count }, (_, i) => ({
			id: `/cmd-${i}`,
			title: `/cmd-${i}`,
			hint: `Does thing ${i}`,
		}));
	}

	it("accounts chrome rows plus per-item description rows in the budget", () => {
		// Failure mode: a fixed page size or chrome-blind budget overflows a
		// small terminal, pushing the search prompt off-screen.
		expect(PALETTE_CHROME_ROWS).toBe(3);
		expect(paletteListBudget(14)).toBe(10);
		setKeybindings(KeybindingsManager.inMemory());
		const palette = new CommandPaletteComponent();
		palette.setItems(hinted(25));
		palette.setMaxVisible(paletteListBudget(14));
		for (let i = 0; i < 24; i++) palette.moveSelection(1);
		const rendered = palette.render(80);
		expect(rendered.length).toBeLessThanOrEqual(PALETTE_CHROME_ROWS + paletteListBudget(14));
		expect(Bun.stripANSI(rendered.join("\n"))).toContain(`${theme.nav.cursor} /cmd-24`);
	});

	it("costs an empty-description draft its argument-marker row", () => {
		// Failure mode: #hasHintRow missed the draft marker when the
		// description was an empty string, under-budgeting by one row.
		setKeybindings(KeybindingsManager.inMemory());
		const palette = new CommandPaletteComponent();
		palette.setItems([
			{ id: "/a", title: "/a", hint: "", intent: "draft" },
			{ id: "/b", title: "/b", hint: "", intent: "draft" },
			{ id: "/c", title: "/c", hint: "", intent: "draft" },
		]);
		palette.setMaxVisible(4);
		palette.render(80);
		// Three two-row items cannot all fit in a 4-row budget.
		expect(palette.visibleItems().length).toBeLessThan(3);
	});

	it("keeps the selection visible across open and resize re-budgets", () => {
		setKeybindings(KeybindingsManager.inMemory());
		const palette = new CommandPaletteComponent();
		palette.setItems(hinted(25));
		// Open at a 24-row terminal (60% overlay share, like production).
		palette.setMaxVisible(paletteListBudget(14));
		for (let i = 0; i < 20; i++) palette.moveSelection(1);
		expect(palette.selectedItem()?.id).toBe("/cmd-20");
		// Resize down hard: the window re-centers on the selection.
		palette.setMaxVisible(4);
		expect(palette.visibleItems().map(item => item.id)).toContain("/cmd-20");
		// Filter resets to the first match and still renders it.
		palette.setQuery("/cmd-2");
		expect(palette.selectedItem()?.id).toBe("/cmd-2");
		expect(Bun.stripANSI(palette.render(80).join("\n"))).toContain("/cmd-2");
	});
});
