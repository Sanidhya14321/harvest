/** Shared filled surfaces for transient dialogs and multi-pane managers.
 * Normal layouts retain the existing one-row title and two-column content
 * inset so their keyboard and mouse ownership remain unchanged. */
import {
	type Component,
	Container,
	Ellipsis,
	type Focusable,
	getKeybindings,
	Input,
	matchesKey,
	type MouseRoutable,
	type OverlayFocusOwner,
	padding,
	parseSgrMouse,
	replaceTabs,
	ScrollView,
	SelectList,
	SettingsList,
	type SgrMouseEvent,
	Spacer,
	Text,
	truncateToWidth,
	visibleWidth,
} from "@harvest/pi-tui";
import { getSymbolTheme, getThemeEpoch, type ThemeBg, type ThemeColor, theme } from "../theme/theme";
import { editorKey } from "./keybinding-hints";

/** Pad or truncate a (possibly ANSI-styled) string to exactly `width` columns. */
export function fit(text: string, width: number): string {
	if (width <= 0) return "";
	const w = visibleWidth(text);
	if (w === width) return text;
	if (w < width) return text + padding(width - w);
	const cut = truncateToWidth(text, width, theme.symbol("sep.ellipsis") === "..." ? Ellipsis.Ascii : Ellipsis.Unicode);
	const cw = visibleWidth(cut);
	return cw < width ? cut + padding(width - cw) : cut;
}

export function surfaceRow(content: string, width: number, background: ThemeBg = "modalBg"): string {
	return theme.bgFill(background, theme.fgOnBg("text", background, fit(content, Math.max(0, Math.floor(width)))));
}

/** Filled title row with the same content inset as the body. */
export function topBorder(width: number, title: string, color?: ThemeColor): string {
	return row(theme.bold(theme.fg(color ?? "accent", collapseTitle(title))), width);
}

/** Quiet space separating sections on the shared dialog surface. */
export function divider(width: number): string {
	return surfaceRow("", width);
}

export function bottomBorder(width: number, color?: ThemeColor): string {
	void color;
	return surfaceRow("", width);
}

/** Fill the assigned width while preserving pre-styled content and nested surfaces. */
export function dialogContentWidth(width: number): number {
	const safeWidth = Math.max(1, Math.floor(width));
	const inset = Math.min(2, Math.floor((safeWidth - 1) / 2));
	return safeWidth - inset * 2;
}

export function row(content: string, width: number, color?: ThemeColor, background: ThemeBg = "modalBg"): string {
	void color;
	const inset = Math.min(2, Math.floor(Math.max(0, width - 1) / 2));
	return surfaceRow(padding(inset) + fit(content, Math.max(0, width - inset * 2)) + padding(inset), width, background);
}

/** Secondary panes collapse before primary content becomes too narrow to use. */
export function canSplitPane(width: number, sidebarWidth: number, minimumBodyWidth = 24): boolean {
	return width >= sidebarWidth + 7 + minimumBodyWidth;
}

/** Body content width for a two-column overlay of total `width`. */
export function splitBodyWidth(width: number, sidebarWidth: number): number {
	return canSplitPane(width, sidebarWidth, 1) ? Math.max(0, width - sidebarWidth - 7) : Math.max(0, width - 4);
}

/** Top border carrying the title, split by a `┬` over the column divider. */
export function topBorderSplit(width: number, title: string, sidebarWidth: number): string {
	void sidebarWidth;
	return topBorder(width, title);
}

/** Section rule that closes the sidebar column with a `┴` over the divider. */
export function dividerSplit(width: number, sidebarWidth: number): string {
	void sidebarWidth;
	return divider(width);
}

/** A two-column content row: `│ sidebar │ body │`, each inset by one column. */
export function splitRow(
	sidebar: string,
	body: string,
	width: number,
	sidebarWidth: number,
	sidebarBackground: ThemeBg = "panelBg",
): string {
	if (!canSplitPane(width, sidebarWidth, 1)) return row(body || sidebar, width);
	const bodyWidth = splitBodyWidth(width, sidebarWidth);
	const left = surfaceRow(`  ${fit(sidebar, sidebarWidth)} `, sidebarWidth + 3, sidebarBackground);
	return surfaceRow(`${left}  ${fit(body, bodyWidth)}  `, width);
}

/** Clamp a dialog width to the viewport using the shared 60/88/116 caps. */
export function clampDialogWidth(columns: number, preferred: 60 | 88 | 116): number {
	const safe = Math.max(1, Math.floor(columns));
	return Math.max(1, Math.min(preferred, Math.max(1, safe - 2)));
}

/** Pick the largest dialog cap that fits the viewport. */
export function pickDialogCap(columns: number): 60 | 88 | 116 {
	if (columns >= 118) return 116;
	if (columns >= 90) return 88;
	return 60;
}

/** Standard dialog chrome: titled top border, padded body rows, bottom border. */
export function dialogChrome(title: string, body: readonly string[], width: number, color?: ThemeColor): string[] {
	const w = clampDialogWidth(width, pickDialogCap(width));
	return [topBorder(w, title, color), ...body.map(line => row(line, w, color)), bottomBorder(w, color)];
}

export interface DialogLayout {
	lines: string[];
	bodyRowStart: number;
	bodyWindowStart: number;
	bodyRows: number;
}

/** Allocate controls before optional chrome, keeping the active body row visible. */
export function renderDialog(
	title: string,
	body: readonly string[],
	width: number,
	height: number,
	footer = "",
	activeRow = 0,
): DialogLayout {
	const budget = Math.max(1, Math.floor(height));
	const showTitle = budget >= 3;
	const showFooter = footer.length > 0 && budget >= 2;
	const showBottom = budget >= 6;
	const bodyRows = Math.max(1, budget - Number(showTitle) - Number(showFooter) - Number(showBottom));
	const bodyWindowStart = Math.max(0, Math.min(activeRow - Math.floor(bodyRows / 2), body.length - bodyRows));
	const lines = showTitle ? [topBorder(width, title)] : [];
	const bodyRowStart = lines.length;
	for (const line of body.slice(bodyWindowStart, bodyWindowStart + bodyRows)) lines.push(row(line, width));
	if (showFooter) lines.push(row(theme.fg("dim", footer), width));
	if (showBottom) lines.push(bottomBorder(width));
	return { lines, bodyRowStart, bodyWindowStart, bodyRows: Math.min(bodyRows, body.length) };
}

/** The active choice stays visible; pointer coordinates refer to this rendered window. */
export function renderChoiceDialog(
	title: string,
	choices: readonly string[],
	selectedIndex: number,
	width: number,
	height: number,
	footer: string,
): DialogLayout {
	const layout = renderDialog(title, choices, width, height, footer, Math.max(0, selectedIndex));
	const selectedLine = selectedIndex - layout.bodyWindowStart;
	if (selectedLine >= 0 && selectedLine < layout.bodyRows) {
		layout.lines[layout.bodyRowStart + selectedLine] = row(
			choices[selectedIndex] ?? "",
			width,
			undefined,
			"selectedBg",
		);
	}
	return layout;
}

/** Sentinel child rendered by {@link OverlayPanel} as a `├───┤` section rule. */
export class PanelDivider implements Component {
	render(): readonly string[] {
		return [];
	}
}

const NO_LINES: readonly string[] = [];

interface OverlayPanelMemo {
	width: number;
	themeEpoch: number;
	title: string;
	children: Component[];
	childLines: (readonly string[])[];
	result: string[];
}

/** Titles inset into a single border row must never carry line breaks. */
function collapseTitle(title: string): string {
	return title.replace(/\s+/g, " ").trim();
}

/**
 * Rounded-box container for inline overlays (selectors, run panels). Children
 * render inside `│ … │` rows between a titled top border and a bottom border,
 * so inline overlays share the chrome of fullscreen overlays. The top border
 * is exactly one row — `routeMouse` offsets written for a one-line top rule
 * stay valid — and content is inset two columns on each side.
 */
export class OverlayPanel implements Component, OverlayFocusOwner {
	children: Component[] = [];
	#title: string;
	#memo: OverlayPanelMemo | undefined;
	#maxHeight: number | undefined;
	#contentRowStart = 1;
	#contentWindowStart = 0;

	constructor(title = "") {
		this.#title = collapseTitle(title);
	}

	/** Permit child controls to receive modal focus without opening the background focus fence. */
	ownsOverlayFocusTarget(component: Component): boolean {
		const pending = [...this.children];
		const visited = new Set<Component>();
		while (pending.length > 0) {
			const child = pending.pop()!;
			if (child === component) return true;
			if (visited.has(child)) continue;
			visited.add(child);
			if (child instanceof Container || child instanceof OverlayPanel) pending.push(...child.children);
			else if (child.debugChildren) pending.push(...child.debugChildren);
		}
		return false;
	}

	/** The mounting owner supplies the current allocated height on every resize. */
	setMaxHeight(height: number): void {
		const next = Math.max(1, Math.floor(height));
		if (this.#maxHeight === next) return;
		this.#maxHeight = next;
		this.#memo = undefined;
	}

	protected getMaxHeight(): number {
		return this.#maxHeight ?? Math.max(1, process.stdout.rows || 40);
	}

	get contentRowStart(): number {
		return this.#contentRowStart;
	}

	get contentWindowStart(): number {
		return this.#contentWindowStart;
	}

	get title(): string {
		return this.#title;
	}

	set title(value: string) {
		const next = collapseTitle(value);
		if (next === this.#title) return;
		this.#title = next;
		this.#memo = undefined;
	}

	addChild(component: Component): void {
		this.children.push(component);
		this.#memo = undefined;
	}

	removeChild(component: Component): void {
		const index = this.children.indexOf(component);
		if (index === -1) return;
		this.children.splice(index, 1);
		this.#memo = undefined;
	}

	clear(): void {
		this.children = [];
		this.#memo = undefined;
	}

	invalidate(): void {
		this.#memo = undefined;
		for (const child of this.children) child.invalidate?.();
	}

	dispose(): void {
		for (const child of this.children) child.dispose?.();
	}

	setIgnoreTight(ignore: boolean): this {
		for (const child of this.children) child.setIgnoreTight?.(ignore);
		return this;
	}

	/**
	 * Body rows at the given content width, without border chrome —
	 * {@link render} draws exactly these (4 columns narrower) inside `│ … │`
	 * rows. Lets callers and tests assert on component-content coordinates
	 * instead of reverse-parsing box glyphs. `PanelDivider` children contribute
	 * no rows here (their rule is border chrome).
	 */
	renderContent(width: number): string[] {
		const result: string[] = [];
		for (const child of this.children) {
			if (child instanceof PanelDivider) continue;
			result.push(...child.render(width));
		}
		return result;
	}

	render(width: number): readonly string[] {
		const innerWidth = dialogContentWidth(width);
		const contentChildren = this.children.filter(
			child => !(child instanceof Spacer || child instanceof PanelDivider),
		);
		const onlyChild = contentChildren.length === 1 ? contentChildren[0] : undefined;
		if (onlyChild instanceof SelectList) {
			const height = this.getMaxHeight();
			const chrome = Number(height >= 3) + Number(height >= 2) + Number(height >= 6);
			onlyChild.setMaxVisible(Math.max(1, height - chrome - 1));
			const body = onlyChild.render(innerWidth);
			const selectedIndex = onlyChild.debugState().selectedIndex;
			const activeRow = Math.max(
				0,
				body.findIndex((_line, index) => onlyChild.hitTest(index) === selectedIndex),
			);
			const footer = [`${editorKey("tui.select.cancel") || "Esc"} cancel`, "Enter select"].join(theme.sep.dot);
			const layout = renderDialog(this.#title, body, width, height, footer, activeRow);
			this.#contentRowStart = layout.bodyRowStart;
			this.#contentWindowStart = layout.bodyWindowStart;
			for (let index = 0; index < layout.bodyRows; index++) {
				const bodyIndex = layout.bodyWindowStart + index;
				if (onlyChild.hitTest(bodyIndex) === selectedIndex)
					layout.lines[layout.bodyRowStart + index] = row(body[bodyIndex] ?? "", width, undefined, "selectedBg");
			}
			return layout.lines;
		}
		this.#contentRowStart = 1;
		this.#contentWindowStart = 0;
		// Children render every frame (renders may carry side effects); the memo
		// only skips re-wrapping unchanged rows in border chrome.
		const childLines = this.children.map(child =>
			child instanceof PanelDivider ? NO_LINES : child.render(innerWidth),
		);
		const memo = this.#memo;
		const themeEpoch = getThemeEpoch();
		if (
			memo !== undefined &&
			memo.width === width &&
			memo.themeEpoch === themeEpoch &&
			memo.title === this.#title &&
			memo.children.length === this.children.length &&
			this.children.every((child, i) => memo.children[i] === child && memo.childLines[i] === childLines[i])
		) {
			return memo.result;
		}
		const result: string[] = [topBorder(width, this.#title)];
		for (let i = 0; i < this.children.length; i++) {
			if (this.children[i] instanceof PanelDivider) {
				result.push(divider(width));
				continue;
			}
			for (const line of childLines[i] ?? NO_LINES) result.push(row(line, width));
		}
		result.push(bottomBorder(width));
		this.#memo = { width, themeEpoch, title: this.#title, children: [...this.children], childLines, result };
		return result;
	}
}

/** Control-first dialog layout with an explicit, scrollable details view.
 * Input and nested setting callbacks continue to be owned by their original
 * components; the presentation layer only allocates and maps visible rows. */
export class InteractiveDialogPanel extends OverlayPanel implements Focusable {
	focused = true;
	#details = false;
	#scroll = new ScrollView([], { height: 1, scrollbar: "never" });
	#control: Component | undefined;
	#controlOffset = 0;
	#nested: Component | undefined;
	#bodyRowStart = 0;
	#bodyRows = 0;
	#bodyWindowStart = 0;
	#inset = 0;

	protected dialogFooter(): string {
		return [`${editorKey("tui.select.cancel") || "Esc"} back`, "Enter continue", "F2 details"].join(theme.sep.dot);
	}

	protected dialogControl(): Component | undefined {
		return this.#parts().find(
			child => child instanceof Input || child instanceof SelectList || child instanceof SettingsList,
		);
	}

	protected resetDialogDetails(): void {
		this.#details = false;
		this.#scroll.scrollToTop();
	}

	#parts(children: readonly Component[] = this.children): Component[] {
		return children.flatMap(child =>
			child instanceof Spacer ? [] : child instanceof Container ? this.#parts(child.children) : [child],
		);
	}

	/** Call before the owner's normal keyboard dispatch; cancel still belongs to the owner. */
	protected handleDialogInput(data: string): boolean {
		if (getKeybindings().matches(data, "tui.select.cancel") || data === "\x1b\x1b") return false;
		const control = this.dialogControl();
		if (control instanceof SettingsList && control.hasOpenSubmenu() && !parseSgrMouse(data)) return false;
		if (matchesKey(data, "f2")) {
			this.#details = !this.#details;
			return true;
		}
		const mouse = parseSgrMouse(data);
		if (mouse) {
			this.routeMouse(mouse, mouse.row, mouse.col);
			return true;
		}
		if (!this.#details) return !control && this.#scroll.handleScrollKey(data);
		if (matchesKey(data, "enter") || data === "\n") this.#details = false;
		else this.#scroll.handleScrollKey(data);
		return true;
	}

	routeMouse(event: SgrMouseEvent, line: number, col: number): void {
		if (this.#control !== this.dialogControl()) return;
		if (this.#details || !this.#control) {
			if (event.wheel !== null) this.#scroll.scroll(event.wheel);
			return;
		}
		if (this.#nested) {
			(this.#nested as Component & Partial<MouseRoutable>).routeMouse?.(event, line, col);
			return;
		}
		const local = line - this.#bodyRowStart;
		if (local < 0 || local >= this.#bodyRows) return;
		const controlLine = local + this.#bodyWindowStart - this.#controlOffset;
		const controlCol = Math.max(0, col - this.#inset);
		const control = this.#control;
		if (control instanceof SelectList) control.routeMouse(event, controlLine, controlCol);
		else if (control instanceof SettingsList) {
			if (event.wheel !== null) control.handleWheelAt(event.wheel, controlLine, controlCol);
			else if (event.motion) control.setHoverItem(control.hoverTest(controlLine, controlCol) ?? null);
			else if (event.leftClick) {
				const id = control.hitTest(controlLine, controlCol);
				if (id && control.selectItem(id)) control.handleInput("\r");
			}
		}
	}

	override render(width: number): readonly string[] {
		const height = this.getMaxHeight();
		const innerWidth = dialogContentWidth(width);
		this.#inset = Math.floor((width - innerWidth) / 2);
		const footer = this.#details
			? [`${editorKey("tui.select.cancel") || "Esc"} back`, "F2 controls", "PgUp/PgDn scroll"].join(theme.sep.dot)
			: this.dialogFooter();
		const chrome = Number(height >= 3) + Number(height >= 2) + Number(height >= 6);
		const budget = Math.max(1, height - chrome);
		const control = this.dialogControl();
		this.#control = control;
		this.#nested = undefined;
		if (control instanceof Input) {
			control.prompt = innerWidth >= 3 ? "> " : "";
			control.focused = this.focused && !this.#details;
		}
		if (control instanceof SelectList) control.setMaxVisible(Math.max(1, budget - 1));
		if (control instanceof SettingsList) {
			const count = control.debugState().filteredItemCount;
			control.setMaxVisible(Math.max(1, Math.min(typeof count === "number" ? count : budget, budget - 4)));
			const nested = control.debugChildren[0];
			if (nested && !this.#details) {
				this.#nested = nested;
				nested.setMaxHeight?.(height);
				if ("focused" in nested) (nested as Component & Focusable).focused = this.focused;
				return nested.render(width);
			}
		}
		const parts = this.#parts();
		const body: string[] = [];
		let controlLines: readonly string[] = [];
		for (const part of parts) {
			if (part instanceof Input && part !== control) part.focused = false;
			const lines =
				part instanceof Text
					? Bun.wrapAnsi(replaceTabs(part.getText()), innerWidth, {
							hard: true,
							wordWrap: false,
							trim: false,
						}).split("\n")
					: part.render(innerWidth);
			if (part === control) {
				this.#controlOffset = body.length;
				controlLines = lines;
			}
			body.push(...lines);
		}
		let active = this.#controlOffset;
		const selectedId = control instanceof SettingsList ? control.getSelectedItem()?.id : undefined;
		const selectedIndex = control instanceof SelectList ? control.debugState().selectedIndex : undefined;
		const selectedLine = controlLines.findIndex((_line, index) =>
			control instanceof SettingsList
				? control.hitTest(index, innerWidth - 1) === selectedId && selectedId !== undefined
				: control instanceof SelectList
					? control.hitTest(index) === selectedIndex
					: index === 0,
		);
		active += Math.max(0, selectedLine);
		let display: readonly string[] = body;
		if (this.#details || !control) {
			this.#scroll.setSymbols(getSymbolTheme());
			this.#scroll.setLines(body);
			this.#scroll.setHeight(budget);
			display = this.#scroll.render(innerWidth);
			active = 0;
		} else if (body.length > budget) {
			display = [...controlLines];
			this.#controlOffset = 0;
			active = Math.max(0, selectedLine);
		}
		const layout = renderDialog(this.title, display, width, height, footer, active);
		this.#bodyRowStart = layout.bodyRowStart;
		this.#bodyRows = layout.bodyRows;
		this.#bodyWindowStart = layout.bodyWindowStart;
		if (!this.#details && (control instanceof SelectList || control instanceof SettingsList)) {
			for (let index = 0; index < layout.bodyRows; index++) {
				const displayIndex = layout.bodyWindowStart + index;
				const controlIndex = displayIndex - this.#controlOffset;
				const selected =
					control instanceof SelectList
						? control.hitTest(controlIndex) === selectedIndex
						: selectedId !== undefined && control.hitTest(controlIndex, innerWidth - 1) === selectedId;
				if (selected)
					layout.lines[layout.bodyRowStart + index] = row(
						display[displayIndex] ?? "",
						width,
						undefined,
						"selectedBg",
					);
			}
		}
		return layout.lines;
	}
}
