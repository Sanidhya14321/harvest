import { type Component, Ellipsis, matchesKey, replaceTabs, ScrollView, Text, wrapTextWithAnsi } from "@harvest/pi-tui";
import { getSymbolTheme, theme } from "../theme/theme";
import { matchesSelectCancel } from "../utils/keybinding-matchers";
import { dialogContentWidth, renderDialog } from "./overlay-box";

/** Terminal surface needed to size the session info viewport. */
export interface SessionInfoOverlayHost {
	readonly terminal: {
		readonly rows: number;
	};
}

/** Focused, dismissible `/session` information panel. */
export class SessionInfoOverlay implements Component {
	readonly #host: SessionInfoOverlayHost;
	readonly #onClose: () => void;
	readonly #info: Text;
	readonly #scrollView: ScrollView;
	#lastInfoLines: readonly string[] | undefined;
	#lastLayoutWidth: number | undefined;
	#lastBodyHeight: number | undefined;
	#lastHeight: number | undefined;
	#maxHeight: number | undefined;

	constructor(host: SessionInfoOverlayHost, info: string, onClose: () => void) {
		this.#host = host;
		this.#onClose = onClose;
		this.#info = new Text(replaceTabs(info), 0, 0);
		this.#scrollView = new ScrollView([], {
			height: 0,
			scrollbar: "auto",
			ellipsis: Ellipsis.Omit,
			theme: {
				track: text => theme.fg("dim", text),
				thumb: text => theme.fg("accent", text),
			},
		});
	}

	handleInput(data: string): void {
		if (matchesSelectCancel(data) || matchesKey(data, "escape") || matchesKey(data, "esc")) {
			this.#onClose();
			return;
		}
		this.#scrollView.handleScrollKey(data);
	}

	invalidate(): void {
		this.#info.invalidate();
		this.#lastInfoLines = undefined;
		this.#lastLayoutWidth = undefined;
		this.#lastBodyHeight = undefined;
		this.#lastHeight = undefined;
		this.#scrollView.invalidate();
	}

	setIgnoreTight(ignore: boolean): this {
		this.#info.setIgnoreTight(ignore);
		return this;
	}

	dispose(): void {
		this.#scrollView.invalidate();
	}

	setMaxHeight(height: number): void {
		this.#maxHeight = Math.max(1, Math.floor(height));
	}

	render(width: number): readonly string[] {
		const innerWidth = dialogContentWidth(width);
		const budget = this.#maxHeight ?? Math.max(1, this.#host.terminal.rows);
		const footer = `${theme.getSymbolPreset() === "ascii" ? "up/down" : "↑/↓"} scroll${theme.sep.dot}Esc close`;
		const chrome = Number(budget >= 3) + Number(budget >= 2) + Number(budget >= 6);
		const maxBodyHeight = Math.max(1, budget - chrome);
		if (this.#lastLayoutWidth !== innerWidth || this.#lastBodyHeight !== maxBodyHeight || !this.#lastInfoLines) {
			const fullWidthInfoLines = wrapTextWithAnsi(this.#info.getText(), innerWidth, { hard: true });
			const infoWidth = fullWidthInfoLines.length > maxBodyHeight ? Math.max(1, innerWidth - 1) : innerWidth;
			const infoLines =
				infoWidth === innerWidth
					? fullWidthInfoLines
					: wrapTextWithAnsi(this.#info.getText(), infoWidth, { hard: true });
			this.#scrollView.setLines(infoLines);
			this.#lastInfoLines = infoLines;
			this.#lastLayoutWidth = innerWidth;
			this.#lastBodyHeight = maxBodyHeight;
		}

		const height = Math.max(1, Math.min(this.#lastInfoLines.length, maxBodyHeight));
		if (this.#lastHeight !== height) {
			this.#scrollView.setHeight(height);
			this.#lastHeight = height;
		}
		this.#scrollView.setSymbols(getSymbolTheme());
		return renderDialog("Session Info", this.#scrollView.render(innerWidth), width, budget, footer).lines;
	}
}
