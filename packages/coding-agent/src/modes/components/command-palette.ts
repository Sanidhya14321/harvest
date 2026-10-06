import type { Component } from "@harvest/pi-tui";
import { getKeybindings, matchesKey, replaceTabs, truncateToWidth, visibleWidth } from "@harvest/pi-tui";
import { formatKeyHints, type AppKeybinding, type KeybindingsManager } from "../../config/keybindings";
import { theme } from "../theme/theme";
import {
	matchesSelectDown,
	matchesSelectPageDown,
	matchesSelectPageUp,
	matchesSelectUp,
} from "../utils/keybinding-matchers";
import { clampDialogWidth } from "../workspace-layout";
import { bottomBorder, row, topBorder } from "./overlay-box";

export interface CommandPaletteItem {
	readonly id: string;
	readonly title: string;
	readonly hint?: string;
	readonly group?: string;
	/**
	 * Invocation intent: `run` executes through `executeBuiltinSlashCommand`;
	 * `draft` prepares `/name ` in the editor so the operator supplies
	 * arguments/subcommands first (never executes an incomplete command).
	 */
	readonly intent?: "run" | "draft";
	/** Static argument hint shown when the command takes arguments. */
	readonly argHint?: string;
}

/** Registry command projected into palette items (builtins, extensions, skills, files, templates). */
export interface PaletteCommandSource {
	readonly name: string;
	readonly description?: string;
	readonly group: string;
	readonly allowArgs?: boolean;
	readonly hasSubcommands?: boolean;
	readonly inlineHint?: string;
}

/**
 * Merge registry sources into palette items, first source winning on
 * duplicate names. Argument/subcommand metadata survives as invocation
 * intent: commands that take arguments prepare a draft instead of
 * executing immediately.
 *
 * Each item carries complete actionable metadata for the owner dispatch:
 * `id`/`title` (`/name`) is the handler key the owner routes through
 * `executeBuiltinSlashCommand` (falling back to production submission for
 * non-builtins), `group` is the command kind (builtin/extension/skill/file/
 * template/session), and `intent` + `argHint` carry the argument metadata
 * (draft-vs-run intent plus the static argument hint). Unknown argument
 * metadata is fail-safe: every non-builtin/non-session kind
 * (extension/skill/file/template/custom/mcp and any future registry kind)
 * defaults to `draft` when neither `allowArgs` nor `hasSubcommands` is set,
 * so an incomplete command is never bare-executed. Builtin/session items
 * keep their explicit contract (only `allowArgs`/`hasSubcommands` draft);
 * file/template-sourced commands execute file content, so bare-executing an
 * argument-less name would run an incomplete command. Explicit
 * `allowArgs: false` + no subcommands still means runnable.
 */
export function mergePaletteItems(...sources: ReadonlyArray<readonly PaletteCommandSource[]>): CommandPaletteItem[] {
	const seen = new Set<string>();
	const items: CommandPaletteItem[] = [];
	for (const source of sources) {
		for (const cmd of source) {
			if (!cmd.name || seen.has(cmd.name)) continue;
			seen.add(cmd.name);
			const argUnknown = cmd.allowArgs === undefined && cmd.hasSubcommands === undefined;
			// Fail-safe draft: unknown argument metadata on any registry kind
			// outside the explicit builtin/session contract drafts `/name `
			// instead of bare-executing a possibly incomplete command.
			const unknownRegistryKind = cmd.group !== "builtin" && cmd.group !== "session";
			const takesArgs = cmd.allowArgs === true || cmd.hasSubcommands === true || (unknownRegistryKind && argUnknown);
			items.push({
				id: `/${cmd.name}`,
				title: `/${cmd.name}`,
				hint: cmd.description,
				group: cmd.group,
				...(takesArgs ? { intent: "draft" as const } : {}),
				...(cmd.inlineHint ? { argHint: cmd.inlineHint } : {}),
			});
		}
	}
	return items;
}

/**
 * Static palette sources for session-tab management, exposed so the owner can
 * project tab verbs into the palette alongside registry commands. They
 * dispatch as `/tab …` slash text through the existing
 * `executeBuiltinSlashCommand` path: verbs that take an optional target draft
 * `/tab <verb> ` (never execute an incomplete command), while arg-free verbs
 * run immediately. Key-hint suffixes resolve through
 * `CONTROL_COMMAND_KEYBINDINGS` below, so remaps stay visible; verbs without
 * a bound app action keep their description-only hint.
 */
export const TAB_MANAGEMENT_PALETTE_SOURCES: readonly PaletteCommandSource[] = [
	{
		name: "tab close",
		description: "Close session tab, keep runtime",
		group: "session",
		hasSubcommands: false,
		allowArgs: true,
		inlineHint: "[number]",
	},
	{
		name: "tab reopen",
		description: "Reopen the last closed session tab",
		group: "session",
		hasSubcommands: false,
		allowArgs: false,
	},
	{
		name: "tab archive",
		description: "Archive session, hide from tabs (reversible)",
		group: "session",
		hasSubcommands: false,
		allowArgs: true,
		inlineHint: "[number|id]",
	},
	{
		name: "tab restore",
		description: "Restore an archived session",
		group: "session",
		hasSubcommands: false,
		allowArgs: true,
		inlineHint: "[id]",
	},
];

/** Help text for `/tab` close/reopen, shared by the palette and the owner help path. */
export const TAB_CLOSE_REOPEN_HELP =
	"Close hides the tab and preserves its runtime (Alt+W); Reopen restores it (Ctrl+Shift+T)";

/**
 * Every dispatch result mapped to visible owner feedback. `executeBuiltinSlashCommand`
 * resolves `true` (consumed), a `string` (remaining prompt), or `false`
 * (no builtin matched): silent no-ops and rejections both become useful
 * messages instead of a dead Enter. The owner shows the returned string via
 * `showStatus`/`showError` and only writes the editor on `{ draft }`.
 */
export type PaletteDispatchFeedback =
	| { readonly kind: "draft"; readonly text: string }
	| { readonly kind: "status"; readonly text: string }
	| { readonly kind: "error"; readonly text: string };

/** Draft slash text (`/tab close `) a draft-intent item prepares in the editor. */
export function paletteDraftText(item: CommandPaletteItem): string {
	return `${paletteSlashText(item)} `;
}

export function describePaletteDispatchResult(
	item: CommandPaletteItem,
	result: string | boolean,
): PaletteDispatchFeedback {
	if (item.intent === "draft") return { kind: "draft", text: paletteDraftText(item) };
	if (typeof result === "string") {
		return result
			? { kind: "draft", text: result }
			: { kind: "status", text: `${item.title} ran with no remaining prompt` };
	}
	if (result === true) return { kind: "status", text: `${item.title} done` };
	return { kind: "error", text: `${item.title} is not a palette action — pick another command` };
}

/** Rejected dispatch promise mapped to a useful owner message. */
export function describePaletteDispatchError(item: CommandPaletteItem, error: unknown): PaletteDispatchFeedback {
	return {
		kind: "error",
		text: `${item.title} failed: ${error instanceof Error ? error.message : String(error)}`,
	};
}

/**
 * Selection routing plan shared by the production palette owner. Draft
 * items prepare editor text; run items first try the builtin dispatcher and,
 * when no builtin matches (`false`), fall back to normal submission
 * dispatch (skills, extension-local, custom/file/template expansion) so a
 * listed runnable non-builtin executes its actual handler exactly once.
 */
export type PaletteSelectionPlan =
	| { readonly kind: "draft"; readonly text: string }
	| { readonly kind: "dispatch"; readonly text: string };

export function resolvePaletteSelection(item: CommandPaletteItem): PaletteSelectionPlan {
	const text = paletteSlashText(item);
	if (item.intent === "draft") return { kind: "draft", text: paletteDraftText(item) };
	return { kind: "dispatch", text };
}

/** Reason the palette settled; the owner hides the overlay on either path. */
export type PaletteCloseReason = "escape" | "select";

/**
 * Owner-wired palette callbacks. The main thread owns the overlay handle, the
 * pre-open focus/draft snapshot, and the registries — this component only
 * reports intent:
 *
 * - `onSelect`: route `paletteSlashText(item)` through
 *   `executeBuiltinSlashCommand` (existing actions/commands; never shell
 *   strings), then hide the overlay.
 * - `onClose("escape")`: hide the overlay, restore the draft/focus snapshot,
 *   and retarget focus with `focusActiveEditorArea()` — never raw
 *   `setFocus(editor)` — so a hook selector/input/editor occupying the editor
 *   slot keeps keyboard focus.
 *
 * Escape is guarded once inside the component: repeated Escape presses invoke
 * `onClose` exactly one time.
 */
export interface CommandPaletteCallbacks {
	onSelect?: (item: CommandPaletteItem) => void;
	onClose?: (reason: PaletteCloseReason) => void;
}

/**
 * Control commands that bind an app-level key chord, mirrored from the
 * `/commands` and `/sidebar` specs in `builtin-control.ts`. The palette
 * derives its hint suffixes from `KeybindingsManager` through this map so
 * remaps stay visible; commands absent here keep their description-only hint.
 * Kept in this leaf module (rather than imported from the slash-command
 * tree) because that tree already reaches back into mode components.
 */
const CONTROL_COMMAND_KEYBINDINGS: Readonly<Record<string, AppKeybinding>> = {
	commands: "app.commands.open",
	sidebar: "app.sidebar.toggle",
	"tab close": "app.session.tab.close",
	"tab reopen": "app.session.tab.reopen",
};

/** Fixed panel chrome rows: top border, search prompt, bottom border. */
export const PALETTE_CHROME_ROWS = 3;
/** Reserved rows below the list so the selected row never sits flush on the frame edge. */
const PALETTE_RESERVE_ROWS = 1;

/**
 * List-body row budget for a viewport height, accounting the chrome rows above.
 * The owner calls this with the palette overlay's share of the terminal (about
 * 60% of rows) on open/resize and passes the result to `setMaxVisible`, so the
 * panel — search prompt, selected row, and controls — always fits instead of
 * assuming a fixed ten-item page.
 */
export function paletteListBudget(totalRows: number): number {
	return Math.max(2, Math.floor(totalRows) - PALETTE_CHROME_ROWS - PALETTE_RESERVE_ROWS);
}

/**
 * Slash text (`/name`) a palette item dispatches. The owner passes it to
 * `executeBuiltinSlashCommand`; items with arguments prepare the draft or open
 * the existing selector instead of executing.
 */
export function paletteSlashText(item: CommandPaletteItem): string {
	return item.id.startsWith("/") ? item.id : `/${item.id}`;
}

/**
 * Registry-backed command palette presentation. Bounded panel with search,
 * clear selection, and focus restoration. Execution routes through existing
 * actions/commands; items with arguments prepare the draft or open the
 * existing selector.
 */
export class CommandPaletteComponent implements Component {
	#items: readonly CommandPaletteItem[] = [];
	#query = "";
	#selected = 0;
	#settled = false;
	#keybindings: Pick<KeybindingsManager, "getDisplayString" | "getKeys"> | undefined;
	/** Maximum list-body rows per render; items with description rows cost two. */
	#maxListRows = paletteListBudget(14);
	/**
	 * Items fully visible in the last render; page jumps move by this count.
	 * Defaults to the full budget (single-row items) so paging before the
	 * first render still clamps instead of wrapping.
	 */
	#lastPageSize = paletteListBudget(14);
	onSelect: ((item: CommandPaletteItem) => void) | undefined;
	onClose: ((reason: PaletteCloseReason) => void) | undefined;

	setItems(items: readonly CommandPaletteItem[]): void {
		this.#items = items;
		this.#selected = 0;
		this.#lastPageSize = this.#maxListRows;
		// A fresh item list marks a fresh open session: re-arm the
		// Escape/select once-guard cleared by the previous close.
		this.#settled = false;
	}

	/**
	 * Refit the visible list budget to the viewport. The owner derives the
	 * value from `paletteListBudget` (overlay share of the terminal rows) on
	 * open and resize — mirroring the SessionList line budget and the
	 * history-search `setMaxVisible` contract.
	 */
	setMaxVisible(rows: number): void {
		this.#maxListRows = Math.max(2, Math.trunc(rows));
		this.#lastPageSize = this.#maxListRows;
		this.#clampSelection(this.filtered().length);
	}

	/** Items fully visible in the last render window, selection included. */
	visibleItems(): CommandPaletteItem[] {
		return this.#windowItems().list;
	}

	/** Page-jump distance: the last rendered window's item count (at least 1). */
	pageSize(): number {
		return Math.max(1, this.#lastPageSize);
	}

	setQuery(query: string): void {
		this.#query = query;
		this.#selected = 0;
	}

	/** Remap-aware key source for hint suffixes; unset keeps description-only hints. */
	setKeybindings(keybindings: Pick<KeybindingsManager, "getDisplayString" | "getKeys"> | undefined): void {
		this.#keybindings = keybindings;
	}

	/** True after the first Escape/Enter settled the session; further keys are ignored. */
	isSettled(): boolean {
		return this.#settled;
	}

	get query(): string {
		return this.#query;
	}

	filtered(): CommandPaletteItem[] {
		const q = this.#query.trim().toLowerCase();
		if (!q) return [...this.#items];
		return this.#items.filter(item => item.title.toLowerCase().includes(q) || item.id.toLowerCase().includes(q));
	}

	moveSelection(delta: number): void {
		const list = this.filtered();
		if (list.length === 0) {
			this.#selected = 0;
			return;
		}
		this.#clampSelection(list.length);
		if (Math.abs(delta) > 1) {
			// Page jumps clamp so they never underflow (JS % keeps the sign,
			// which produced -1 for len < 10) and never run past the end.
			this.#selected = Math.min(list.length - 1, Math.max(0, this.#selected + delta));
			return;
		}
		this.#selected = (((this.#selected + delta) % list.length) + list.length) % list.length;
	}

	selectedItem(): CommandPaletteItem | undefined {
		const list = this.filtered();
		this.#clampSelection(list.length);
		return list[this.#selected];
	}

	/** Keep the selection inside [0, length-1] (0 when empty). */
	#clampSelection(length: number): void {
		if (length <= 0) {
			this.#selected = 0;
			return;
		}
		if (this.#selected < 0) this.#selected = 0;
		else if (this.#selected > length - 1) this.#selected = length - 1;
	}

	handleInput(data: string): void {
		if (this.#settled) return;
		// Mouse tracking is off outside fullscreen overlays; drop stray SGR reports.
		if (data.startsWith("\x1b[<")) return;
		if (getKeybindings().matches(data, "tui.select.cancel")) {
			this.#closeOnce("escape");
			return;
		}
		if (matchesSelectUp(data)) {
			this.moveSelection(-1);
			return;
		}
		if (matchesSelectDown(data)) {
			this.moveSelection(1);
			return;
		}
		if (matchesSelectPageUp(data)) {
			this.moveSelection(-this.pageSize());
			return;
		}
		if (matchesSelectPageDown(data)) {
			this.moveSelection(this.pageSize());
			return;
		}
		if (matchesKey(data, "home")) {
			this.#selected = 0;
			return;
		}
		if (matchesKey(data, "end")) {
			this.#selected = Math.max(0, this.filtered().length - 1);
			return;
		}
		if (matchesKey(data, "enter") || matchesKey(data, "return") || data === "\n" || data === "\r") {
			this.#selectOnce();
			return;
		}
		if (getKeybindings().matches(data, "tui.editor.deleteCharBackward")) {
			this.#query = this.#query.slice(0, -1);
			this.#selected = 0;
			return;
		}
		if (data.length === 1 && (data.codePointAt(0) ?? 0) >= 32) {
			this.#query += data;
			this.#selected = 0;
			return;
		}
		if (data.length > 1) {
			// Unmatched escape sequences are editor/global chords, not text.
			if (data.charCodeAt(0) === 0x1b) return;
			// Multi-char paste/IME commit: insert the printable run, stripping
			// control chars (ANSI leftovers included — ESC itself is < 32).
			let run = "";
			for (const ch of data) {
				if ((ch.codePointAt(0) ?? 0) >= 32) run += ch;
			}
			if (run.length > 0) {
				this.#query += run;
				this.#selected = 0;
			}
			return;
		}
		// Every other chord (Ctrl+P/T, Shift+Tab, Ctrl+L/Q, F5/Alt+R, tabs,
		// word-left Alt+B, …) is intentionally ignored: the palette never steals
		// editor/global bindings.
	}

	/** Remap-aware key suffix for commands that bind an app action; undefined otherwise. */
	#keyHint(item: CommandPaletteItem): string | undefined {
		const manager = this.#keybindings;
		if (!manager) return undefined;
		const name = item.id.startsWith("/") ? item.id.slice(1) : item.id;
		const action = CONTROL_COMMAND_KEYBINDINGS[name];
		if (!action) return undefined;
		const display = manager.getDisplayString(action) || formatKeyHints(manager.getKeys(action));
		return display || undefined;
	}

	#selectOnce(): void {
		if (this.#settled) return;
		const item = this.selectedItem();
		if (!item) return;
		this.#settled = true;
		this.onSelect?.(item);
	}

	#closeOnce(reason: PaletteCloseReason): void {
		if (this.#settled) return;
		this.#settled = true;
		this.onClose?.(reason);
	}

	render(width: number): readonly string[] {
		const w = clampDialogWidth(width, 60);
		const inner = Math.max(1, w - 4);
		const rows: string[] = [topBorder(w, "Commands")];
		const prompt = replaceTabs(`› ${this.#query}`);
		rows.push(row(truncateToWidth(prompt, inner), w, "accent"));
		const full = this.filtered();
		this.#clampSelection(full.length);
		// Viewport-budgeted window: grow around the selection until the list
		// body budget is spent, so the selected row is always visible without a
		// fixed page-size assumption. Items with a description row cost two.
		const { list, start } = this.#windowItems(full);
		this.#lastPageSize = Math.max(1, list.length);
		const windowStart = start;
		if (list.length === 0) {
			rows.push(row(theme.fg("muted", "No matching commands"), w));
		}
		list.forEach((item, index) => {
			const active = windowStart + index === this.#selected;
			const key = this.#keyHint(item);
			const argMarker = item.argHint ?? (item.intent === "draft" ? "takes arguments" : undefined);
			const hintParts = [item.hint, argMarker, key].filter((part): part is string => !!part);
			const hintText = hintParts.length > 0 ? hintParts.join(" · ") : undefined;
			const label = active ? theme.fg("accent", `› ${item.title}`) : `  ${item.title}`;
			const hint = hintText ? theme.fg("muted", ` ${hintText}`) : "";
			const combined = truncateToWidth(`${label}${visibleWidth(hint) > 0 ? "" : ""}`, inner);
			rows.push(row(active ? theme.bg("selectedBg", combined) : combined, w, active ? "accent" : undefined));
			if (hint) rows.push(row(theme.fg("muted", truncateToWidth(replaceTabs(`    ${hintText ?? ""}`), inner)), w));
		});
		rows.push(bottomBorder(w));
		return rows;
	}

	/**
	 * Budget window over `full` (or the current filter) centered on the
	 * selection: alternate growth below/above until the row budget is spent.
	 * Returns the visible items plus the window's start index in `full`.
	 */
	#windowItems(full?: CommandPaletteItem[]): { list: CommandPaletteItem[]; start: number } {
		const items = full ?? this.filtered();
		this.#clampSelection(items.length);
		if (items.length === 0) return { list: [], start: 0 };
		const cost = (item: CommandPaletteItem): number => (this.#hasHintRow(item) ? 2 : 1);
		let start = this.#selected;
		let end = this.#selected + 1;
		let used = cost(items[this.#selected]!);
		for (let preferDown = true; ; preferDown = !preferDown) {
			const canDown = end < items.length && used + cost(items[end]!) <= this.#maxListRows;
			const canUp = start > 0 && used + cost(items[start - 1]!) <= this.#maxListRows;
			if (!canDown && !canUp) break;
			if (canDown && (preferDown || !canUp)) {
				used += cost(items[end]!);
				end++;
			} else {
				start--;
				used += cost(items[start]!);
			}
		}
		return { list: items.slice(start, end), start };
	}

	#hasHintRow(item: CommandPaletteItem): boolean {
		// Mirrors the render path: a description row appears whenever any hint
		// part (description, argument marker, or remap-aware key suffix) exists.
		// Empty-string descriptions carry no visible text, so they must not
		// mask a draft marker when costing the row budget.
		if (item.hint || item.argHint || item.intent === "draft") return true;
		return this.#keyHint(item) !== undefined;
	}
}
