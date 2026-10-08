/**
 * Shared rendering for extension/hook custom message frames.
 *
 * Both `CustomMessageComponent` and `HookMessageComponent` use a quiet
 * output panel, try a user-supplied renderer first, and fall
 * back to a label + markdown body when the renderer returns nothing or
 * throws. The only meaningful difference is that hook messages collapse to
 * the first N lines when not expanded; extension messages render in full.
 */

import type { TextContent } from "@harvest/pi-ai";
import type { Component, Container } from "@harvest/pi-tui";
import { Markdown, Spacer, Text } from "@harvest/pi-tui";
import { getMarkdownTheme, type Theme, type ThemeColor, theme } from "../../modes/theme/theme";
import { OutputPanel } from "../../tui/output-block";

/** Message shape consumed by the shared frame. */
export interface FramedMessage {
	customType: string;
	content: string | (TextContent | { type: string })[];
}

/**
 * Callable signature shared by `MessageRenderer` (extensions) and
 * `HookMessageRenderer` (hooks). Both narrow `message` to their own type;
 * this signature is the structural intersection callers can hand off here.
 */
export type FramedRenderer<M extends FramedMessage> = (
	message: M,
	options: { expanded: boolean },
	theme: Theme,
) => Component | undefined;

export interface RebuildFrameOptions<M extends FramedMessage> {
	message: M;
	box: Container;
	expanded: boolean;
	/** Icon glyph shown before the customType in the default header (e.g. a hook/extension icon). */
	icon?: string;
	/** Hide the default type header while retaining the message body. */
	hideHeader?: boolean;
	/** Semantic color for the notice rail, defaulting to the muted border. */
	borderColor?: ThemeColor;
	/** Collapse the markdown body to this many lines when `expanded` is false. Omit to never collapse. */
	collapseAfterLines?: number;
	customRenderer?: FramedRenderer<M>;
}

/**
 * Attempt the custom renderer; on failure or undefined return, populate `box`
 * with the configured rail, optional type header, and markdown body. When
 * the custom renderer succeeds, return its Component so the caller can mount
 * it and skip the default box.
 */
export function renderFramedMessage<M extends FramedMessage>(opts: RebuildFrameOptions<M>): Component | undefined {
	if (opts.customRenderer) {
		try {
			const component = opts.customRenderer(opts.message, { expanded: opts.expanded }, theme);
			if (component) return component;
		} catch {
			// Fall through to default rendering
		}
	}

	opts.box.clear();
	if (opts.box instanceof OutputPanel) opts.box.setAccent(opts.borderColor ?? "borderMuted");
	if (!opts.hideHeader) {
		const tag = opts.icon ? `${opts.icon} ${opts.message.customType}` : opts.message.customType;
		opts.box.addChild(new Text(theme.fg("customMessageLabel", theme.bold(tag)), 0, 0));
		opts.box.addChild(new Spacer(1));
	}

	let text: string;
	if (typeof opts.message.content === "string") {
		text = opts.message.content;
	} else {
		text = opts.message.content
			.filter((c): c is TextContent => c.type === "text")
			.map(c => c.text)
			.join("\n");
	}

	if (!opts.expanded && opts.collapseAfterLines !== undefined) {
		const lines = text.split("\n");
		if (lines.length > opts.collapseAfterLines) {
			text = `${lines.slice(0, opts.collapseAfterLines).join("\n")}\n${theme.symbol("sep.ellipsis")} (ctrl+o to expand)`;
		}
	}

	opts.box.addChild(
		new Markdown(text, 0, 0, getMarkdownTheme(), {
			color: (value: string) => theme.fg("customMessageText", value),
		}),
	);

	return undefined;
}
