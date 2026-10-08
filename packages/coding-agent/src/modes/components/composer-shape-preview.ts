/**
 * Live preview for the `composer.shape` setting and the setup-wizard composer
 * scene. Chrome is rendered through the same {@link ComposerStyle} objects the
 * real editor uses, and status rows come from the live
 * {@link ComposerPreviewStatusSource} (the session's StatusLineComponent) —
 * nothing about the preview is a re-implementation. Prompt text is a preview
 * stand-in, and the `session_name` segment falls back to a stand-in title
 * (passed via `previewTitle`) when the session is unnamed.
 */
import {
	type Component,
	Editor,
	Ellipsis,
	type EditorTopBorder,
	getComposerStyle,
	truncateToWidth,
} from "@harvest/pi-tui";
import type { ComposerShape } from "../../config/settings-schema";
import { getEditorTheme, theme } from "../theme/theme";

/**
 * Real status renderer the preview borrows rows from — structurally satisfied
 * by {@link StatusLineComponent}. Layout is parameterized so a preview can
 * render a candidate shape's placement instead of the active one.
 */
export interface ComposerPreviewStatusSource {
	/** Powerline bar with the context gauge (box top border content). */
	getTopBorder(width: number, previewTitle?: string): { content: string; width: number };
	/** Flush soft-capped powerline band (band composer top row). */
	getBandTopBorder(width: number, previewTitle?: string): { content: string; width: number };
	/** Plain right-group chip (claude top rule content). */
	getStandaloneTopBorder(width: number, previewTitle?: string): { content: string; width: number };
	/** Plain standalone bottom bar carrying the given segment groups. */
	renderBottomBar(width: number, groups: "left" | "full", previewTitle?: string): string;
}

export interface ComposerShapePreviewOptions {
	requestRender?: () => void;
	/** Live status renderer; omitted (tests), the chrome renders without status rows. */
	status?: ComposerPreviewStatusSource;
}
/** Stand-in session title shown while the previewed session is unnamed. */
const PREVIEW_TITLE = "harvest";

export function renderComposerShapePreview(
	shape: ComposerShape,
	width: number,
	status?: ComposerPreviewStatusSource,
): readonly string[] {
	const previewWidth = Math.max(0, Math.min(Math.floor(width), 96));
	if (previewWidth === 0) return [];
	const style = getComposerStyle(shape);
	const paddingX = style.defaultPaddingX(undefined);
	const chromeWidth = style.sideChromeWidth(paddingX);

	let topBorder: EditorTopBorder | undefined;
	if (status) {
		if (style.statusAttachment === "top-border") {
			topBorder = status.getTopBorder(Math.max(1, previewWidth - chromeWidth * 2), PREVIEW_TITLE);
		} else if (style.statusAttachment === "top-band") {
			topBorder = status.getBandTopBorder(previewWidth, PREVIEW_TITLE);
		} else if (style.statusAttachment === "top-rule-chip") {
			topBorder = status.getStandaloneTopBorder(previewWidth, PREVIEW_TITLE);
		}
	}

	// Borrow the complete editor layout, including tiny-width chrome collapse,
	// symbol gutters and composer surface colors, without claiming UI focus.
	const editor = new Editor(getEditorTheme());
	editor.setBorderStyle(shape);
	editor.setMaxHeight(previewWidth < 5 ? 1 : style.verticalChrome + 1);
	editor.setTopBorder(topBorder);
	editor.setText("Ask anything, edit files, run tools");
	const lines = [...editor.render(previewWidth)];

	if (style.bottomBar !== "none" && status) {
		const bar = status.renderBottomBar(previewWidth, style.bottomBar, PREVIEW_TITLE);
		if (bar) {
			if (style.bottomBarGap) lines.push("");
			lines.push(bar);
		}
	}
	return lines.map(line =>
		truncateToWidth(line, previewWidth, theme.getSymbolPreset() === "ascii" ? Ellipsis.Ascii : Ellipsis.Unicode),
	);
}

export class ComposerShapePreview implements Component {
	#shape: ComposerShape;
	#options: ComposerShapePreviewOptions;

	constructor(initialValue: ComposerShape = "band", options: ComposerShapePreviewOptions = {}) {
		this.#shape = initialValue;
		this.#options = options;
	}

	setValue(shape: ComposerShape): void {
		if (this.#shape === shape) return;
		this.#shape = shape;
		this.#options.requestRender?.();
	}

	render(width: number): readonly string[] {
		if (width <= 0) return [];
		const lines = renderComposerShapePreview(this.#shape, width, this.#options.status);
		return [
			"",
			theme.fg(
				"muted",
				truncateToWidth("Preview:", width, theme.getSymbolPreset() === "ascii" ? Ellipsis.Ascii : Ellipsis.Unicode),
			),
			...lines,
		];
	}
}
