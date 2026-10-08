/**
 * Standalone TUI model picker used by `omp setup speech`.
 *
 * Mirrors {@link ./session-picker.ts} for the standalone-TUI lifecycle: spin up
 * a one-shot fullscreen {@link TUI} over a {@link SelectList}, resolve on
 * select/cancel, and restore the previous screen. Titles and controls share
 * the viewport allocation and the TUI automatically renders on input.
 */
import { Input, ProcessTerminal, type SelectItem, SelectList, type Terminal, TUI } from "@harvest/pi-tui";
import { editorKey } from "../modes/components/keybinding-hints";
import { InteractiveDialogPanel } from "../modes/components/overlay-box";
import { getSelectListTheme, theme } from "../modes/theme/theme";

export interface StandalonePickerOptions {
	/** An embedded host can supply its terminal without taking over process stdio. */
	terminal?: Terminal;
}

/** Shared control-first surface for one-shot CLI selectors and text prompts. */
export class StandalonePickerDialog extends InteractiveDialogPanel {
	constructor(
		title: string,
		private readonly control: SelectList | Input,
	) {
		super(title);
		this.addChild(control);
	}

	protected override dialogFooter(): string {
		return [
			`${editorKey("tui.select.confirm") || "Enter"} ${this.control instanceof Input ? "submit" : "select"}`,
			`${editorKey("tui.select.cancel") || "Esc"} cancel`,
			"F2 details",
		].join(theme.sep.dot);
	}

	handleInput(data: string): void {
		if (!this.handleDialogInput(data)) this.control.handleInput(data);
	}

	override invalidate(): void {
		super.invalidate();
		this.control.invalidate();
	}
}

/** Mount only through the TUI so title, controls and cleanup share one lifetime. */
export async function runStandalonePicker(
	createDialog: (finish: (value: string | null) => void) => StandalonePickerDialog,
	options: StandalonePickerOptions = {},
): Promise<string | null> {
	const { promise, resolve } = Promise.withResolvers<string | null>();
	const ui = new TUI(options.terminal ?? new ProcessTerminal());
	let resolved = false;
	const finish = (value: string | null): void => {
		if (resolved) return;
		resolved = true;
		ui.stop();
		resolve(value);
	};
	const dialog = createDialog(finish);
	ui.showOverlay(dialog, {
		anchor: "top-left",
		width: "100%",
		maxHeight: "100%",
		margin: 0,
		fullscreen: true,
	});
	ui.setFocus(dialog);
	ui.start();
	return promise;
}

/**
 * Show a single-column model picker and resolve with the chosen item's value,
 * or `null` if the user cancelled. `currentValue` pre-selects the matching row.
 */
export async function selectSetupModel(
	title: string,
	items: SelectItem[],
	currentValue: string,
	options: StandalonePickerOptions = {},
): Promise<string | null> {
	return runStandalonePicker(finish => {
		const list = new SelectList(items, 1, getSelectListTheme());
		const currentIndex = items.findIndex(item => item.value === currentValue);
		if (currentIndex >= 0) list.setSelectedIndex(currentIndex);
		list.onSelect = item => finish(item.value);
		list.onCancel = () => finish(null);
		return new StandalonePickerDialog(title, list);
	}, options);
}
