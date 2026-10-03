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
};

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
	onSelect: ((item: CommandPaletteItem) => void) | undefined;
	onClose: ((reason: PaletteCloseReason) => void) | undefined;

	setItems(items: readonly CommandPaletteItem[]): void {
		this.#items = items;
		this.#selected = 0;
		// A fresh item list marks a fresh open session: re-arm the
		// Escape/select once-guard cleared by the previous close.
		this.#settled = false;
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
		this.#selected = (this.#selected + delta + list.length) % list.length;
	}

	selectedItem(): CommandPaletteItem | undefined {
		return this.filtered()[this.#selected];
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
			this.moveSelection(-10);
			return;
		}
		if (matchesSelectPageDown(data)) {
			this.moveSelection(10);
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
		if (data.length === 1 && data.charCodeAt(0) >= 32) {
			this.#query += data;
			this.#selected = 0;
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
		const list = this.filtered().slice(0, 10);
		if (list.length === 0) {
			rows.push(row(theme.fg("muted", "No matching commands"), w));
		}
		list.forEach((item, index) => {
			const active = index === this.#selected;
			const key = this.#keyHint(item);
			const hintText = key ? (item.hint ? `${item.hint} · ${key}` : key) : item.hint;
			const label = active ? theme.fg("accent", `› ${item.title}`) : `  ${item.title}`;
			const hint = hintText ? theme.fg("muted", ` ${hintText}`) : "";
			const combined = truncateToWidth(`${label}${visibleWidth(hint) > 0 ? "" : ""}`, inner);
			rows.push(row(active ? theme.bg("selectedBg", combined) : combined, w, active ? "accent" : undefined));
			if (hint) rows.push(row(theme.fg("muted", truncateToWidth(replaceTabs(`    ${hintText ?? ""}`), inner)), w));
		});
		rows.push(bottomBorder(w));
		return rows;
	}
}
