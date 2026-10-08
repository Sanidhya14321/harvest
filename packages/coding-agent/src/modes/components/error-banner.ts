import { type Component, Ellipsis, truncateToWidth } from "@harvest/pi-tui";
import { outputPanelContentWidth, renderOutputPanelLines } from "../../tui/output-block";
import { getThemeEpoch, theme } from "../theme/theme";
import { formatErrorBlock } from "./error-block";

/** Max wrapped rows of the error message shown in the pinned banner. */
const MAX_BANNER_ROWS = 4;
const DEFAULT_MAX_HEIGHT = MAX_BANNER_ROWS + 2;

/**
 * A persistent error banner pinned above the editor. Unlike the transcript
 * "Error: …" line (which scrolls away as the conversation grows), this stays in
 * the fixed region directly above the input so a turn that ended on a provider
 * error — e.g. Anthropic's "Output blocked by content filtering policy" — cannot
 * be missed. It is cleared when the next turn starts. The message wraps to the
 * render width and keeps {@link MAX_BANNER_ROWS} rows; the expand hint on the
 * overflow row reveals the full body inline in the transcript.
 */
export class ErrorBannerComponent implements Component {
	#maxHeight = DEFAULT_MAX_HEIGHT;
	#cache: { width: number; height: number; epoch: number; rows: string[] } | undefined;

	constructor(private readonly message: string) {}

	setMaxHeight(height: number): void {
		this.#maxHeight = Math.max(1, Math.floor(height));
	}

	invalidate(): void {
		this.#cache = undefined;
	}

	render(width: number): readonly string[] {
		const safeWidth = Math.max(0, Math.floor(width));
		if (safeWidth === 0) return [];
		const epoch = getThemeEpoch();
		if (this.#cache?.width === safeWidth && this.#cache.height === this.#maxHeight && this.#cache.epoch === epoch)
			return this.#cache.rows;
		const contentWidth = outputPanelContentWidth(safeWidth);
		const footerRows = Number(this.#maxHeight >= 2);
		const bodyRows = this.#maxHeight - footerRows;
		const style = (line: string, index: number): string =>
			index === 0
				? theme.bold(theme.fg("error", `${contentWidth >= 4 ? `${theme.status.error} ` : ""}${line}`))
				: theme.fg("error", line);
		let body = formatErrorBlock(this.message, contentWidth, MAX_BANNER_ROWS, style).split("\n");
		if (body.length > bodyRows && bodyRows >= 2) {
			body = formatErrorBlock(this.message, contentWidth, bodyRows - 1, style).split("\n");
		}
		body = body.slice(0, bodyRows);
		if (footerRows)
			body.push(
				theme.fg(
					"dim",
					truncateToWidth(
						"Dismissed when you send your next message.",
						contentWidth,
						theme.getSymbolPreset() === "ascii" ? Ellipsis.Ascii : Ellipsis.Unicode,
					),
				),
			);
		const rows = renderOutputPanelLines(body, safeWidth, theme, { accent: "error", background: "panelBg" });
		this.#cache = { width: safeWidth, height: this.#maxHeight, epoch, rows };
		return rows;
	}
}
