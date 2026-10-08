import * as path from "node:path";
import { describe, expect, it } from "bun:test";
import { bgAnsi, colorToAnsi, detectColorMode, fgAnsi } from "../../../src/modes/theme/color";
import { createTheme, getBuiltinThemes } from "../../../src/modes/theme/loader";
import type { ThemeBg, ThemeColor } from "../../../src/modes/theme/schema";
import { VirtualTerminal } from "../../../../tui/test/virtual-terminal";

describe("application visual foundation", () => {
	it("honors the shared disable and explicit-force precedence across foreground, background, and contrast paths", () => {
		const mode = detectColorMode({ NO_COLOR: "", WT_SESSION: "supported-terminal" });
		expect(mode).toBe("none");
		expect(detectColorMode({ TERM: "dumb" })).toBe("none");
		expect(detectColorMode({ FORCE_COLOR: "0", WT_SESSION: "supported-terminal" })).toBe("none");
		expect(detectColorMode({ NO_COLOR: "1", FORCE_COLOR: "3", TERM: "dumb" })).toBe("truecolor");
		expect(detectColorMode({ FORCE_COLOR: "2", WT_SESSION: "supported-terminal" })).toBe("256color");
		const activeTheme = createTheme(getBuiltinThemes().harvest, { mode });
		const output = activeTheme.bgFill(
			"composerBg",
			activeTheme.fgOnBg("text", "composerBg", activeTheme.bold("typed")),
		);
		const terminal = new VirtualTerminal(12, 2);
		terminal.write(output);
		expect(terminal.getViewport()[0].trimEnd()).toBe("typed");
		expect(terminal.getViewportRowForegroundColumns(0)).toEqual([]);
		expect(terminal.getViewportRowBackgroundColumns(0)).toEqual([]);
		expect(output).not.toContain("\x1b[");
		for (const paint of [fgAnsi, bgAnsi]) {
			expect(paint(42, mode)).toBe("");
			expect(paint("", mode)).toBe("");
		}
		expect(colorToAnsi("#aabbcc", mode)).toBe("");
		expect(activeTheme.fgResolved("text", "body")).toBe("body");
		expect(activeTheme.getFgAnsi("text")).toBe("");
		expect(activeTheme.getBgAnsi("panelBg")).toBe("");
		expect(activeTheme.getContrastFgAnsi("accent")).toBe("");
		// Mode controls generated styling, never authored control sequences.
		expect(activeTheme.bgFill("panelBg", "\x1b[31mraw")).toBe("\x1b[31mraw");
		expect(() => activeTheme.getFgAnsi("missing" as ThemeColor)).toThrow("Unknown theme color");
		expect(() => activeTheme.getBgAnsi("missing" as ThemeBg)).toThrow("Unknown theme background");
	});

	it("adopts composer surfaces in new and legacy light/dark palettes and disables generated Markdown colors", async () => {
		// Adapter themes are process-global; a child isolates live-theme changes
		// from other files in the package's parallel test suite.
		const child = Bun.spawn(
			[
				process.execPath,
				"--eval",
				`
import {createTheme,getBuiltinThemes} from './packages/coding-agent/src/modes/theme/loader.ts';
import {setThemeInstance,getEditorTheme,getMarkdownTheme} from './packages/coding-agent/src/modes/theme/theme.ts';
import {Editor,Markdown} from './packages/tui/src/index.ts';
const results=[];
for (const name of ['harvest','harvest-light','dark','light']) {
 const active=createTheme(getBuiltinThemes()[name],{mode:'truecolor',symbolPresetOverride:'ascii'});
 setThemeInstance(active);
 const editor=new Editor(getEditorTheme()); editor.setBorderStyle('rail'); editor.setText('typed');
 results.push({name,rows:editor.render(24),surface:active.bgFill('composerBg',' '.repeat(24))});
}
setThemeInstance(createTheme(getBuiltinThemes().harvest,{mode:'none',symbolPresetOverride:'ascii'}));
const source='Color #abcdef and \x60#123456\x60\\n\\n\x60\x60\x60typescript\\nconst value = 42;\\n\x60\x60\x60\\n\\n\x60\x60\x60mermaid\\ngraph TD; A-->B;\\n\x60\x60\x60';
const markdown=new Markdown(source,0,0,getMarkdownTheme());
const first=markdown.render(60); markdown.setText(source+'\\nMore #ff00ff');
process.stdout.write(JSON.stringify({results,markdown:first,appended:markdown.render(60)}));
`,
			],
			{
				cwd: path.resolve(import.meta.dir, "../../../../.."),
				stdout: "pipe",
				stderr: "pipe",
			},
		);
		const [code, stdout, stderr] = await Promise.all([
			child.exited,
			new Response(child.stdout).text(),
			new Response(child.stderr).text(),
		]);
		expect(code, stderr).toBe(0);
		const result = JSON.parse(stdout) as {
			results: Array<{ name: string; rows: string[]; surface: string }>;
			markdown: string[];
			appended: string[];
		};
		for (const palette of result.results) {
			const terminal = new VirtualTerminal(24, 3);
			terminal.write(palette.rows.join("\r\n"));
			expect(terminal.getViewport()[0].trimEnd(), palette.name).toBe("| typed|");
			expect(terminal.getViewportRowBackgroundColumns(0), palette.name).toEqual(
				Array.from({ length: 23 }, (_, index) => index + 1),
			);
			const surface = new VirtualTerminal(24, 2);
			surface.write(palette.surface);
			expect(terminal.getViewportCellRows()[0][8 + 4], palette.name).toBe(surface.getViewportCellRows()[0][4]);
		}
		for (const rows of [result.markdown, result.appended]) {
			const body = rows.join("\n");
			expect(body).toContain("#abcdef");
			expect(body).toContain("#123456");
			expect(body).toContain("const value = 42;");
			expect(body).toContain("A");
			expect(body).toContain("B");
			expect(body).not.toMatch(/\x1b\[[0-9;]*m/);
		}
		expect(result.appended.join("\n")).toContain("#ff00ff");
	}, 15000);
});
