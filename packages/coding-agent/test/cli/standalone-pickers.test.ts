import { afterEach, beforeEach, describe, expect, it, spyOn } from "bun:test";
import { Input, type SelectItem, SelectList, visibleWidth } from "@harvest/pi-tui";
import { stripVTControlCharacters } from "node:util";
import { VirtualTerminal } from "../../../tui/test/virtual-terminal";
import { pickCleanseTarget, promptCleanseRequest } from "../../src/cli/cleanse-picker";
import { selectSetupModel, StandalonePickerDialog } from "../../src/cli/setup-model-picker";
import { createTheme, getBuiltinThemes } from "../../src/modes/theme/loader";
import { getSelectListTheme, setThemeInstance, type Theme, theme } from "../../src/modes/theme/theme";

describe("standalone CLI pickers", () => {
	let previousTheme: Theme;
	beforeEach(() => {
		previousTheme = theme;
		setThemeInstance(createTheme(getBuiltinThemes().harvest, { mode: "none", symbolPresetOverride: "ascii" }));
	});
	afterEach(() => setThemeInstance(previousTheme));

	it("keeps a preselected model visible through short-screen resizing and returns its full value", async () => {
		const terminal = new VirtualTerminal(80, 16);
		const stop = spyOn(terminal, "stop");
		const fullValue = "provider/model-with-a-long-authoritative-identifier";
		const items: SelectItem[] = Array.from({ length: 16 }, (_, index) => ({
			value: index === 15 ? fullValue : `model-${index}`,
			label: index === 15 ? "Selected final model" : `Model ${index}`,
		}));
		const result = selectSetupModel("Speech model", items, fullValue, { terminal });
		try {
			await terminal.waitForRender(() => terminal.getViewport().join("\n").includes("Selected final model"));
			terminal.resize(24, 4);
			await terminal.waitForRender(() => terminal.getViewport().join("\n").includes("Selected final"));
			expect(terminal.getViewport().join("\n")).toContain("Selected final");
			expect(terminal.getViewport().every(line => visibleWidth(line) <= 24)).toBe(true);
			terminal.resize(80, 16);
			await terminal.waitForRender(() => terminal.getViewport().join("\n").includes("Selected final model"));
			terminal.sendInput("\r");
			expect(await result).toBe(fullValue);
			expect(stop).toHaveBeenCalledTimes(1);
		} finally {
			terminal.sendInput("\x1b");
			await result;
			stop.mockRestore();
		}
	});

	it("ignores clicks on short-screen chrome and maps the visible choice to its unchanged payload", async () => {
		const terminal = new VirtualTerminal(24, 4);
		const fullValue = "provider/full-model-value";
		const result = selectSetupModel(
			"Select",
			[
				{ value: "first", label: "First" },
				{ value: fullValue, label: "Second" },
			],
			fullValue,
			{ terminal },
		);
		let completed = false;
		void result.then(() => {
			completed = true;
		});
		try {
			await terminal.waitForRender(() => terminal.getViewport().join("\n").includes("Second"));
			terminal.sendInput("\x1b[<0;3;4M");
			await Bun.sleep(10);
			expect(completed).toBe(false);
			terminal.sendInput("\x1b[<0;3;2M");
			expect(await result).toBe(fullValue);
		} finally {
			terminal.sendInput("\x1b");
			await result;
		}
	});

	it("keeps a one-cell prompt editable and preserves text after expanding its allocation", () => {
		const input = new Input();
		const dialog = new StandalonePickerDialog("A long prompt", input);
		let submitted: string | undefined;
		input.onSubmit = value => {
			submitted = value;
		};
		dialog.focused = true;
		dialog.setMaxHeight(1);
		const fullRequest = "repair all TypeScript diagnostics without changing valid behavior";
		dialog.handleInput(fullRequest);
		const tiny = dialog.render(1);
		expect(tiny).toHaveLength(1);
		expect(visibleWidth(tiny[0]!)).toBe(1);
		dialog.setMaxHeight(4);
		expect(stripVTControlCharacters(dialog.render(80).join("\n"))).toContain(fullRequest);
		dialog.handleInput("\r");
		expect(submitted).toBe(fullRequest);
	});

	it("renders overflow, footer and details using the ASCII preset without losing cancel", async () => {
		const items = Array.from({ length: 12 }, (_, index) => ({ value: `${index}`, label: `Choice ${index}` }));
		const list = new SelectList(items, 1, getSelectListTheme());
		const dialog = new StandalonePickerDialog("Choose", list);
		let cancelled = false;
		list.onCancel = () => {
			cancelled = true;
		};
		dialog.setMaxHeight(4);
		dialog.render(24);
		dialog.handleInput("\x1bOQ");
		const details = stripVTControlCharacters(dialog.render(24).join("\n"));
		expect(details).not.toMatch(/[^\x00-\x7f]/);
		dialog.handleInput("\x1b");
		expect(cancelled).toBe(true);
	});

	it("retains the checker identity when its display command is truncated", async () => {
		const terminal = new VirtualTerminal(24, 4);
		const checkerId = "typescript/workspace/check-with-full-id";
		const result = pickCleanseTarget(
			[
				{
					id: checkerId,
					label: "TypeScript check",
					language: "TypeScript",
					cwd: "packages/coding-agent",
					command: "bun --cwd=packages/coding-agent run check:types",
				},
			],
			{ terminal },
		);
		try {
			await terminal.waitForRender();
			terminal.sendInput("\x1b[B");
			await terminal.waitForRender(() => terminal.getViewport().join("\n").includes("TypeScript chec"));
			expect(terminal.getViewport().join("\n")).toContain("TypeScript chec");
			terminal.sendInput("\r");
			expect(await result).toEqual({ kind: "checker", id: checkerId });
		} finally {
			terminal.sendInput("\x1b");
			await result;
		}
	});

	it("starts the request prompt after the choice closes and returns the full edited request", async () => {
		const terminal = new VirtualTerminal(80, 12);
		const stop = spyOn(terminal, "stop");
		const result = pickCleanseTarget([], { terminal });
		try {
			await terminal.waitForRender();
			terminal.sendInput("\x1b[B");
			terminal.sendInput("\r");
			await terminal.waitForRender(() => terminal.getViewport().join("\n").includes("Describe what to detect"));
			terminal.resize(3, 1);
			await terminal.waitForRender();
			const request = "  resolve all TypeScript errors in this workspace  ";
			terminal.sendInput(`\x1b[200~${request}\x1b[201~`);
			terminal.sendInput("\r");
			expect(await result).toEqual({ kind: "request", request: request.trim() });
			expect(stop).toHaveBeenCalledTimes(2);
		} finally {
			terminal.sendInput("\x1b");
			await result;
			stop.mockRestore();
		}
	});

	it("cancels a request without leaking a running terminal input handler", async () => {
		const terminal = new VirtualTerminal(24, 4);
		const stop = spyOn(terminal, "stop");
		const result = promptCleanseRequest({ terminal });
		try {
			await terminal.waitForRender();
			terminal.sendInput("partial request");
			terminal.sendInput("\x1b");
			expect(await result).toBeNull();
			terminal.sendInput("\r");
			expect(stop).toHaveBeenCalledTimes(1);
		} finally {
			terminal.sendInput("\x1b");
			await result;
			stop.mockRestore();
		}
	});
});
