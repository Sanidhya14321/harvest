import { describe, expect, it } from "bun:test";
import { createTheme } from "@harvest/pi-coding-agent/modes/theme/loader";
import { getBuiltinThemes } from "@harvest/pi-coding-agent/modes/theme/loader";
import { getBuiltinThemes as getBuiltins } from "@harvest/pi-coding-agent/modes/theme/loader";

describe("theme surface tokens", () => {
	it("loads legacy themes through central surface fallbacks", () => {
		const theme = createTheme(getBuiltins().dark as never, { mode: "truecolor" });
		for (const bg of ["screenBg", "panelBg", "raisedBg", "composerBg", "modalBg"] as const) {
			expect(() => theme.getBgHex(bg)).not.toThrow();
			expect(() => theme.getBgAnsi(bg)).not.toThrow();
		}
	});

	it("provides harvest palettes with restrained OpenCode-like values", () => {
		const builtins = getBuiltinThemes();
		expect(builtins.harvest).toBeDefined();
		expect(builtins["harvest-light"]).toBeDefined();
		const dark = createTheme(builtins.harvest as never, { mode: "truecolor" });
		expect(dark.getBgHex("screenBg").toLowerCase()).toBe("#0a0a0a");
		expect(dark.getBgHex("panelBg").toLowerCase()).toBe("#141414");
		expect(dark.getBgHex("composerBg").toLowerCase()).toBe("#1e1e1e");
		const light = createTheme(builtins["harvest-light"] as never, { mode: "truecolor" });
		expect(light.isLight).toBe(true);
		expect(dark.isLight).toBe(false);
	});

	it("keeps background reset handling stable for padded surfaces", () => {
		const builtins = getBuiltinThemes();
		const theme = createTheme(builtins.harvest as never, { mode: "truecolor" });
		const filled = theme.bgFill("panelBg", "a\x1b[0mb");
		expect(filled).toContain("\x1b[");
		expect(theme.fgOnBg("text", "panelBg", "hi")).toContain("hi");
	});
});
