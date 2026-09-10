import { describe, expect, it } from "bun:test";
import { SYMBOL_PRESETS } from "@harvest/pi-coding-agent/modes/theme/symbols";
import { getThemeByName } from "@harvest/pi-coding-agent/modes/theme/theme";

/** Emojis with default emoji presentation (colored pictographs that cause terminal width and tinting defects). */
const EMOJI_PRESENTATION_REGEX = /\p{Emoji_Presentation}/u;

describe("UI emoji-free icon contract", () => {
	it("ensures UNICODE_SYMBOLS preset has no default emoji presentation characters", () => {
		const unicodeSymbols = SYMBOL_PRESETS.unicode;
		const emojiViolations: Array<{ key: string; value: string }> = [];

		for (const [key, value] of Object.entries(unicodeSymbols)) {
			if (EMOJI_PRESENTATION_REGEX.test(value)) {
				emojiViolations.push({ key, value });
			}
		}

		expect(emojiViolations).toEqual([]);
	});

	it("ensures language icons in unicode preset are monochrome text symbols that support ANSI tinting", async () => {
		const theme = await getThemeByName("dark");
		expect(theme).toBeDefined();

		const languages = [
			"typescript",
			"javascript",
			"python",
			"rust",
			"go",
			"java",
			"c",
			"cpp",
			"csharp",
			"ruby",
			"julia",
			"php",
			"swift",
			"kotlin",
			"shell",
			"html",
			"css",
			"json",
			"yaml",
			"markdown",
			"sql",
			"docker",
			"lua",
		];

		for (const lang of languages) {
			const icon = theme!.getLangIcon(lang);
			expect(icon.length).toBeGreaterThan(0);
			expect(EMOJI_PRESENTATION_REGEX.test(icon)).toBe(false);

			const styled = theme!.getLangIconStyled(lang);
			expect(styled.length).toBeGreaterThan(0);
			expect(EMOJI_PRESENTATION_REGEX.test(styled)).toBe(false);
		}
	});

	it("ensures all tool identity symbols are emoji-free icons", () => {
		const unicodeSymbols = SYMBOL_PRESETS.unicode;
		const toolKeys = Object.keys(unicodeSymbols).filter(k => k.startsWith("tool."));

		for (const key of toolKeys) {
			const symbol = unicodeSymbols[key as keyof typeof unicodeSymbols];
			expect(EMOJI_PRESENTATION_REGEX.test(symbol)).toBe(false);
		}
	});

	it("ensures all settings tab symbols are emoji-free icons", () => {
		const unicodeSymbols = SYMBOL_PRESETS.unicode;
		const tabKeys = Object.keys(unicodeSymbols).filter(k => k.startsWith("tab."));

		for (const key of tabKeys) {
			const symbol = unicodeSymbols[key as keyof typeof unicodeSymbols];
			expect(EMOJI_PRESENTATION_REGEX.test(symbol)).toBe(false);
		}
	});

	it("ensures composer attachment chip symbols are emoji-free icons", () => {
		const unicodeSymbols = SYMBOL_PRESETS.unicode;
		const chipKeys = Object.keys(unicodeSymbols).filter(k => k.startsWith("chip."));

		for (const key of chipKeys) {
			const symbol = unicodeSymbols[key as keyof typeof unicodeSymbols];
			expect(EMOJI_PRESENTATION_REGEX.test(symbol)).toBe(false);
		}
	});
});
