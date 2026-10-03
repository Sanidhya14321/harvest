import { afterEach, beforeEach, describe, expect, it } from "bun:test";
import { KeybindingsManager, setKeyHintPlatform } from "@harvest/pi-coding-agent/config/keybindings";
import { getKeybindings, setKeybindings, type KeybindingsManager as TuiKeybindingsManager } from "@harvest/pi-tui";
import { initThemeSync, theme } from "@harvest/pi-coding-agent/modes/theme/theme";
import { CommandPaletteComponent, paletteSlashText } from "@harvest/pi-coding-agent/modes/components/command-palette";
import { WorkspaceSidebar } from "@harvest/pi-coding-agent/modes/components/workspace-sidebar";
import { clampDialogWidth } from "@harvest/pi-coding-agent/modes/components/overlay-box";
import { renderInlineActivity, renderOutputBlock } from "@harvest/pi-coding-agent/tui/output-block";

describe("workspace sidebar", () => {
	it("omits empty optional sections and stays truthful about unknown usage", () => {
		initThemeSync();
		const sidebar = new WorkspaceSidebar();
		sidebar.setSnapshot({
			title: "session",
			contextKnown: false,
			contextPercent: null,
			contextTokens: null,
			contextWindow: null,
			costKnown: false,
			costLabel: null,
			mcpServers: [],
			lspServers: [],
			todos: [],
			agents: [],
			changes: [],
			changesTruncated: false,
			changesState: "loading",
			extensions: [],
			version: "0.0.0",
		});
		const text = Bun.stripANSI(sidebar.render(42).join("\n"));
		expect(text).toContain("Context unknown");
		expect(text).toContain("Cost unknown");
		expect(text).not.toContain("0%");
		expect(text).not.toContain("MCP");
	});

	it("labels repository changes without claiming session authorship", () => {
		initThemeSync();
		const sidebar = new WorkspaceSidebar();
		sidebar.setSnapshot({
			title: "t",
			contextKnown: true,
			contextPercent: 12,
			contextTokens: 1200,
			contextWindow: 10000,
			costKnown: true,
			costLabel: "$0.01",
			mcpServers: [{ name: "fs", connected: true }],
			lspServers: [],
			todos: [{ label: "write test", done: false }],
			agents: [],
			changes: [{ path: "src/a.ts", staged: false, untracked: false, added: 1, removed: 0 }],
			changesTruncated: false,
			changesState: "ready",
			extensions: [],
			version: "",
		});
		const text = Bun.stripANSI(sidebar.render(42).join("\n"));
		expect(text).toContain("Workspace Changes");
		expect(text).toContain("Context 12%");
		expect(text).not.toMatch(/made by this session/i);
	});
});

describe("command palette execution", () => {
	let previous: TuiKeybindingsManager;
	beforeEach(() => {
		previous = getKeybindings();
		setKeybindings(KeybindingsManager.inMemory());
		setKeyHintPlatform("linux");
	});
	afterEach(() => {
		setKeybindings(previous);
		setKeyHintPlatform(undefined);
	});

	function openPalette(): CommandPaletteComponent {
		initThemeSync();
		const palette = new CommandPaletteComponent();
		palette.setItems([
			{ id: "/commands", title: "/commands", hint: "Open command palette" },
			{ id: "/sidebar", title: "/sidebar", hint: "Toggle session sidebar" },
		]);
		return palette;
	}

	it("routes arrow-key selection through onSelect with registry slash text", () => {
		const palette = openPalette();
		const selected: string[] = [];
		palette.onSelect = item => {
			selected.push(paletteSlashText(item));
		};
		palette.handleInput("\x1b[B");
		expect(palette.selectedItem()?.id).toBe("/sidebar");
		palette.handleInput("\r");
		expect(selected).toEqual(["/sidebar"]);
		expect(palette.isSettled()).toBe(true);
		palette.handleInput("\r");
		expect(selected).toEqual(["/sidebar"]);
	});

	it("closes on Escape exactly once across repeat presses", () => {
		const palette = openPalette();
		const reasons: string[] = [];
		palette.onClose = reason => {
			reasons.push(reason);
		};
		palette.handleInput("\x1b");
		palette.handleInput("\x1b");
		expect(reasons).toEqual(["escape"]);
		expect(palette.isSettled()).toBe(true);
	});

	it("types to filter, backspaces to edit, and never steals editor chords", () => {
		const palette = openPalette();
		for (const ch of "side") palette.handleInput(ch);
		expect(palette.query).toBe("side");
		expect(palette.filtered().map(i => i.id)).toEqual(["/sidebar"]);
		palette.handleInput("\x7f");
		expect(palette.query).toBe("sid");
		// Ctrl+P (session toggles) and Alt+B (editor word-left) must pass
		// through without touching the query or the highlight.
		palette.handleInput("\x10");
		palette.handleInput("\x1bb");
		expect(palette.query).toBe("sid");
		expect(palette.selectedItem()?.id).toBe("/sidebar");
	});

	it("derives hints from the active keybindings, including remaps", () => {
		const palette = openPalette();
		palette.setKeybindings(KeybindingsManager.inMemory());
		expect(Bun.stripANSI(palette.render(80).join("\n"))).toContain("Alt+K");

		palette.setKeybindings(KeybindingsManager.inMemory({ "app.commands.open": "ctrl+k" }));
		const remapped = Bun.stripANSI(palette.render(80).join("\n"));
		expect(remapped).toContain("Ctrl+K");
		expect(remapped).not.toContain("Alt+K");
	});
});
describe("command palette", () => {
	it("filters registry items and keeps selection stable", () => {
		initThemeSync();
		const palette = new CommandPaletteComponent();
		palette.setItems([
			{ id: "/commands", title: "/commands", hint: "Open command palette" },
			{ id: "/sidebar", title: "/sidebar", hint: "Toggle session sidebar" },
		]);
		palette.setQuery("side");
		expect(palette.filtered().map(i => i.id)).toEqual(["/sidebar"]);
		expect(palette.selectedItem()?.id).toBe("/sidebar");
		const rows = palette.render(80);
		expect(rows.length).toBeGreaterThan(2);
		expect(Bun.stripANSI(rows.join("\n"))).toContain("Commands");
	});
});

describe("output block rail variant", () => {
	it("renders quiet rail activity without a rounded frame per operation", () => {
		initThemeSync();
		const railed = renderOutputBlock(
			{ header: "Read src/a.ts", state: "success", sections: [{ lines: ["line1"] }], width: 40, variant: "rail" },
			theme,
		).join("\n");
		const stripped = Bun.stripANSI(railed);
		expect(stripped).toContain("Read");
		expect(stripped).not.toContain("╭─");

		const inline = renderInlineActivity(theme, "Read", "src/a.ts", "success", 40).join("\n");
		expect(Bun.stripANSI(inline)).toContain("Read");
		expect(Bun.stripANSI(inline)).toContain("src/a.ts");
	});

	it("clamps dialogs to shared caps", () => {
		expect(clampDialogWidth(200, 116)).toBe(116);
		expect(clampDialogWidth(70, 88)).toBe(68);
	});
});
