import { type Component, visibleWidth } from "@harvest/pi-tui";
import type { AdvisorMessageDetails, AdvisorSeverity } from "../../advisor";
import {
	createCachedComponent,
	formatBadge,
	replaceTabs,
	type ToolUIColor,
	wrapTextWithAnsi,
} from "../../tools/render-utils";
import { Ellipsis, truncateToWidth } from "../../tui";
import type { Theme } from "../theme/theme";

const COLLAPSED_NOTES = 3;
const NOTE_LINE_WIDTH = 110;

function severityColor(severity: AdvisorSeverity | undefined): ToolUIColor {
	switch (severity) {
		case "blocker":
			return "error";
		case "concern":
			return "warning";
		default:
			return "muted";
	}
}

/**
 * Display-only transcript card for advisor notes injected into the primary
 * session. Styled as a distinct voice so notes never blend into thinking
 * output (whose `thinkingText` color equals `toolOutput` in most themes):
 * a bold `customMessageLabel` header tag (skill-card convention), a heavy
 * rail tinted per-note severity, and the note body on the default text color.
 */
export function createAdvisorMessageCard(
	details: AdvisorMessageDetails | undefined,
	getExpanded: () => boolean,
	uiTheme: Theme,
): Component {
	const notes = details?.notes ?? [];
	const blockers = notes.filter(note => note.severity === "blocker").length;
	const meta: string[] = [`${notes.length} ${notes.length === 1 ? "note" : "notes"}`];
	if (blockers > 0) meta.push(uiTheme.fg("error", `${blockers} blocker${blockers === 1 ? "" : "s"}`));

	return createCachedComponent(
		getExpanded,
		(width, expanded) => {
			const tag = uiTheme.fg("customMessageLabel", uiTheme.bold(`${uiTheme.status.info} Advisor`));
			const lines = [`${tag} ${uiTheme.fg("dim", meta.join(uiTheme.sep.dot))}`];
			const railGlyph = uiTheme.symbol("advisor.rail");
			const shown = expanded ? notes : notes.slice(0, COLLAPSED_NOTES);
			for (const entry of shown) {
				const badge = entry.severity
					? `${formatBadge(entry.severity, severityColor(entry.severity), uiTheme)} `
					: "";
				// Multi-advisor: attribute the note to its source. The implicit
				// single ("default") advisor renders unlabeled, as before.
				const who =
					entry.advisor && entry.advisor !== "default"
						? `${uiTheme.fg("dim", `[${replaceTabs(entry.advisor)}]`)} `
						: "";
				const rail = uiTheme.fg(severityColor(entry.severity), railGlyph);
				const prefix = width > 4 ? `  ${rail} ` : width > 2 ? `${rail} ` : "";
				const bodyWidth = Math.max(1, Math.min(NOTE_LINE_WIDTH, width) - visibleWidth(prefix));
				// Attribution owns its row: an arbitrarily long source can never
				// consume the first note's body allocation and silently erase words.
				const attribution = `${badge}${who}`.trimEnd();
				if (attribution) {
					const attributionLines = expanded
						? wrapTextWithAnsi(attribution, bodyWidth)
						: [
								truncateToWidth(
									attribution,
									bodyWidth,
									uiTheme.getSymbolPreset() === "ascii" ? Ellipsis.Ascii : Ellipsis.Unicode,
								),
							];
					for (const line of attributionLines) lines.push(prefix + line);
				}
				for (const paragraph of entry.note.split("\n").filter(p => p.trim())) {
					for (const line of wrapTextWithAnsi(replaceTabs(paragraph), bodyWidth)) {
						lines.push(`${prefix}${uiTheme.fg("customMessageText", line)}`);
					}
				}
			}
			const hidden = notes.length - shown.length;
			if (hidden > 0) {
				const rail = uiTheme.fg("dim", railGlyph);
				lines.push(
					`  ${rail} ${uiTheme.fg("dim", `${uiTheme.symbol("sep.ellipsis")} +${hidden} more ${hidden === 1 ? "note" : "notes"}`)}`,
				);
			}
			return lines.map(line =>
				truncateToWidth(line, width, uiTheme.getSymbolPreset() === "ascii" ? Ellipsis.Ascii : Ellipsis.Unicode),
			);
		},
		{ paddingX: 1 },
	);
}
