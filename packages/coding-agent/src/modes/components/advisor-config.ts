/**
 * Fullscreen `/advisor configure` overlay: a mouse- and keyboard-driven editor
 * for the `WATCHDOG.yml` advisor roster at project or user level.
 *
 * It paints the entire alternate screen from row 0 (so SGR mouse rows index
 * directly into the rendered frame) using the shared {@link ./overlay-box} chrome.
 * The list screen is a two-pane split (the `/extensions` idiom): a clickable
 * advisor/action sidebar on the left, and a scrollable preview of the highlighted
 * advisor's model / tools / instructions on the right, filling the free space.
 *
 * Each screen is backed by a proven primitive — {@link SelectList} (list / detail
 * / tools / thinking), {@link Input} (name), {@link ModelSelectorComponent} (the
 * same rich `/model` picker, in direct-select mode), and {@link HookEditorComponent}
 * (multiline instructions; Ctrl+G opens `$EDITOR`). The overlay edits an in-memory
 * {@link WatchdogConfigDoc} and only touches disk + the live advisors via the host
 * `save` callback.
 */
import type { ThinkingLevel } from "@harvest/pi-agent-core";
import type { Model, UsageReport } from "@harvest/pi-ai";
import { getSupportedEfforts } from "@harvest/pi-catalog/model-thinking";
import {
	type Component,
	Ellipsis,
	Input,
	matchesKey,
	type MouseRoutable,
	routeSgrMouseInput,
	type SelectItem,
	SelectList,
	type SgrMouseEvent,
	type TUI,
	truncateToWidth,
} from "@harvest/pi-tui";
import {
	ADVISOR_DEFAULT_TOOL_NAMES,
	type AdvisorConfig,
	type AdvisorConfigScope,
	type WatchdogConfigDoc,
} from "../../advisor";
import type { ModelRegistry } from "../../config/model-registry";
import { formatModelSelectorValue } from "../../config/model-resolver";
import type { Settings } from "../../config/settings";
import type { PerAdvisorStat } from "../../session/agent-session";
import type { OAuthAccountIdentity } from "../../session/auth-storage";
import { formatCompactQuota } from "../controllers/command-controller";
import { getSelectListTheme, getThemeEpoch, theme } from "../theme/theme";
import { matchesSelectDown, matchesSelectUp } from "../utils/keybinding-matchers";
import { editorKey } from "./keybinding-hints";
import { HookEditorComponent } from "./hook-editor";
import { buildBrowserItems, ModelBrowser, sortModelItems } from "./model-browser";
import { canSplitPane, dialogContentWidth, renderDialog, splitBodyWidth, splitRow, surfaceRow } from "./overlay-box";

/** Host callbacks: all disk + live-runtime effects flow through these. */
export interface AdvisorConfigCallbacks {
	/** Load a scope's `WATCHDOG.yml` into an editable doc (empty when absent). */
	loadDoc: (scope: AdvisorConfigScope) => Promise<WatchdogConfigDoc>;
	/** Persist the doc to the scope's file and rebuild the live advisors. */
	save: (scope: AdvisorConfigScope, doc: WatchdogConfigDoc) => Promise<void>;
	/** Tear down the overlay and restore the editor. */
	close: () => void;
	requestRender: () => void;
	/** Surface a transient status/warning line to the user. */
	notify: (message: string) => void;
	/** Live advisor usage stats; lets the preview show tokens/cost per advisor. */
	getAdvisorStats?: () => PerAdvisorStat[];
	getUsageReports?: () => Promise<UsageReport[] | null>;
	/** Resolve the active OAuth identity for quota filtering (per-advisor account stickiness). */
	resolveActiveAccount?: (provider: string, sessionId?: string) => OAuthAccountIdentity | undefined;
}

export interface AdvisorConfigDeps {
	modelRegistry: ModelRegistry;
	settings: Settings;
	scopedModels: ReadonlyArray<{ model: Model; thinkingLevel?: ThinkingLevel }>;
	availableToolNames: string[];
	/** Formatted advisor-role model shown on the seeded default row (e.g. "anthropic/claude-..."). */
	defaultModelLabel?: string;
}

const PREVIEW_WIDTH = 60;

function previewLine(text: string | undefined): string {
	if (!text?.trim()) return "(none)";
	const first = text.trim().split("\n", 1)[0] ?? "";
	return truncateToWidth(
		first,
		PREVIEW_WIDTH,
		theme.getSymbolPreset() === "ascii" ? Ellipsis.Ascii : Ellipsis.Unicode,
	);
}

/** Omitted means default read/grep/glob; an explicit empty set means no tools. */
function commitTools(selected: ReadonlySet<string>, all: readonly string[]): string[] | undefined {
	if (selected.size === 0) return [];
	if (selected.size === ADVISOR_DEFAULT_TOOL_NAMES.size) {
		let matchesDefault = true;
		for (const name of ADVISOR_DEFAULT_TOOL_NAMES) {
			if (!selected.has(name)) {
				matchesDefault = false;
				break;
			}
		}
		if (matchesDefault) return undefined;
	}
	return all.filter(name => selected.has(name));
}

function formatAdvisorTools(tools: readonly string[] | undefined, emptyLabel: string): string {
	if (tools === undefined) return "read, grep, glob (default)";
	return tools.length > 0 ? tools.join(", ") : emptyLabel;
}

/** Soft-wrap plain text to `width`, returning at least one (possibly empty) line. */
function wrap(text: string, width: number): string[] {
	if (!text) return [""];
	return Bun.wrapAnsi(text, Math.max(1, width), { trim: false, hard: true }).split("\n");
}

type Screen = "list" | "detail" | "name" | "model" | "tools" | "thinking" | "instructions";

/**
 * Fullscreen advisor-configuration overlay. Implements {@link Component} directly
 * (rather than extending Container) so it owns the whole frame and the mouse
 * geometry needed to make every row clickable.
 */
export class AdvisorConfigOverlayComponent implements Component {
	#tui: TUI;
	#modelRegistry: ModelRegistry;
	#settings: Settings;
	#scopedModels: ReadonlyArray<{ model: Model; thinkingLevel?: ThinkingLevel }>;
	#availableToolNames: readonly string[];
	#defaultModelLabel: string | undefined;
	#cb: AdvisorConfigCallbacks;
	#scope: AdvisorConfigScope;
	#doc: WatchdogConfigDoc;
	/** Cached usage reports (quota/window/reset) prefetched on overlay open. */
	#cachedReports: UsageReport[] | null = null;
	#dirty = false;

	#screen: Screen = "list";
	/** The interactive element for the current screen. */
	#active: Component = new SelectList([], 1, getSelectListTheme());
	#footerHint = "";
	#previewScroll = 0;

	// Frame geometry from the last render (the frame paints from screen row 0,
	// so SGR `event.row`/`event.col` — already 0-based — index it directly).
	#bodyRowStart = 0;
	#dividerCol = 0;
	#maxHeight: number | undefined;
	#splitLayout = true;
	#previewOnly = false;
	#themeEpoch = getThemeEpoch();
	#detailIndex = 0;

	constructor(
		tui: TUI,
		deps: AdvisorConfigDeps,
		scope: AdvisorConfigScope,
		doc: WatchdogConfigDoc,
		callbacks: AdvisorConfigCallbacks,
	) {
		this.#tui = tui;
		this.#modelRegistry = deps.modelRegistry;
		this.#settings = deps.settings;
		this.#scopedModels = deps.scopedModels;
		this.#availableToolNames = deps.availableToolNames;
		this.#defaultModelLabel = deps.defaultModelLabel;
		this.#cb = callbacks;
		this.#scope = scope;
		this.#doc = doc;
		this.#ensureRosterVisible();
		this.#showList();
		// Prefetch usage reports for quota display; non-fatal if unavailable.
		if (callbacks.getUsageReports) {
			void callbacks
				.getUsageReports()
				.then(r => {
					this.#cachedReports = r;
					this.#cb.requestRender();
				})
				.catch(() => {});
		}
	}

	// ───────────────────────────── render ─────────────────────────────

	render(width: number): readonly string[] {
		if (this.#themeEpoch !== getThemeEpoch()) {
			this.#themeEpoch = getThemeEpoch();
			const previousIndex = this.#active instanceof SelectList ? this.#active.debugState().selectedIndex : 0;
			const selectedIndex = typeof previousIndex === "number" ? previousIndex : 0;
			if (this.#screen === "list") this.#showList();
			else if (this.#screen === "detail") this.#showDetail(this.#detailIndex);
			if (this.#active instanceof SelectList) this.#active.setSelectedIndex(selectedIndex);
		}
		const height = Math.max(1, this.#maxHeight ?? this.#tui.terminal?.rows ?? process.stdout.rows ?? 40);
		const chrome = Number(height >= 3) + Number(height >= 2) + Number(height >= 6);
		const bodyRows = Math.max(1, height - chrome);
		const sidebarWidth = Math.max(22, Math.min(42, Math.floor(width * 0.34)));
		this.#splitLayout = this.#screen === "list" && canSplitPane(width, sidebarWidth);
		this.#dividerCol = this.#splitLayout ? sidebarWidth + 3 : Number.POSITIVE_INFINITY;
		const bodyWidth = this.#splitLayout ? splitBodyWidth(width, sidebarWidth) : dialogContentWidth(width);
		if (this.#active instanceof SelectList) this.#active.setMaxVisible(Math.max(1, bodyRows - 1));
		if (this.#active instanceof ModelBrowser) {
			this.#active.setMaxVisible(bodyRows);
			this.#active.setMaxHeight(bodyRows);
		}
		if (this.#active instanceof HookEditorComponent) {
			this.#active.setMaxHeight(bodyRows);
			this.#active.focused = true;
		}
		if (this.#active instanceof Input) this.#active.focused = true;
		const active =
			this.#active instanceof HookEditorComponent
				? this.#active.renderContent(bodyWidth)
				: this.#active.render(this.#splitLayout ? sidebarWidth : bodyWidth);
		const preview =
			this.#screen === "list" && (this.#splitLayout || this.#previewOnly)
				? this.#previewWindow(bodyWidth, bodyRows)
				: [];
		const body = this.#splitLayout
			? Array.from({ length: bodyRows }, (_, index) =>
					splitRow(active[index] ?? "", preview[index] ?? "", width, sidebarWidth),
				)
			: (this.#previewOnly && this.#screen === "list" ? preview : active).slice(0, bodyRows);
		const title = `Advisor configuration${theme.sep.dot}${this.#scope}${this.#dirty ? `${theme.sep.dot}unsaved` : ""}`;
		const footer = [
			`${editorKey("tui.select.cancel") || "Esc"} back`,
			...(this.#screen === "list" && !this.#splitLayout ? [`F2 ${this.#previewOnly ? "list" : "preview"}`] : []),
			this.#footerHint,
		]
			.filter(Boolean)
			.join(theme.sep.dot);
		const layout = renderDialog(title, body, width, height, footer);
		if (this.#splitLayout)
			for (let index = 0; index < layout.bodyRows; index++)
				layout.lines[layout.bodyRowStart + index] = surfaceRow(body[index] ?? "", width);
		this.#bodyRowStart = layout.bodyRowStart;
		return layout.lines;
	}

	setMaxHeight(height: number): void {
		const next = Math.max(1, Math.floor(height));
		if (this.#maxHeight === next) return;
		this.#maxHeight = next;
	}

	// ───────────────────────────── input ─────────────────────────────

	handleInput(data: string): void {
		if (this.#screen === "list" && !this.#splitLayout && matchesKey(data, "f2")) {
			this.#previewOnly = !this.#previewOnly;
			this.#cb.requestRender();
			return;
		}
		if (
			this.#screen === "list" &&
			!this.#splitLayout &&
			this.#previewOnly &&
			(matchesSelectUp(data) || matchesSelectDown(data))
		) {
			this.#previewScroll = Math.max(0, this.#previewScroll + (matchesSelectDown(data) ? 1 : -1));
			this.#cb.requestRender();
			return;
		}
		if (data.startsWith("\x1b[<")) {
			routeSgrMouseInput(data, event => this.#routeMouseEvent(event));
			return;
		}
		this.#active.handleInput?.(data);
	}

	/** Forward enhanced-paste transports into a multiline instructions editor. */
	pasteText(text: string): void {
		if (this.#active instanceof HookEditorComponent) this.#active.pasteText(text);
	}

	#routeMouseEvent(event: SgrMouseEvent): boolean {
		// Right pane of the split (the preview) only scrolls; everything left of the
		// divider routes into the active list/component at frame-local coordinates.
		if (this.#screen === "list" && (event.col >= this.#dividerCol || (!this.#splitLayout && this.#previewOnly))) {
			if (event.wheel !== null) {
				this.#previewScroll = Math.max(0, this.#previewScroll + event.wheel);
				this.#cb.requestRender();
			}
			return true;
		}
		const el = this.#active as Partial<MouseRoutable>;
		if (typeof el.routeMouse === "function") {
			el.routeMouse(event, event.row - this.#bodyRowStart, event.col);
			return true;
		}
		return false;
	}

	// ───────────────────────────── preview ───────────────────────────

	#previewWindow(bodyWidth: number, rows: number): string[] {
		const lines = this.#previewContent(bodyWidth);
		const maxScroll = Math.max(0, lines.length - rows);
		const start = Math.min(this.#previewScroll, maxScroll);
		const window = lines.slice(start, start + rows);
		if (lines.length > rows) {
			const marker =
				start + rows < lines.length
					? theme.fg("dim", `  ${lines.length - rows - start} more`)
					: theme.fg("dim", "  (end)");
			window[rows - 1] = marker;
		}
		return window;
	}

	#previewContent(bodyWidth: number): string[] {
		const list = this.#active;
		const value = list instanceof SelectList ? (list.getSelectedItem()?.value ?? "") : "";
		const match = /^advisor:(\d+)$/.exec(value);
		if (match) {
			const advisor = this.#doc.advisors[Number(match[1])];
			if (advisor) return this.#advisorPreview(advisor, bodyWidth);
		}
		if (value === "shared") {
			const lines = [theme.bold("Shared instructions"), ""];
			const text = this.#doc.instructions?.trim();
			lines.push(...(text ? wrap(text, bodyWidth) : [theme.fg("muted", "(none)")]));
			return lines.map(line => truncateToWidth(line, bodyWidth));
		}
		const help =
			value === "add"
				? "Create a new advisor entry, then edit its model, tools, and instructions."
				: value === "scope"
					? `Switch between the project and user WATCHDOG.yml. Currently editing the ${this.#scope}-level file.`
					: value === "save"
						? "Write this scope's WATCHDOG.yml and reload the live advisors without a restart."
						: value === "close"
							? "Close the editor. Unsaved changes are discarded."
							: "";
		return wrap(help, bodyWidth).map(line => truncateToWidth(theme.fg("muted", line), bodyWidth));
	}

	#advisorPreview(advisor: AdvisorConfig, bodyWidth: number): string[] {
		const model = advisor.model?.trim() || this.#defaultModelLabel || "advisor role default";
		const tools = formatAdvisorTools(advisor.tools, "no tools");
		const lines = [
			theme.bold(advisor.name || "(unnamed)"),
			"",
			`${theme.fg("dim", "Enabled:")} ${advisor.enabled === false ? `${theme.status.disabled} off` : `${theme.status.enabled} on`}`,
			`${theme.fg("dim", "Model:")} ${model}`,
			`${theme.fg("dim", "Tools:")} ${tools}`,
			"",
			theme.fg("dim", "Instructions:"),
		];
		const instr = advisor.instructions?.trim();
		lines.push(...(instr ? wrap(instr, bodyWidth) : [theme.fg("muted", "(none)")]));
		// Show live usage stats when available from the session.
		const liveStat = this.#cb.getAdvisorStats?.()?.find(s => s.name === (advisor.name || "default"));
		if (liveStat && (liveStat.status === "running" || liveStat.status === "quota_exhausted")) {
			lines.push("", theme.fg("dim", "Usage:"));
			const spendParts: string[] = [
				`${liveStat.tokens.input.toLocaleString()} in`,
				`${liveStat.tokens.output.toLocaleString()} out`,
			];
			if (liveStat.tokens.cacheRead > 0) spendParts.push(`${liveStat.tokens.cacheRead.toLocaleString()} cache`);
			lines.push(theme.fg("dim", `  Tokens: ${spendParts.join(", ")}`));
			if (liveStat.cost > 0) lines.push(theme.fg("dim", `  Cost: $${liveStat.cost.toFixed(4)}`));
			if (liveStat.contextWindow > 0) {
				const pct = Math.round((liveStat.contextTokens / liveStat.contextWindow) * 100);
				lines.push(
					theme.fg(
						"dim",
						`  Context: ${liveStat.contextTokens.toLocaleString()}/${liveStat.contextWindow.toLocaleString()} (${pct}%)`,
					),
				);
			}
		}
		const quotaProvider =
			(advisor.model?.includes("/") ? advisor.model.split("/")[0] : null) ?? liveStat?.model?.provider;
		if (this.#cachedReports && quotaProvider) {
			const activeAccount = this.#cb.resolveActiveAccount?.(quotaProvider, liveStat?.sessionId);
			const quota = formatCompactQuota(quotaProvider, this.#cachedReports, Date.now(), activeAccount);
			if (quota) lines.push(theme.fg("dim", `  ${quota}`));
		}
		return lines.map(line => truncateToWidth(line, bodyWidth));
	}

	// ───────────────────────────── screens ───────────────────────────

	#setScreen(screen: Screen, active: Component, footerHint: string): void {
		this.#screen = screen;
		this.#active = active;
		this.#footerHint = footerHint;
		this.#previewScroll = 0;
		this.#cb.requestRender();
	}

	#otherScope(): AdvisorConfigScope {
		return this.#scope === "project" ? "user" : "project";
	}

	#ensureRosterVisible(): void {
		if (this.#doc.advisors.length === 0) this.#doc.advisors.push({ name: "default" });
	}

	#isBareDefaultDoc(doc: WatchdogConfigDoc): boolean {
		if (doc.advisors.length !== 1 || doc.instructions?.trim()) return false;
		const advisor = doc.advisors[0];
		if (!advisor) return false;
		return (
			advisor.name === "default" &&
			!advisor.model?.trim() &&
			advisor.tools === undefined &&
			!advisor.instructions?.trim() &&
			advisor.enabled !== false
		);
	}

	#advisorSummary(advisor: AdvisorConfig): string {
		const model = advisor.model?.trim() || this.#defaultModelLabel || "advisor role default";
		const tools = formatAdvisorTools(advisor.tools, "no tools");
		return `${model}${theme.sep.dot}${tools}`;
	}

	#showList(): void {
		this.#ensureRosterVisible();
		const items: SelectItem[] = this.#doc.advisors.map((advisor, index) => ({
			value: `advisor:${index}`,
			label: `${advisor.enabled === false ? theme.status.disabled : theme.status.enabled} ${advisor.name || "(unnamed)"}`,
			description: this.#advisorSummary(advisor),
		}));
		items.push({ value: "add", label: "+ Add advisor" });
		items.push({ value: "shared", label: "Shared instructions", description: previewLine(this.#doc.instructions) });
		items.push({ value: "scope", label: `Scope: ${this.#scope}`, description: `Switch to ${this.#otherScope()}` });
		items.push({ value: "save", label: "Save & apply" });
		items.push({ value: "close", label: "Close" });

		// Show every row (no internal overflow-search); the split frame supplies height.
		const list = new SelectList(items, Math.max(1, items.length), getSelectListTheme());
		list.onSelectionChange = () => {
			this.#previewScroll = 0;
			this.#cb.requestRender();
		};
		list.onSelect = item =>
			void this.#onListSelect(item.value).catch(err => {
				this.#cb.notify(`Advisor config: ${err instanceof Error ? err.message : String(err)}`);
			});
		list.onCancel = () => this.#cb.close();
		this.#setScreen("list", list, "Up/Down move / Enter select / scroll preview / Esc close");
	}

	async #onListSelect(value: string): Promise<void> {
		if (value === "add") {
			this.#doc.advisors.push({ name: `Advisor ${this.#doc.advisors.length + 1}` });
			this.#dirty = true;
			this.#showDetail(this.#doc.advisors.length - 1);
			return;
		}
		if (value === "shared") {
			this.#showInstructionsEditor(-1);
			return;
		}
		if (value === "scope") {
			if (this.#dirty) {
				this.#cb.notify('Unsaved changes: "Save & apply" or Close before switching scope.');
				return;
			}
			const next = this.#otherScope();
			this.#doc = await this.#cb.loadDoc(next);
			this.#ensureRosterVisible();
			this.#scope = next;
			this.#showList();
			return;
		}
		if (value === "save") {
			await this.#cb.save(this.#scope, this.#isBareDefaultDoc(this.#doc) ? { advisors: [] } : this.#doc);
			this.#dirty = false;
			this.#showList();
			return;
		}
		if (value === "close") {
			this.#cb.close();
			return;
		}
		const match = /^advisor:(\d+)$/.exec(value);
		if (match) this.#showDetail(Number(match[1]));
	}

	#showDetail(index: number): void {
		this.#detailIndex = index;
		const advisor = this.#doc.advisors[index];
		if (!advisor) {
			this.#showList();
			return;
		}
		const modelDescription = advisor.model?.trim() || this.#defaultModelLabel || "advisor role default";
		const toolsDescription = formatAdvisorTools(advisor.tools, "no tools");
		const items: SelectItem[] = [
			{ value: "name", label: "Name", description: advisor.name },
			{
				value: "toggleEnabled",
				label: "Enabled",
				description: advisor.enabled === false ? `${theme.status.disabled} off` : `${theme.status.enabled} on`,
			},
			{ value: "model", label: "Model", description: modelDescription },
		];
		if (advisor.model?.trim()) {
			items.push({ value: "resetModel", label: "Reset model to advisor-role default" });
		}
		items.push(
			{ value: "tools", label: "Tools", description: toolsDescription },
			{ value: "instructions", label: "Instructions", description: previewLine(advisor.instructions) },
			{ value: "delete", label: "Delete this advisor" },
			{ value: "back", label: "Back" },
		);
		const list = new SelectList(items, Math.max(1, items.length), getSelectListTheme());
		list.onSelect = item => this.#onDetailSelect(index, item.value);
		list.onCancel = () => this.#showList();
		this.#setScreen("detail", list, `Editing "${advisor.name}" / Enter edit field / Esc back`);
	}

	#onDetailSelect(index: number, field: string): void {
		switch (field) {
			case "toggleEnabled": {
				const a = this.#doc.advisors[index];
				a.enabled = a.enabled === false ? undefined : false;
				this.#dirty = true;
				this.#showDetail(index);
				return;
			}
			case "name":
				this.#showNameEditor(index);
				return;
			case "model":
				this.#showModelPicker(index);
				return;
			case "tools":
				this.#showToolsEditor(
					index,
					new Set(this.#doc.advisors[index].tools ?? [...ADVISOR_DEFAULT_TOOL_NAMES]),
					0,
				);
				return;
			case "resetModel":
				this.#doc.advisors[index].model = undefined;
				this.#dirty = true;
				this.#showDetail(index);
				return;
			case "instructions":
				this.#showInstructionsEditor(index);
				return;
			case "delete":
				this.#doc.advisors.splice(index, 1);
				this.#dirty = true;
				this.#showList();
				return;
			default:
				this.#showList();
		}
	}

	#showNameEditor(index: number): void {
		const input = new Input();
		input.setValue(this.#doc.advisors[index].name);
		input.onSubmit = value => {
			const name = value.trim();
			if (name) {
				this.#doc.advisors[index].name = name;
				this.#dirty = true;
			}
			this.#showDetail(index);
		};
		input.onEscape = () => this.#showDetail(index);
		this.#setScreen("name", input, "Type a name / Enter save / Esc cancel");
	}

	#showModelPicker(index: number): void {
		const storage = this.#settings.getStorage();
		const mruOrder = storage?.getModelUsageOrder() ?? [];
		let models: ReadonlyArray<Model>;
		if (this.#scopedModels.length > 0) {
			models = this.#scopedModels.map(scoped => scoped.model);
		} else {
			try {
				models = this.#modelRegistry.getAvailable();
			} catch {
				models = [];
			}
		}
		const items = buildBrowserItems(models);
		sortModelItems(items, { mruOrder });

		const picker = new ModelBrowser(this.#settings, {});
		picker.setMruOrder(mruOrder);
		picker.setPerfStats(storage?.getModelPerf() ?? new Map());
		picker.setItems(items);
		picker.onActivate = item => {
			const efforts = getSupportedEfforts(item.model);
			if (efforts.length === 0) {
				this.#doc.advisors[index].model = item.selector;
				this.#dirty = true;
				this.#showDetail(index);
			} else {
				this.#showThinkingPicker(index, item.selector, efforts);
			}
		};
		picker.onCancel = () => this.#showDetail(index);
		this.#setScreen("model", picker, "Type to search / Enter or click twice picks / Esc back");
	}

	#showThinkingPicker(index: number, selector: string, efforts: readonly string[]): void {
		const items: SelectItem[] = [{ value: "", label: "(model default thinking)" }];
		for (const effort of efforts) items.push({ value: effort, label: effort });
		const list = new SelectList(items, Math.max(1, items.length), getSelectListTheme());
		list.onSelect = item => {
			// `item.value` is one of the model's own supported efforts (or "" for the
			// model default); `formatModelSelectorValue` spells the `:level` suffix.
			const level = item.value ? (item.value as ThinkingLevel) : undefined;
			this.#doc.advisors[index].model = formatModelSelectorValue(selector, level);
			this.#dirty = true;
			this.#showDetail(index);
		};
		list.onCancel = () => this.#showModelPicker(index);
		this.#setScreen("thinking", list, `Thinking effort for ${selector} / Enter pick / Esc back`);
	}

	#showToolsEditor(index: number, selected: Set<string>, cursor: number): void {
		const all = this.#availableToolNames;
		const items: SelectItem[] = all.map(name => ({
			value: name,
			label: `${selected.has(name) ? "[x]" : "[ ]"} ${name}`,
		}));
		items.push({ value: "__done", label: "Done" });
		const list = new SelectList(items, Math.max(1, items.length), getSelectListTheme());
		list.setSelectedIndex(cursor);
		let cursorIndex = cursor;
		list.onSelectionChange = item => {
			cursorIndex = items.findIndex(i => i.value === item.value);
		};
		list.onSelect = item => {
			if (item.value === "__done") {
				this.#doc.advisors[index].tools = commitTools(selected, all);
				this.#dirty = true;
				this.#showDetail(index);
				return;
			}
			if (selected.has(item.value)) selected.delete(item.value);
			else selected.add(item.value);
			this.#showToolsEditor(index, selected, cursorIndex);
		};
		list.onCancel = () => {
			this.#doc.advisors[index].tools = commitTools(selected, all);
			this.#dirty = true;
			this.#showDetail(index);
		};
		this.#setScreen("tools", list, "Enter toggle / Done or Esc applies (empty = no tools; read/grep/glob = default)");
	}

	/** `index === -1` edits the shared top-level instructions; otherwise advisor[index]. */
	#showInstructionsEditor(index: number): void {
		const shared = index < 0;
		const current = shared ? this.#doc.instructions : this.#doc.advisors[index].instructions;
		const title = shared ? "Shared advisor instructions" : `Instructions: ${this.#doc.advisors[index].name}`;
		const editor = new HookEditorComponent(
			this.#tui,
			title,
			current,
			value => {
				const text = value.trim() ? value : undefined;
				if (shared) this.#doc.instructions = text;
				else this.#doc.advisors[index].instructions = text;
				this.#dirty = true;
				if (shared) this.#showList();
				else this.#showDetail(index);
			},
			() => {
				if (shared) this.#showList();
				else this.#showDetail(index);
			},
		);
		this.#setScreen("instructions", editor, "");
	}
}
