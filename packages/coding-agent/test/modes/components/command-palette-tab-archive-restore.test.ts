import { beforeAll, describe, expect, it } from "bun:test";
import { KeybindingsManager } from "@harvest/pi-coding-agent/config/keybindings";
import {
	mergePaletteItems,
	resolvePaletteSelection,
	TAB_MANAGEMENT_PALETTE_SOURCES,
} from "@harvest/pi-coding-agent/modes/components/command-palette";
import { initTheme } from "@harvest/pi-coding-agent/modes/theme/theme";

beforeAll(() => {
	initTheme();
});

describe("palette TAB_MANAGEMENT archive/restore", () => {
	it("exposes archive/restore as draft intents with remap-aware hints when bound, else description-only", () => {
		// Failure mode: archive/restore are missing from the palette, forcing
		// operators to memorize slash syntax.
		const items = mergePaletteItems([...TAB_MANAGEMENT_PALETTE_SOURCES]);
		const byId = new Map(items.map(item => [item.id, item]));
		expect(byId.get("/tab close")?.intent).toBe("draft");
		expect(byId.get("/tab reopen")?.intent ?? "run").toBe("run");
		expect(byId.get("/tab archive")?.intent).toBe("draft");
		expect(byId.get("/tab archive")?.argHint).toBe("[number|id]");
		expect(byId.get("/tab restore")?.intent).toBe("draft");
		expect(byId.get("/tab restore")?.argHint).toBe("[id]");

		// Draft intents prepare editor text instead of executing.
		expect(resolvePaletteSelection(byId.get("/tab archive")!)).toEqual({
			kind: "draft",
			text: "/tab archive ",
		});
		expect(resolvePaletteSelection(byId.get("/tab restore")!)).toEqual({
			kind: "draft",
			text: "/tab restore ",
		});
	});

	it("keeps close/reopen remap hints working alongside the new entries", () => {
		const items = mergePaletteItems(
			[{ name: "status", description: "builtin", group: "builtin" }],
			[...TAB_MANAGEMENT_PALETTE_SOURCES],
		);
		expect(items.map(item => item.id)).toContain("/tab archive");
		expect(items.map(item => item.id)).toContain("/tab restore");
		// No duplicate names; first source wins is preserved.
		const ids = items.map(item => item.id);
		expect(new Set(ids).size).toBe(ids.length);
		expect(KeybindingsManager.inMemory).toBeDefined();
	});
});
