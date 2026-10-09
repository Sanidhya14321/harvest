import { afterEach, beforeEach, expect, test } from "bun:test";
import type { ModelRegistry } from "../../../src/config/model-registry";
import { DebugSelectorComponent } from "../../../src/debug";
import { Settings } from "../../../src/config/settings";
import { AdvisorConfigOverlayComponent } from "../../../src/modes/components/advisor-config";
import { BorderedLoader } from "../../../src/modes/components/bordered-loader";
import { BtwPanelComponent } from "../../../src/modes/components/btw-panel";
import { CleansePanelComponent } from "../../../src/modes/components/cleanse-panel";
import { OmfgPanelComponent } from "../../../src/modes/components/omfg-panel";
import { SnapcompactShapePreview } from "../../../src/modes/components/snapcompact-shape-preview";
import { TinyTitleDownloadProgressComponent } from "../../../src/modes/components/tiny-title-download-progress";
import { DEFAULT_TINY_TITLE_LOCAL_MODEL_KEY } from "../../../src/tiny/models";
import { createTheme, getBuiltinThemes } from "../../../src/modes/theme/loader";
import { setThemeInstance, theme } from "../../../src/modes/theme/theme";
import type { InteractiveModeContext } from "../../../src/modes/types";
import { renderSetupSplash, renderStarfield } from "../../../src/modes/setup-wizard/scenes/splash";
import { type Component, type TUI, visibleWidth } from "@harvest/pi-tui";

let previousTheme = theme;
const host = { requestRender() {}, requestComponentRender() {} } as unknown as TUI;
beforeEach(() => {
	previousTheme = theme;
	setThemeInstance(createTheme(getBuiltinThemes().harvest!, { mode: "none", symbolPresetOverride: "ascii" }));
});
afterEach(() => {
	setThemeInstance(previousTheme);
});

function checkFrame(component: Component, width: number, height: number): string {
	component.setMaxHeight?.(height);
	const lines = component.render(width);
	expect(lines.length).toBeLessThanOrEqual(height);
	for (const line of lines) {
		expect(visibleWidth(line)).toBeLessThanOrEqual(width);
		expect(line).not.toMatch(/\x1b\[[\d;]*m/);
		expect(Bun.stripANSI(line)).not.toMatch(/[^\x00-\x7f]/);
	}
	return Bun.stripANSI(lines.join("\n"));
}

test("debug choices remain reachable after shrinking a selector with decorative children", () => {
	let cancelled = false;
	const panel = new DebugSelectorComponent({} as InteractiveModeContext, () => {
		cancelled = true;
	});
	for (let index = 0; index < 12; index++) panel.handleInput("\x1b[B");
	for (const height of [1, 2, 4]) {
		const text = checkFrame(panel, 40, height);
		expect(text).toContain("artifact cache");
		if (height >= 2) expect(text).toMatch(/Esc.*cancel/);
	}
	panel.handleInput("\x1b");
	expect(cancelled).toBe(true);
});

test("setup artwork respects ASCII and no-color policy across tiny and full-screen allocations", () => {
	for (const [width, height] of [
		[1, 1],
		[4, 2],
		[40, 8],
		[100, 30],
	]) {
		for (const lines of [renderSetupSplash(width, height, 100), renderStarfield(width, height, 10)]) {
			expect(lines.length).toBe(height);
			for (const line of lines) {
				expect(visibleWidth(line)).toBeLessThanOrEqual(width);
				expect(line).not.toMatch(/\x1b\[[\d;]*m/);
				expect(line).not.toMatch(/[^\x00-\x7f]/);
			}
		}
	}
});

test("completed side answers keep copy/branch controls at short heights and preserve the full copy payload", () => {
	const panel = new BtwPanelComponent({ tui: host, question: "Question" });
	const answer = `${"Full answer\n".repeat(20)}Final answer`;
	panel.setAnswer(answer);
	panel.markComplete();
	for (const height of [1, 2, 4]) {
		const text = checkFrame(panel, 40, height);
		expect(text).toContain("c copy");
		expect(text).toContain("b branch");
	}
	expect(panel.getCopyText()).toBe(answer);
	panel.markError("Provider\tfailed");
	expect(checkFrame(panel, 24, 4).replace(/\s+/g, " ")).toContain("Provider failed");
	expect(panel.getCopyText()).toBeUndefined();
});

test("rule confirmation and failures retain their control or cause before optional status rows", () => {
	const panel = new OmfgPanelComponent({ tui: host, complaint: "Problem" });
	panel.setRule("Rule content");
	panel.setStatus("confirming", "Check the proposed rule");
	expect(checkFrame(panel, 40, 2)).toContain("Rule content");
	panel.markError("Validation\tfailed");
	expect(checkFrame(panel, 24, 3).replace(/\s+/g, " ")).toContain("Validation failed");
	for (const width of [1, 2, 3, 4]) checkFrame(panel, width, 1);
});

test("a one-row loader exposes Escape and cancellation aborts its signal", () => {
	const loader = new BorderedLoader(host, theme, "Working on a long task");
	try {
		expect(checkFrame(loader, 24, 1)).toContain("Esc cancel");
		loader.handleInput("\x1b");
		expect(loader.signal.aborted).toBe(true);
	} finally {
		loader.dispose();
	}
});

test("cleanse live work keeps its cancel row and theme policy through checker completion and failure", () => {
	const panel = new CleansePanelComponent({ tui: host });
	try {
		for (let index = 0; index < 20; index++) panel.log(`Earlier diagnostic ${index}`);
		const checker = { id: "types", label: "Type checker", language: "ts", cwd: ".", command: "check" };
		panel.checkerStarted(checker);
		panel.agentStarted("Cleanse1", { index: 0, groups: [], weight: 1 });
		const live = checkFrame(panel, 80, 8);
		expect(live).toContain("Type checker");
		expect(live).toContain("Repairing");
		for (const height of [1, 2, 4]) expect(checkFrame(panel, 24, height)).toContain("Esc cancel");
		panel.repairFinished();
		panel.checkerFinished({ ...checker, diagnostics: [], exitCode: 0 }, 100);
		expect(checkFrame(panel, 80, 6)).toContain("Type checker clean");
		panel.markError("Provider\tfailed\n" + "Details ".repeat(100));
		const failed = checkFrame(panel, 24, 4);
		expect(failed.replace(/\s+/g, " ")).toContain("Provider failed");
		expect(failed).toContain("Esc dismiss");
	} finally {
		panel.dispose();
	}
});

test("tiny-model progress retains its status at one row and respects ASCII on resized surfaces", () => {
	const panel = new TinyTitleDownloadProgressComponent(DEFAULT_TINY_TITLE_LOCAL_MODEL_KEY);
	panel.update({
		modelKey: DEFAULT_TINY_TITLE_LOCAL_MODEL_KEY,
		status: "progress",
		progress: 50,
		loaded: 1024,
		total: 2048,
	});
	expect(checkFrame(panel, 24, 1)).toContain("Downloading");
	expect(checkFrame(panel, 40, 4)).toContain("50%");
	expect(panel.isComplete()).toBe(false);
	panel.update({ modelKey: DEFAULT_TINY_TITLE_LOCAL_MODEL_KEY, status: "error" });
	expect(checkFrame(panel, 24, 1)).toContain("Failed");
	expect(panel.isComplete()).toBe(true);
	for (const width of [1, 2, 3, 4]) checkFrame(panel, width, 1);
});

test("settled cleanse verdicts adopt new color and symbol policy without losing their outcome", () => {
	setThemeInstance(createTheme(getBuiltinThemes().harvest!, { mode: "truecolor", symbolPresetOverride: "unicode" }));
	const panel = new CleansePanelComponent({ tui: host });
	try {
		panel.checkerFinished(
			{
				id: "types",
				label: "Type checker",
				language: "ts",
				cwd: ".",
				command: "check",
				diagnostics: [],
				exitCode: 0,
			},
			100,
		);
		panel.agentFinished(
			{ name: "Cleanse1", success: false, output: "", error: "Repair failed" },
			{ index: 0, groups: [], weight: 1 },
		);
		panel.logError("Diagnostic failed");
		panel.finish("unresolved");
		expect(panel.render(80).join("\n")).toMatch(/\x1b\[[\d;]*m/);
		setThemeInstance(createTheme(getBuiltinThemes().harvest!, { mode: "none", symbolPresetOverride: "ascii" }));
		const text = checkFrame(panel, 80, 8);
		expect(text).toContain("Type checker clean");
		expect(text).toContain("Repair failed");
		expect(text).toContain("Diagnostic failed");
	} finally {
		panel.dispose();
	}
});

test("snapcompact textual fallback fits its allocated cells and restores statistics after resize", () => {
	const preview = new SnapcompactShapePreview("auto");
	try {
		const wide = checkFrame(preview, 100, 8);
		expect(wide).toContain("cells");
		expect(wide).toContain("tokens");
		expect(wide).toContain("Kitty-graphics");
		for (const width of [1, 2, 3, 4, 24]) for (const height of [1, 2, 4]) checkFrame(preview, width, height);
		expect(checkFrame(preview, 100, 8)).toBe(wide);
	} finally {
		preview.dispose();
	}
});

test("advisor configuration preserves selection and edits when switching to ASCII and shrinking its viewport", () => {
	setThemeInstance(createTheme(getBuiltinThemes().harvest!, { mode: "truecolor", symbolPresetOverride: "unicode" }));
	const doc = {
		advisors: [
			{ name: "Active", enabled: true },
			{ name: "Disabled", enabled: false },
		],
	};
	const panel = new AdvisorConfigOverlayComponent(
		host,
		{
			modelRegistry: {} as unknown as ModelRegistry,
			settings: Settings.isolated(),
			scopedModels: [],
			availableToolNames: [],
		},
		"project",
		doc,
		{
			loadDoc: async () => ({ advisors: [] }),
			save: async () => {},
			close() {},
			requestRender() {},
			notify() {},
		},
	);
	panel.setMaxHeight(8);
	panel.render(100);
	panel.handleInput("\x1b[B");
	setThemeInstance(createTheme(getBuiltinThemes().harvest!, { mode: "none", symbolPresetOverride: "ascii" }));
	expect(checkFrame(panel, 100, 8)).toContain("[ ] Disabled");
	panel.handleInput("\r");
	expect(checkFrame(panel, 100, 8)).toContain('Editing "Disabled"');
	panel.handleInput("\x1b[B");
	checkFrame(panel, 24, 4);
	panel.handleInput("\r");
	expect(doc.advisors[1]!.enabled).toBeUndefined();
	expect(checkFrame(panel, 100, 8)).toContain("[x] on");
	for (const width of [1, 2, 3, 4]) checkFrame(panel, width, 1);
});
