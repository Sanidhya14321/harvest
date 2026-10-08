import {
	Container,
	matchesKey,
	parseSgrMouse,
	ScrollView,
	type SgrMouseEvent,
	Spacer,
	TruncatedText,
} from "@harvest/pi-tui";
import { theme } from "../../modes/theme/theme";
import { matchesSelectCancel, matchesSelectDown, matchesSelectUp } from "../../modes/utils/keybinding-matchers";
import type { LogoutAccount } from "../../slash-commands/helpers/logout";
import { editorKey } from "./keybinding-hints";
import { type DialogLayout, OverlayPanel, renderChoiceDialog } from "./overlay-box";

const LOGOUT_SELECTOR_MAX_VISIBLE = 10;

/** Account picker for `/logout` after the provider has been selected. */
export class LogoutAccountSelectorComponent extends OverlayPanel {
	#listContainer: Container;
	#accounts: LogoutAccount[];
	#selectedIndex = 0;
	#statusMessage: string | undefined;
	#onSelectCallback: (account: LogoutAccount) => void;
	#onCancelCallback: () => void;
	#layout: DialogLayout | undefined;

	constructor(
		providerName: string,
		accounts: LogoutAccount[],
		onSelect: (account: LogoutAccount) => void,
		onCancel: () => void,
	) {
		super(`Select ${providerName} account to log out`);
		this.#accounts = accounts;
		this.#onSelectCallback = onSelect;
		this.#onCancelCallback = onCancel;
		const activeIndex = accounts.findIndex(account => account.active);
		this.#selectedIndex = activeIndex >= 0 ? activeIndex : 0;

		this.#listContainer = new Container();
		this.addChild(this.#listContainer);
		this.#updateList();
	}

	#updateList(): void {
		this.#listContainer.clear();

		const total = this.#accounts.length;
		const maxVisible = LOGOUT_SELECTOR_MAX_VISIBLE;
		const startIndex =
			total <= maxVisible
				? 0
				: Math.max(0, Math.min(this.#selectedIndex - Math.floor(maxVisible / 2), total - maxVisible));
		const endIndex = Math.min(startIndex + maxVisible, total);

		const rows: string[] = [];
		for (let i = startIndex; i < endIndex; i++) {
			const account = this.#accounts[i];
			if (!account) continue;
			const activeTag = account.active ? theme.fg("muted", " (active)") : "";
			const detail = account.detail ? theme.fg("dim", `  ${account.detail}`) : "";
			if (i === this.#selectedIndex) {
				rows.push(`${theme.fg("accent", `${theme.nav.cursor} ${account.label}`)}${activeTag}${detail}`);
			} else {
				rows.push(`  ${account.label}${activeTag}${detail}`);
			}
		}

		if (rows.length > 0) {
			const sv = new ScrollView(rows, {
				height: rows.length,
				scrollbar: "auto",
				totalRows: total,
				theme: { track: text => theme.fg("muted", text), thumb: text => theme.fg("accent", text) },
			});
			sv.setScrollOffset(startIndex);
			this.#listContainer.addChild(sv);
		}

		if (total === 0) {
			this.#listContainer.addChild(new TruncatedText(theme.fg("muted", "No stored accounts to log out"), 0, 0));
		}

		this.#listContainer.addChild(
			new TruncatedText(
				theme.fg("muted", ["Up/Down select", "Enter log out account", "Esc cancel"].join(theme.sep.dot)),
				0,
				0,
			),
		);

		if (this.#statusMessage) {
			this.#listContainer.addChild(new Spacer(1));
			this.#listContainer.addChild(new TruncatedText(theme.fg("warning", this.#statusMessage), 0, 0));
		}
	}

	handleInput(keyData: string): void {
		const mouse = parseSgrMouse(keyData);
		if (mouse) {
			this.routeMouse(mouse, mouse.row, mouse.col);
			return;
		}
		if (matchesSelectCancel(keyData)) {
			this.#onCancelCallback();
			return;
		}

		if (matchesSelectUp(keyData)) {
			if (this.#accounts.length > 0) {
				this.#selectedIndex = this.#selectedIndex === 0 ? this.#accounts.length - 1 : this.#selectedIndex - 1;
			}
			this.#statusMessage = undefined;
			this.#updateList();
		} else if (matchesSelectDown(keyData)) {
			if (this.#accounts.length > 0) {
				this.#selectedIndex = this.#selectedIndex === this.#accounts.length - 1 ? 0 : this.#selectedIndex + 1;
			}
			this.#statusMessage = undefined;
			this.#updateList();
		} else if (matchesKey(keyData, "pageUp")) {
			if (this.#accounts.length > 0) {
				this.#selectedIndex = Math.max(0, this.#selectedIndex - LOGOUT_SELECTOR_MAX_VISIBLE);
			}
			this.#statusMessage = undefined;
			this.#updateList();
		} else if (matchesKey(keyData, "pageDown")) {
			if (this.#accounts.length > 0) {
				this.#selectedIndex = Math.min(
					this.#accounts.length - 1,
					this.#selectedIndex + LOGOUT_SELECTOR_MAX_VISIBLE,
				);
			}
			this.#statusMessage = undefined;
			this.#updateList();
		} else if (matchesKey(keyData, "enter") || matchesKey(keyData, "return") || keyData === "\n") {
			const account = this.#accounts[this.#selectedIndex];
			if (!account) return;
			this.#onSelectCallback(account);
		}
	}

	override render(width: number): readonly string[] {
		const choices = this.#accounts.map(
			(account, index) =>
				`${index === this.#selectedIndex ? theme.nav.cursor : " "} ${account.label}${account.active ? " (active)" : ""}${account.detail ? `  ${account.detail}` : ""}`,
		);
		if (choices.length === 0) choices.push(theme.fg("muted", "No stored accounts to log out"));
		const footer = [
			`${editorKey("tui.select.cancel") || "Esc"} cancel`,
			"Enter log out",
			`${this.#accounts.length ? this.#selectedIndex + 1 : 0}/${this.#accounts.length}`,
		].join(theme.sep.dot);
		this.#layout = renderChoiceDialog(
			this.title,
			choices,
			this.#selectedIndex,
			width,
			Math.min(this.getMaxHeight(), LOGOUT_SELECTOR_MAX_VISIBLE + 3),
			footer,
		);
		return this.#layout.lines;
	}

	routeMouse(event: SgrMouseEvent, line: number, _col: number): void {
		const layout = this.#layout;
		if (!layout) return;
		const row = line - layout.bodyRowStart;
		if (row < 0 || row >= layout.bodyRows) return;
		if (event.wheel !== null) {
			if (this.#accounts.length)
				this.#selectedIndex = (this.#selectedIndex + event.wheel + this.#accounts.length) % this.#accounts.length;
			this.#statusMessage = undefined;
			this.#updateList();
		} else if (event.leftClick) {
			const index = layout.bodyWindowStart + row;
			const account = this.#accounts[index];
			if (!account) return;
			this.#selectedIndex = index;
			this.#onSelectCallback(account);
		}
	}
}
