import { Ellipsis, replaceTabs, truncateToWidth, visibleWidth, wrapTextWithAnsi } from "@harvest/pi-tui";
import { expandKeyHint } from "../../tools/render-utils";
import { theme } from "../theme/theme";

/** Indent for every row after the first, so continuations hang under the prefix. */
const CONTINUATION_INDENT = "  ";

/**
 * Format a provider error into at most `maxRows` wrapped rows for a
 * `WidthAwareText` of `contentWidth` cells. Lines wrap to the render width
 * instead of being cut at a fixed column, so a long single-line body — a raw
 * 400 JSON envelope, a `raw-http-request=` dump path — stays readable. Blank
 * lines are dropped. When rows are cut, a dim `… +N more lines (<key> to
 * expand)` row follows; pass `Infinity` to keep every row. `styleLine` colors
 * logical line `index` and supplies line 0's prefix (`Error: `, the banner
 * glyph). Shared by the inline transcript error and the pinned banner.
 */
export function formatErrorBlock(
	message: string,
	contentWidth: number,
	maxRows: number,
	styleLine: (line: string, index: number) => string,
): string {
	contentWidth = Math.max(0, Math.floor(contentWidth));
	if (contentWidth === 0) return "";
	const lines = replaceTabs(message)
		.split("\n")
		.map(line => line.trim())
		.filter(line => line.length > 0);
	if (lines.length === 0) lines.push("Unknown error");
	const indent = CONTINUATION_INDENT.slice(0, Math.max(0, contentWidth - 1));
	const wrapWidth = contentWidth - indent.length;
	const rows: string[] = [];
	for (let index = 0; index < lines.length; index++) {
		for (const row of wrapTextWithAnsi(styleLine(lines[index]!, index), wrapWidth, { hard: true })) {
			rows.push(rows.length === 0 ? row : `${indent}${row}`);
		}
	}
	if (rows.length > maxRows) {
		const hidden = rows.length - maxRows;
		rows.length = maxRows;
		const expandKey = expandKeyHint();
		const fullHint = `${indent}${theme.symbol("sep.ellipsis")} +${hidden} more line${hidden === 1 ? "" : "s"} (${expandKey} to expand)`;
		// Keep the action key reachable when explanation cannot fit the row.
		const hint = visibleWidth(fullHint) > contentWidth ? `${expandKey} expand` : fullHint;
		rows.push(
			theme.fg(
				"dim",
				truncateToWidth(
					hint,
					contentWidth,
					theme.getSymbolPreset() === "ascii" ? Ellipsis.Ascii : Ellipsis.Unicode,
				),
			),
		);
	}
	return rows.join("\n");
}
