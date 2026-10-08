/**
 * Standalone TUI pickers for `omp cleanse`.
 *
 * Mirrors {@link ./setup-model-picker.ts}: one-shot {@link TUI} instances over a
 * {@link SelectList} or {@link Input}, resolved on select/submit/cancel and torn
 * down immediately so the command can keep writing plain stdout afterwards.
 */
import { Input, type SelectItem, SelectList } from "@harvest/pi-tui";
import type { CleanseCheckerDescriptor } from "../cleanse/checkers";
import type { CleanseTargetChoice } from "../cleanse/types";
import { getSelectListTheme, theme } from "../modes/theme/theme";
import { runStandalonePicker, StandalonePickerDialog, type StandalonePickerOptions } from "./setup-model-picker";

/** Pick between running every discovered checker, one specific checker, or a free-form request. */
export async function pickCleanseTarget(
	checkers: readonly CleanseCheckerDescriptor[],
	options: StandalonePickerOptions = {},
): Promise<CleanseTargetChoice> {
	const items: SelectItem[] = [
		{
			value: "all",
			label: `Run all ${checkers.length} discovered checker${checkers.length === 1 ? "" : "s"}`,
		},
		...checkers.map(checker => ({
			value: `checker:${checker.id}`,
			label: checker.label,
			description: `${checker.language}${theme.sep.dot}${checker.command}`,
		})),
		{
			value: "request",
			label: `Describe what to fix${theme.symbol("sep.ellipsis")}`,
			description: "A discovery agent figures out the command to run",
		},
	];
	const selection = await selectOne("Select what to cleanse:", items, options);
	if (selection === null) return { kind: "cancel" };
	if (selection === "all") return { kind: "all" };
	if (selection === "request") {
		const request = await promptCleanseRequest(options);
		return request === null ? { kind: "cancel" } : { kind: "request", request };
	}
	return { kind: "checker", id: selection.slice("checker:".length) };
}

/** One-shot text prompt for a free-form cleanse request; `null` when cancelled or left empty. */
export async function promptCleanseRequest(options: StandalonePickerOptions = {}): Promise<string | null> {
	return runStandalonePicker(finish => {
		const input = new Input();
		input.onSubmit = value => finish(value.trim() || null);
		input.onEscape = () => finish(null);
		return new StandalonePickerDialog('Describe what to detect and fix (e.g. "ts errors"):', input);
	}, options);
}

async function selectOne(title: string, items: SelectItem[], options: StandalonePickerOptions): Promise<string | null> {
	return runStandalonePicker(finish => {
		const list = new SelectList(items, 1, getSelectListTheme());
		list.onSelect = item => finish(item.value);
		list.onCancel = () => finish(null);
		return new StandalonePickerDialog(title, list);
	}, options);
}
