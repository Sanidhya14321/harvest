/**
 * Bordered output container with optional header and sections.
 */
import type { Component } from "@harvest/pi-tui";
import {
	applyBackgroundToLine,
	Container,
	Ellipsis,
	ImageProtocol,
	padding,
	TERMINAL,
	visibleWidth,
	wrapTextWithAnsi,
} from "@harvest/pi-tui";
import { getThemeEpoch, type Theme, type ThemeBg, type ThemeColor } from "../modes/theme/theme";
import { getSixelLineMask } from "../utils/sixel";
import type { State } from "./types";
import type { RenderCache } from "./utils";
import { getStateBgColor, Hasher, padToWidth, truncateToWidth } from "./utils";

export interface OutputBlockOptions {
	header?: string;
	headerMeta?: string;
	state?: State;
	sections?: Array<{ label?: string; lines: readonly string[]; separator?: boolean }>;
	width: number;
	applyBg?: boolean;
	contentPaddingLeft?: number;
	contentPaddingRight?: number;
	/** Override the state-derived border color. Used for muted "legacy" tool
	 * frames that should not visually compete with framed-output tools. */
	borderColor?: ThemeColor;
	/**
	 * Visual variant: `frame` keeps the rounded output box; `rail` renders a
	 * quiet panel with a left accent rail (OpenCode-like inline activity);
	 * `plain` renders inset content on the screen background without chrome.
	 */
	variant?: "frame" | "rail" | "plain";
}

const FRAMED_BLOCK_COMPONENT = Symbol("framedBlockComponent");

export type FramedBlockComponent = Component & { [FRAMED_BLOCK_COMPONENT]?: true };

export function markFramedBlockComponent<T extends Component>(component: T): T & FramedBlockComponent {
	(component as T & FramedBlockComponent)[FRAMED_BLOCK_COMPONENT] = true;
	return component as T & FramedBlockComponent;
}

export function isFramedBlockComponent(component: Component): boolean {
	return (component as FramedBlockComponent)[FRAMED_BLOCK_COMPONENT] === true;
}

type BlockRow =
	| { kind: "bar"; leftChar: string; rightChar: string; label?: string; meta?: string }
	| { kind: "bottom"; leftChar: string; rightChar: string }
	| { kind: "content"; inner: string }
	| { kind: "sixel"; raw: string };

function normalizeContentPaddingLeft(value: number | undefined): number {
	if (value === undefined || !Number.isFinite(value)) return 1;
	return Math.max(0, Math.floor(value));
}

/**
 * Inner content width that {@link renderOutputBlock} wraps its body to, for a
 * given outer `width`: both vertical borders plus symmetric content padding.
 * An explicit left padding of zero keeps legacy flush blocks flush on both
 * sides unless a right padding is provided separately.
 */
export function outputBlockContentWidth(
	width: number,
	contentPaddingLeft?: number,
	contentPaddingRight?: number,
): number {
	const left = normalizeContentPaddingLeft(contentPaddingLeft);
	const right = normalizeContentPaddingLeft(contentPaddingRight ?? left);
	return Math.max(1, width - 2 - left - right);
}

export function renderOutputBlock(options: OutputBlockOptions, theme: Theme): string[] {
	if (options.width <= 0) return [];
	if (options.width < 5) return renderRailPanel(options, theme);
	if (options.variant === "rail" || options.variant === "plain") return renderRailPanel(options, theme);
	const { header, headerMeta, state, sections = [], width, applyBg = true } = options;
	const ellipsis = theme.getSymbolPreset() === "ascii" ? Ellipsis.Ascii : Ellipsis.Unicode;
	const h = theme.boxRound.horizontal;
	const v = theme.boxRound.vertical;
	const cap = h.repeat(3);
	const lineWidth = Math.max(0, width);
	// Border colors: running/pending use accent, success uses dim (gray), error/warning keep their colors
	const borderColor: ThemeColor =
		options.borderColor ??
		(state === "error"
			? "error"
			: state === "warning"
				? "warning"
				: state === "running" || state === "pending"
					? "accent"
					: "dim");
	const border = (text: string) => theme.fg(borderColor, text);
	const bgFn = (() => {
		if (!state || !applyBg) return undefined;
		const bgAnsi = theme.getBgAnsi(getStateBgColor(state));
		if (!bgAnsi) return undefined;
		// Keep block background stable even if inner content contains SGR resets (e.g. "\x1b[0m"),
		// which would otherwise clear the outer background mid-line.
		return (text: string) => {
			const stabilized = text
				.replace(/\x1b\[(?:0)?m/g, m => `${m}${bgAnsi}`)
				.replace(/\x1b\[49m/g, m => `${m}${bgAnsi}`);
			return `${bgAnsi}${stabilized}\x1b[49m`;
		};
	})();

	const contentPaddingLeft = normalizeContentPaddingLeft(options.contentPaddingLeft);
	const contentPaddingRight = normalizeContentPaddingLeft(options.contentPaddingRight ?? contentPaddingLeft);
	const contentWidth = Math.max(
		0,
		lineWidth - visibleWidth(v) - contentPaddingLeft - contentPaddingRight - visibleWidth(v),
	);
	const contentLeftPadding = contentPaddingLeft > 0 ? padding(contentPaddingLeft) : "";
	const contentRightPadding = contentPaddingRight > 0 ? padding(contentPaddingRight) : "";

	// ── Layout pass: collect row descriptors before emitting the bordered lines. ──
	const rows: BlockRow[] = [];
	rows.push({
		kind: "bar",
		leftChar: theme.boxRound.topLeft,
		rightChar: theme.boxRound.topRight,
		label: header,
		meta: headerMeta,
	});

	const normalizedSections = sections.length > 0 ? sections : [{ lines: [] as string[] }];
	for (let sectionIndex = 0; sectionIndex < normalizedSections.length; sectionIndex++) {
		const section = normalizedSections[sectionIndex]!;
		// A labeled section always draws its titled separator bar. A label-less
		// section can still request a plain divider via `separator`, but only
		// between sections — leading with one would just double the header bar.
		if (section.label) {
			rows.push({
				kind: "bar",
				leftChar: theme.boxRound.teeRight,
				rightChar: theme.boxRound.teeLeft,
				label: section.label,
			});
		} else if (section.separator && sectionIndex > 0) {
			rows.push({
				kind: "bar",
				leftChar: theme.boxRound.teeRight,
				rightChar: theme.boxRound.teeLeft,
			});
		}
		const allLines = section.lines.flatMap(l => l.split("\n"));
		const sixelLineMask = TERMINAL.imageProtocol === ImageProtocol.Sixel ? getSixelLineMask(allLines) : undefined;
		for (let lineIndex = 0; lineIndex < allLines.length; lineIndex++) {
			const line = allLines[lineIndex]!;
			if (sixelLineMask?.[lineIndex]) {
				rows.push({ kind: "sixel", raw: line });
				continue;
			}
			const wrappedLines = wrapTextWithAnsi(line.trimEnd(), contentWidth);
			for (const wrappedLine of wrappedLines) {
				const innerPadding = padding(Math.max(0, contentWidth - visibleWidth(wrappedLine)));
				rows.push({ kind: "content", inner: `${wrappedLine}${innerPadding}` });
			}
		}
	}

	rows.push({ kind: "bottom", leftChar: theme.boxRound.bottomLeft, rightChar: theme.boxRound.bottomRight });

	const H = rows.length;

	const renderBar = (row: { leftChar: string; rightChar: string; label?: string; meta?: string }): string => {
		const leftGlyphs = `${row.leftChar}${cap}`;
		const rightGlyph = row.rightChar;
		if (lineWidth <= 0) return border(leftGlyphs) + border(rightGlyph);
		const labelText = [row.label, row.meta].filter(Boolean).join(theme.sep.dot);
		if (!labelText) {
			// No header: draw a clean, continuous top/separator bar (no 1-col gap).
			const fillCount = Math.max(0, lineWidth - visibleWidth(leftGlyphs) - visibleWidth(rightGlyph));
			return `${border(leftGlyphs)}${border(h.repeat(fillCount))}${border(rightGlyph)}`;
		}
		const rawLabel = ` ${labelText} `;
		const leftWidth = visibleWidth(leftGlyphs);
		const rightWidth = visibleWidth(rightGlyph);
		const maxLabelWidth = Math.max(0, lineWidth - leftWidth - rightWidth);
		const trimmedLabel = truncateToWidth(rawLabel, maxLabelWidth, ellipsis);
		const labelWidth = visibleWidth(trimmedLabel);
		const fillCount = Math.max(0, lineWidth - leftWidth - labelWidth - rightWidth);
		const fillGlyphs = h.repeat(fillCount);
		return `${border(leftGlyphs)}${trimmedLabel}${border(fillGlyphs)}${border(rightGlyph)}`;
	};

	const renderBottom = (row: { leftChar: string; rightChar: string }): string => {
		const leftGlyphs = `${row.leftChar}${cap}`;
		const rightGlyph = row.rightChar;
		const fillCount = Math.max(0, lineWidth - visibleWidth(leftGlyphs) - visibleWidth(rightGlyph));
		const fillGlyphs = h.repeat(fillCount);
		return `${border(leftGlyphs)}${border(fillGlyphs)}${border(rightGlyph)}`;
	};

	const renderContent = (inner: string): string =>
		`${border(v)}${contentLeftPadding}${inner}${contentRightPadding}${border(v)}`;

	const lines: string[] = [];
	for (let r = 0; r < H; r++) {
		const row = rows[r]!;
		if (row.kind === "sixel") {
			lines.push(row.raw);
			continue;
		}
		const line =
			row.kind === "bar" ? renderBar(row) : row.kind === "bottom" ? renderBottom(row) : renderContent(row.inner);
		lines.push(padToWidth(line, lineWidth, bgFn));
	}

	return lines;
}

/**
 * Quiet rail/panel variant: one accent rail on the left, content inset on a
 * panel surface, no enclosing rounded box. Successful simple tools occupy one
 * or two activity rows; output/diff/code panels keep the framed variant.
 * Resolves the rail glyph through the active symbol preset (ASCII-safe).
 */
export function renderRailPanel(options: OutputBlockOptions, theme: Theme): string[] {
	const { header, headerMeta, state, sections = [], width } = options;
	const lineWidth = Math.max(0, Math.floor(width));
	if (lineWidth === 0) return [];
	const accent: ThemeColor =
		options.borderColor ??
		(state === "error"
			? "error"
			: state === "warning"
				? "warning"
				: state === "running" || state === "pending"
					? "accent"
					: "borderMuted");
	const contentWidth = outputPanelContentWidth(lineWidth);
	const ellipsis = theme.getSymbolPreset() === "ascii" ? Ellipsis.Ascii : Ellipsis.Unicode;
	const out: string[] = [];
	const title = [header, headerMeta].filter(Boolean).join(theme.sep.dot);
	if (title) {
		for (const wrapped of wrapTextWithAnsi(title, contentWidth)) {
			out.push(theme.fg("toolTitle", truncateToWidth(wrapped, contentWidth, ellipsis)));
		}
	}
	for (const section of sections) {
		if (section.label) out.push(theme.fg("muted", truncateToWidth(section.label, contentWidth, ellipsis)));
		const lines = section.lines.flatMap(l => l.split("\n"));
		const sixelMask = getSixelLineMask(lines);
		for (let index = 0; index < lines.length; index++) {
			const line = lines[index]!;
			if (sixelMask[index]) {
				out.push(line);
				continue;
			}
			for (const wrapped of wrapTextWithAnsi(line.trimEnd(), contentWidth)) {
				out.push(truncateToWidth(wrapped, contentWidth, ellipsis));
			}
		}
	}
	if (out.length === 0) out.push("");
	return renderOutputPanelLines(out, lineWidth, theme, {
		accent,
		plain: options.variant === "plain",
		background: options.applyBg === false ? undefined : "panelBg",
	});
}

/** Body allocation shared by Markdown, execution previews and quiet output panels. */
export function outputPanelContentWidth(width: number): number {
	const safeWidth = Math.max(0, Math.floor(width));
	return safeWidth > 2 ? safeWidth - 2 : safeWidth;
}

/** Paint already laid-out content, preserving image protocol rows byte-for-byte. */
export function renderOutputPanelLines(
	lines: readonly string[],
	width: number,
	theme: Theme,
	options: { accent?: ThemeColor; background?: ThemeBg; plain?: boolean } = {},
): string[] {
	const safeWidth = Math.max(0, Math.floor(width));
	if (safeWidth === 0) return [];
	const glyph = theme.getSymbolPreset() === "ascii" ? "|" : "▎";
	const prefix = safeWidth > 2 ? (options.plain ? "  " : `${theme.fg(options.accent ?? "borderMuted", glyph)} `) : "";
	const contentWidth = outputPanelContentWidth(safeWidth);
	const sixelMask = getSixelLineMask(lines.slice());
	const ellipsis = theme.getSymbolPreset() === "ascii" ? Ellipsis.Ascii : Ellipsis.Unicode;
	return lines.map((line, index) => {
		if (sixelMask[index]) return line;
		const row = prefix + truncateToWidth(line, contentWidth, ellipsis);
		const background = options.background;
		return background
			? applyBackgroundToLine(row, safeWidth, text => theme.bgFill(background, text))
			: padToWidth(row, safeWidth);
	});
}

/** Shared notice/manual-output surface; child components retain their own layout and expansion. */
export class OutputPanel extends Container {
	#source?: readonly string[];
	#cachedTheme?: Theme;
	#cache?: { width: number; epoch: number; lines: readonly string[] };

	constructor(
		private readonly themeSource: Theme | (() => Theme),
		private accent: ThemeColor = "borderMuted",
		private readonly background: ThemeBg = "panelBg",
	) {
		super();
	}

	setAccent(accent: ThemeColor): void {
		if (this.accent === accent) return;
		this.accent = accent;
		this.#cache = undefined;
	}

	override invalidate(): void {
		super.invalidate();
		this.#cache = undefined;
	}

	override render(width: number): readonly string[] {
		const safeWidth = Math.max(0, Math.floor(width));
		if (safeWidth === 0) return [];
		const source = super.render(outputPanelContentWidth(safeWidth));
		const epoch = getThemeEpoch();
		const uiTheme = typeof this.themeSource === "function" ? this.themeSource() : this.themeSource;
		if (
			source === this.#source &&
			this.#cachedTheme === uiTheme &&
			this.#cache?.width === safeWidth &&
			this.#cache.epoch === epoch
		) {
			return this.#cache.lines;
		}
		const lines = renderOutputPanelLines(source, safeWidth, uiTheme, {
			accent: this.accent,
			background: this.background,
		});
		this.#source = source;
		this.#cachedTheme = uiTheme;
		this.#cache = { width: safeWidth, epoch, lines };
		return lines;
	}
}

/**
 * Compact inline activity row: `› label · detail` with state color, used for
 * successful simple tools (read/glob/grep/search) that should not each claim
 * a rounded frame. Expanded detail uses the rail/panel or framed variant.
 */
export function renderInlineActivity(
	theme: Theme,
	label: string,
	detail: string | undefined,
	state: State | undefined,
	width: number,
): string[] {
	const lineWidth = Math.max(0, Math.floor(width));
	if (lineWidth === 0) return [];
	const color: ThemeColor =
		state === "error"
			? "error"
			: state === "warning"
				? "warning"
				: state === "running" || state === "pending"
					? "accent"
					: "muted";
	const glyph =
		state === "error"
			? theme.status.error
			: state === "running" || state === "pending"
				? theme.status.running
				: theme.nav.cursor;
	const text = detail ? `${label}${theme.sep.dot}${detail}` : label;
	const prefix = lineWidth > visibleWidth(glyph) + 1 ? `${theme.fg(color, glyph)} ` : "";
	const rowText =
		prefix +
		theme.fg(
			"muted",
			truncateToWidth(
				text,
				lineWidth - visibleWidth(prefix),
				theme.getSymbolPreset() === "ascii" ? Ellipsis.Ascii : Ellipsis.Unicode,
			),
		);
	return [padToWidth(rowText, lineWidth)];
}

/**
 * Cached wrapper around `renderOutputBlock`.
 *
 * Since output blocks are re-rendered on every frame (via `render(width)` closures),
 * but their content rarely changes, this cache avoids redundant `visibleWidth()` and
 * `padding()` computations on ~99% of render calls.
 */
export class CachedOutputBlock {
	#cache?: RenderCache;
	#cachedTheme?: Theme;

	/** Render with caching. Returns the cached (shared, caller-immutable) lines if options haven't changed. */
	render(options: OutputBlockOptions, theme: Theme): readonly string[] {
		const key = this.#buildKey(options);
		if (this.#cachedTheme === theme && this.#cache?.key === key) return this.#cache.lines;
		const lines = renderOutputBlock(options, theme);
		this.#cache = { key, lines };
		this.#cachedTheme = theme;
		return lines;
	}

	/** Invalidate the cache, forcing a rebuild on next render. */
	invalidate(): void {
		this.#cache = undefined;
	}

	#buildKey(options: OutputBlockOptions): bigint {
		const h = new Hasher();
		h.u32(getThemeEpoch());
		h.u32(options.width);
		h.u32(normalizeContentPaddingLeft(options.contentPaddingLeft));
		h.u32(
			normalizeContentPaddingLeft(
				options.contentPaddingRight ?? normalizeContentPaddingLeft(options.contentPaddingLeft),
			),
		);
		h.optional(options.header);
		h.optional(options.headerMeta);
		h.optional(options.state);
		h.optional(options.borderColor);
		h.optional(options.variant);
		h.bool(options.applyBg ?? true);
		if (options.sections) {
			for (const s of options.sections) {
				h.optional(s.label);
				h.bool(s.separator ?? false);
				for (const line of s.lines) {
					h.str(line);
				}
			}
		}
		return h.digest();
	}
}

/**
 * Build a self-framing tool component backed by a cached output block. The
 * `build` callback returns the block options for a given width; the cache
 * dedupes re-renders. Pass `borderColor: "borderMuted"` for the dim "legacy"
 * look that does not compete with the state-colored framed tools.
 */
export function framedBlock(theme: Theme, build: (width: number) => OutputBlockOptions): Component {
	const block = new CachedOutputBlock();
	// Marked so the tool-execution container treats it as self-framing (renders
	// flush, no extra padding/background) the same way `markFramedBlockComponent`
	// blocks are treated.
	return markFramedBlockComponent({
		render: (width: number): readonly string[] => block.render(build(width), theme),
		invalidate: () => block.invalidate(),
	});
}
