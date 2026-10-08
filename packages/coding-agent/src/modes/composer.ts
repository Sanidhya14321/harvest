import {
	type Component,
	Container,
	CURSOR_MARKER,
	Ellipsis,
	type EditorTopBorder,
	isInsideTerminalMultiplexer,
	ProcessTerminal,
	type ResizeScrollbackMode,
	Spacer,
	sliceWithWidth,
	type Terminal,
	type TerminalFramePlan,
	type TerminalFrameProvider,
	truncateToWidth,
	TUI,
	type TUIOptions,
	type ViewportSize,
	visibleWidth,
} from "@harvest/pi-tui";
import { CustomEditor } from "./components/custom-editor";
import type { SessionTabStrip } from "./components/session-tab-strip";
import type { TabStripScreenOrigin } from "../session/session-tabs";
import type { KeybindingsManager } from "../config/keybindings";
import { type AnimationFrame, TranscriptContainer } from "./components/transcript-container";
import type { WorkspaceSidebar } from "./components/workspace-sidebar";
import { type LspServerInfo, type RecentSession, WelcomeComponent } from "./components/welcome";
import { getEditorTheme, initThemeSync, theme } from "./theme/theme";
import {
	composerMaxHeight,
	computeWorkspaceLayout,
	resolveWorkspaceComposerShape,
	sidebarOverlayWidth,
	WORKSPACE_LAYOUT,
	type SidebarPreference,
} from "./workspace-layout";

const DOUBLE_INTERRUPT_MS = 500;

/** Live settings that affect the composer before and after session adoption. */
export interface ComposerPreferences {
	readonly quiet: boolean;
	readonly fullscreen: boolean;
	readonly composerShape: string;
	readonly sidebar?: string;
	readonly showHardwareCursor: boolean;
	readonly maxInlineImages: number;
	readonly resizeScrollback: ResizeScrollbackMode;
	readonly imeSafeCursor: boolean;
	readonly autocompleteMaxVisible: number;
	readonly spellingTypoDetection: boolean;
	readonly spellingAutocomplete: boolean;
	readonly spellingAutocorrect: boolean;
}

/** Settings-schema-compatible defaults used when constructing a dependency-free composer. */
export const COMPOSER_DEFAULTS: ComposerPreferences = {
	quiet: false,
	fullscreen: true,
	composerShape: "band",
	sidebar: "auto",
	showHardwareCursor: true,
	maxInlineImages: 8,
	resizeScrollback: "rebuild",
	imeSafeCursor: false,
	autocompleteMaxVisible: 10,
	spellingTypoDetection: true,
	spellingAutocomplete: true,
	spellingAutocorrect: false,
};

/** Welcome data that can be supplied initially or patched as startup resolves it. */
export interface ComposerWelcomeUpdate {
	readonly version?: string;
	readonly modelName?: string;
	readonly providerName?: string;
	readonly recentSessions?: readonly RecentSession[];
	readonly lspServers?: readonly LspServerInfo[];
}

/**
 * Placeholder-only status chrome replayed on the next first frame so the
 * status band/border exists before the session-aware status line attaches.
 * Bound to the composer shape it was rendered for; a different shape drops it.
 */
export interface ComposerStatusSnapshot {
	readonly shape: string;
	/** ANSI wrapper of the editor border at snapshot time (session accent or thinking color). */
	readonly borderColor?: {
		readonly prefix: string;
		readonly suffix: string;
	};
	/** Status content embedded in the editor's top chrome (`top-border`, `top-band`, `top-rule-chip`). */
	readonly topBorder?: {
		readonly content: string;
		readonly width: number;
	};
	/** Standalone bottom-bar rows (`pi`/`claude` shapes), gap row included. */
	readonly bottomLines: readonly string[];
}

/** Optional dependencies and initial state for a standalone composer. */
export interface ComposerOptions {
	readonly terminal?: Terminal;
	/** Extra TUI construction options (render scheduler injection for tests and `omp render`). */
	readonly tuiOptions?: TUIOptions;
	readonly preferences?: Partial<ComposerPreferences>;
	readonly welcome?: ComposerWelcomeUpdate;
	readonly status?: ComposerStatusSnapshot;
	readonly exit?: (code: number) => void;
	readonly now?: () => number;
}

/** Controls the first terminal paint for a composer that does not already own the terminal. */
export interface ComposerStartOptions {
	readonly clearScrollback?: boolean;
	readonly playWelcomeIntro?: boolean;
	/**
	 * Paint without owning stdin: the tty keeps cooked-mode echo/editing so
	 * typing stays visible while startup module loading blocks the event loop.
	 * {@link Composer.enableInput} later switches to raw input and replays the
	 * kernel-buffered keystrokes into the editor.
	 */
	readonly deferInput?: boolean;
}

/**
 * Mount slot for the session-aware status component below the editor. Shows
 * placeholder rows during startup until the real component mounts.
 */
class StatusHost implements Component {
	#lines: readonly string[] = [];
	#component: Component | undefined;
	#maxHeight = Infinity;

	get mounted(): boolean {
		return this.#component !== undefined;
	}

	setLines(lines: readonly string[]): void {
		this.#lines = lines;
	}

	setComponent(component: Component): void {
		this.#component = component;
		this.#lines = [];
	}

	setMaxHeight(height: number): void {
		this.#maxHeight = Math.max(0, height);
		if (height > 0) this.#component?.setMaxHeight?.(height);
	}

	render(width: number): readonly string[] {
		if (this.#maxHeight === 0) return [];
		const rows = this.#component ? this.#component.render(width) : this.#lines.map(line => truncateToWidth(line, width, theme.getSymbolPreset() === "ascii" ? Ellipsis.Ascii : Ellipsis.Unicode));
		return rows.slice(-this.#maxHeight);
	}
}
/**
 * Canonical interactive composer, usable before session/settings exist and updatable in place.
 * It owns the terminal, welcome header, and editor; InteractiveMode later supplies authoritative
 * data and mounts the session-aware runtime children without replacing the visible header.
 */
export class Composer implements TerminalFrameProvider {
	/** Terminal renderer shared with InteractiveMode after adoption. */
	readonly ui: TUI;
	#editor: CustomEditor;
	readonly #header = new Container();
	readonly #bootstrapInputGap = new Spacer(1);
	readonly #statusHost = new StatusHost();
	readonly #exit: (code: number) => void;
	readonly #now: () => number;
	#preferences: ComposerPreferences;
	#welcome: WelcomeComponent | undefined;
	#version = "";
	#modelName = "";
	#providerName = "";
	#recentSessions: RecentSession[] = [];
	#lspServers: LspServerInfo[] = [];
	#headerBefore: readonly Component[] = [];
	#headerAfter: readonly Component[] = [];
	#runtimeChildren: readonly Component[] = [];
	#pinnedErrorContainer: Container | undefined;
	#attachmentContainer: Component | undefined;

	setPinnedErrorContainer(container: Container | undefined): void {
		this.#pinnedErrorContainer = container;
	}

	setAttachmentContainer(container: Component | undefined): void {
		this.#attachmentContainer = container;
	}
	/**
	 * Zero-tab override: the last tab was closed and its session continues
	 * hidden. The transcript stays mounted (state is preserved for reopen)
	 * but the workspace renders Home and never dispatches input against the
	 * hidden session. Cleared by any new-session/select/reopen flow.
	 */
	#forceHome = false;

	/** Enter or leave the zero-tab Home override. */
	setForceHome(force: boolean): void {
		if (this.#forceHome === force) return;
		this.#forceHome = force;
		this.ui.requestRender();
	}

	get forceHome(): boolean {
		return this.#forceHome;
	}
	#workspaceTabs: SessionTabStrip | undefined;
	/**
	 * Screen-space frame of the tab-strip rows in the last composed frame,
	 * derived from the final placement (offsets, headers, clipping, sidebar
	 * cover) — never assumed. `undefined` when no strip row is displayed.
	 * `coverFromCol` marks the sidebar-overlay cover cutoff: strip cells at
	 * or beyond it are visually hidden and must not hit-test.
	 */
	#stripFrame:
		| { origin: TabStripScreenOrigin; rows: number; coverFromCol: number | undefined }
		| undefined;
	#workspaceSidebar: WorkspaceSidebar | undefined;
	#sidebarOverlayOpen = false;
	#sidebarOverlayRequestedOnce = false;
	#workspaceScrollOffset = 0;
	// Offset/width the reading anchor was last pinned for. While scrolled
	// back the transcript paints from the pinned anchor (identity-stable
	// across appended rows) instead of a bottom-relative offset; the anchor
	// is re-pinned only when the offset or width changes. Offset 0 follows
	// the live tail and releases any anchor.
	#scrollAnchorOffset: number | undefined;
	#scrollAnchorWidth: number | undefined;
	#statusSnapshot: ComposerStatusSnapshot | undefined;
	#keyHintSource: Pick<KeybindingsManager, "getDisplayString"> | undefined;
	#runtimeMounted = false;
	// Composer-owned history id space. Transcript batch ids restart across
	// container clears/swaps; the composer translates them into one monotonic
	// sequence the terminal's accepted-id watermark can trust.
	#nextHistoryId = 1;
	#offeredHistory:
		| {
				id: number;
				rows: readonly string[];
				kind: "append" | "replay";
				source:
					| "header"
					| {
							transcript: TranscriptContainer;
							transcriptId?: number;
							header: "none" | "replay";
							/** Recomposed header rows to accept as the new retired-header bytes. */
							headerRows?: readonly string[];
					  };
		  }
		| undefined;
	#historyReplayRequested = false;
	#headerReplayPending = false;
	#historyFlush = false;
	// The welcome header retires to terminal history exactly once, after the
	// intro settles; until then it renders as mutable viewport chrome.
	#headerRetired = false;
	// Exact hard rows accepted into native history. Transient resize-alt
	// paints reflow these rows to match the terminal's own rewrap of history
	// it still holds; a settled replay owns every byte it emits, so it
	// recomposes the header at the replay width and refreshes these rows.
	#retiredHeaderRows: readonly string[] | undefined;
	// Hard-row prefix currently above the native viewport. The first resize
	// frame may pull part of it down before the normal buffer is borrowed.
	#retiredHeaderStart = 0;
	#resizeRetiredHeaderStart: number | undefined;
	#lastNormalRows = 0;
	#lastInterruptAt = 0;
	#started = false;
	#stopped = false;
	#transferred = false;

	constructor(options: ComposerOptions = {}) {
		if (typeof theme === "undefined") initThemeSync();
		this.#exit = options.exit ?? (code => process.exit(code));
		this.#now = options.now ?? Date.now;
		this.#preferences = { ...COMPOSER_DEFAULTS, ...options.preferences };
		this.#statusSnapshot = options.status;
		this.#applyWelcomeUpdate(options.welcome ?? {});

		this.ui = new TUI(
			options.terminal ?? new ProcessTerminal(),
			this.#preferences.showHardwareCursor,
			options.tuiOptions,
		);
		this.ui.setFrameProvider(this);
		this.ui.setBaseFullscreen(this.#preferences.fullscreen === true);
		this.ui.setMaxInlineImages(this.#preferences.maxInlineImages);
		this.ui.setResizeScrollback(this.#preferences.resizeScrollback);

		this.#editor = new CustomEditor(getEditorTheme());
		this.editor.disableSubmit = true;
		this.editor.setUseTerminalCursor(this.ui.getShowHardwareCursor());
		this.editor.setImeSafeCursorLayout(this.#preferences.imeSafeCursor);
		this.editor.setAutocompleteMaxVisible(this.#preferences.autocompleteMaxVisible);
		this.editor.setSpellingFeatures({
			typoDetection: this.#preferences.spellingTypoDetection,
			autocomplete: this.#preferences.spellingAutocomplete,
			autocorrect: this.#preferences.spellingAutocorrect,
		});
		try {
			this.editor.setBorderStyle(this.#workspaceComposerShape());
		} catch {
			// Extension-defined styles arrive with the session; InteractiveMode reapplies them.
		}
		this.#applyStatusSnapshot();
		// Emergency controls stay active until InteractiveMode installs configured bindings.
		this.editor.setActionKeys("app.clear", ["ctrl+c"]);
		this.editor.setActionKeys("app.exit", ["ctrl+d"]);
		this.editor.onClear = () => this.#handleInterrupt();
		this.editor.onExit = () => this.#requestExit(0);
		this.editor.setShimmerRepaintHandler(() => this.ui.requestComponentRender(this.editor));

		if (!this.#preferences.quiet) this.#ensureWelcome();
		this.#rebuildHeader();
		this.ui.addChild(this.#header);
		this.ui.addChild(this.#bootstrapInputGap);
		this.ui.addChild(this.editor);
		this.ui.addChild(this.#statusHost);
		this.ui.setFocus(this.editor);
	}
	/** Compose the bounded mutable viewport and the next ordered history append. */
	renderFrame(viewport: ViewportSize): TerminalFramePlan {
		if (!this.#started || this.#stopped) return { viewport: [] };
		const width = Math.max(1, viewport.columns);
		const rows = Math.max(0, viewport.rows);
		if (this.#preferences.fullscreen) return { viewport: this.#renderWorkspace(width, rows) };
		if (this.#resizeRetiredHeaderStart !== undefined) {
			this.#retiredHeaderStart = this.#resizeRetiredHeaderStart;
			this.#resizeRetiredHeaderStart = undefined;
		}
		this.#lastNormalRows = rows;
		const roots = this.#runtimeMounted
			? [...this.#runtimeChildren, this.#statusHost]
			: [this.#header, this.#bootstrapInputGap, this.editor, this.#statusHost];
		const transcriptIndex = roots.findIndex(root => root instanceof TranscriptContainer);
		if (transcriptIndex < 0) {
			return { viewport: this.#renderFixedRoots(roots, width, rows) };
		}
		const transcript = roots[transcriptIndex] as TranscriptContainer;
		const preRoots = this.#renderRoots(roots.slice(0, transcriptIndex), width);
		const after = this.#renderFixedRoots(roots.slice(transcriptIndex + 1), width, rows);
		// Offer history under capacity pressure only: blocks stay live (and keep
		// reflowing to the current width) while the screen has room. A batch
		// leaves the mutable viewport in the same frame it is appended, so its
		// rows are never painted twice.
		const history = this.#offerHistory(transcript, width, rows, preRoots.length + after.length);
		const headerVisible = !this.#headerRetired && this.#offeredHistory?.source !== "header";
		const headerRows = headerVisible ? this.#header.render(width) : [];
		const before = [...headerRows, ...preRoots];
		const now = performance.now();
		const frame: AnimationFrame = { now, tick: Math.floor(now / 80) };
		const active = transcript.renderViewport(width, Math.max(0, rows - before.length - after.length), frame);
		const composed = [...before, ...active, ...after];
		if (history !== undefined && this.#offeredHistory?.source === "header") {
			const visibleHeaderRows = Math.max(0, rows - composed.length);
			this.#retiredHeaderStart = Math.max(0, history.rows.length - visibleHeaderRows);
		}
		return {
			history,
			viewport: composed.length <= rows ? composed : composed.slice(-rows),
		};
	}

	/** Acknowledges one accepted header, replay, or transcript batch. */
	acknowledgeHistory(id: number): void {
		const offered = this.#offeredHistory;
		if (offered === undefined || offered.id !== id) return;
		if (offered.source === "header") {
			this.#headerRetired = true;
			this.#retiredHeaderRows = offered.rows;
		} else {
			if (offered.source.transcriptId !== undefined) {
				offered.source.transcript.acknowledgeFinalizedBatch(offered.source.transcriptId);
			}
			if (offered.source.header === "replay") {
				this.#headerReplayPending = false;
				if (offered.source.headerRows !== undefined) this.#retiredHeaderRows = offered.source.headerRows;
			}
		}
		this.#offeredHistory = undefined;
		if (this.#historyReplayRequested) this.#startHistoryReplay();
	}

	/** Render the semantic transcript tail while the terminal borrows its resize buffer. */
	renderResizeFrame(viewport: ViewportSize): readonly string[] {
		if (!this.#started || this.#stopped) return [];
		const width = Math.max(1, viewport.columns);
		const rows = Math.max(0, viewport.rows);
		if (this.#preferences.fullscreen) return this.#renderWorkspace(width, rows);
		const tail = this.#runtimeMounted
			? this.#renderResizeTail(width, rows)
			: this.#renderFixedRoots([this.#bootstrapInputGap, this.editor, this.#statusHost], width, rows);
		let header: readonly string[];
		if (this.#headerRetired) {
			this.#resizeRetiredHeaderStart ??= Math.max(
				0,
				this.#retiredHeaderStart - Math.max(0, rows - this.#lastNormalRows),
			);
			header = this.#reflowRetiredHeader(width, this.#resizeRetiredHeaderStart);
		} else {
			header = this.#header.render(width);
		}
		const rendered = [...header, ...tail];
		return rendered.length <= rows ? rendered : rendered.slice(rendered.length - rows);
	}

	/** Replays committed presentation without changing logical retirement state. */
	beginHistoryReplay(): void {
		if (this.#offeredHistory !== undefined) {
			this.#historyReplayRequested = true;
			return;
		}
		this.#startHistoryReplay();
	}

	/** Forces every currently eligible finalized prefix to retire before stop. */
	beginHistoryFlush(): void {
		this.#historyFlush = true;
		// A pending replay would re-render and re-stream the entire committed
		// ledger during shutdown; the terminal already holds that history, so
		// flush emits only genuinely un-retired rows. An already offered batch
		// stays valid and is accepted by the flush loop.
		this.#historyReplayRequested = false;
		this.#headerReplayPending = false;
		for (const child of this.#runtimeChildren) {
			if (child instanceof TranscriptContainer) child.cancelReplay();
		}
	}

	#startHistoryReplay(): void {
		this.#headerReplayPending = this.#headerRetired && (this.#retiredHeaderRows?.length ?? 0) > 0;
		this.#historyReplayRequested = false;
		for (const child of this.#runtimeChildren) {
			if (child instanceof TranscriptContainer) child.beginReplay();
		}
	}

	/** Header retires first; replay coalesces it with the complete transcript ledger. */
	#offerHistory(
		transcript: TranscriptContainer,
		width: number,
		rows: number,
		chromeRows: number,
	): { id: number; rows: readonly string[]; kind: "append" | "replay" } | undefined {
		if (this.#offeredHistory !== undefined) {
			this.#rerenderOfferedHistory(width);
			return {
				id: this.#offeredHistory.id,
				rows: this.#offeredHistory.rows,
				kind: this.#offeredHistory.kind,
			};
		}
		if (this.#headerReplayPending) {
			const transcriptReplay = transcript.peekReplayBatch(width);
			// A replay follows a scrollback clear, so the header recomposes at
			// the new width exactly like transcript entries do. An empty
			// recompose (welcome unmounted after retirement) falls back to the
			// committed rows, hard-wrapped the way the terminal would.
			const recomposed = this.#header.render(width);
			const headerRows = recomposed.length > 0 ? [...recomposed, ""] : this.#reflowRetiredHeader(width, 0);
			this.#offeredHistory = {
				id: this.#nextHistoryId++,
				rows: [...headerRows, ...(transcriptReplay?.rows ?? [])],
				kind: "replay",
				source: {
					transcript,
					transcriptId: transcriptReplay?.id,
					header: "replay",
					headerRows,
				},
			};
			return {
				id: this.#offeredHistory.id,
				rows: this.#offeredHistory.rows,
				kind: this.#offeredHistory.kind,
			};
		}
		if (!this.#headerRetired) {
			const welcome = this.#welcome;
			if (welcome !== undefined && !welcome.isTranscriptBlockFinalized()) return undefined;
			// The header stays live viewport chrome until the screen fills; then it
			// retires first so transcript prefixes can follow in order.
			const renderedHeader = this.#header.render(width);
			if (renderedHeader.length > 0) {
				const liveRows = transcript.liveRowCount(width);
				if (!this.#historyFlush && renderedHeader.length + chromeRows + liveRows <= rows) return undefined;
				this.#offeredHistory = {
					id: this.#nextHistoryId++,
					rows: [...renderedHeader, ""],
					kind: "append",
					source: "header",
				};
				return {
					id: this.#offeredHistory.id,
					rows: this.#offeredHistory.rows,
					kind: this.#offeredHistory.kind,
				};
			}
			this.#headerRetired = true;
			this.#retiredHeaderRows = [];
		}
		const batch = this.#historyFlush
			? transcript.peekFlushBatch(width)
			: transcript.peekFinalizedBatch(width, Math.max(0, rows - chromeRows));
		if (batch === undefined) return undefined;
		this.#offeredHistory = {
			id: this.#nextHistoryId++,
			rows: batch.rows,
			kind: batch.kind ?? "append",
			source: { transcript, transcriptId: batch.id, header: "none" },
		};
		return {
			id: this.#offeredHistory.id,
			rows: this.#offeredHistory.rows,
			kind: this.#offeredHistory.kind,
		};
	}

	#rerenderOfferedHistory(width: number): void {
		const offered = this.#offeredHistory;
		if (offered === undefined) return;
		if (offered.source === "header") {
			const rows = this.#header.render(width);
			offered.rows = rows.length > 0 ? [...rows, ""] : [];
			return;
		}
		const transcript = offered.source.transcript.rerenderOfferedBatch(width);
		if (offered.source.header === "none") {
			if (transcript !== undefined) offered.rows = transcript.rows;
			return;
		}
		const recomposed = this.#header.render(width);
		const headerRows = recomposed.length > 0 ? [...recomposed, ""] : this.#reflowRetiredHeader(width, 0);
		offered.source.headerRows = headerRows;
		offered.rows = [...headerRows, ...(transcript?.rows ?? [])];
	}

	#renderRoots(roots: readonly Component[], width: number): string[] {
		const rows: string[] = [];
		for (const root of roots) rows.push(...root.render(width));
		return rows;
	}

	/** Collapse optional chrome before the current draft/cursor can leave the viewport. */
	#renderFixedRoots(roots: readonly Component[], width: number, height: number): string[] {
		if (height <= 0) return [];
		const containsEditor = (component: Component): boolean => component === this.editor ||
			(component instanceof Container && component.children.some(containsEditor));
		const editorRoot = roots.find(containsEditor);
		this.editor.setMaxHeight(composerMaxHeight(height, 4));
		const editorRows = editorRoot?.render(width) ?? [];
		const attachmentRows = this.#attachmentContainer && roots.includes(this.#attachmentContainer) ? this.#attachmentContainer.render(width) : [];
		const coreEditorRows = this.#trimPaddingRows(editorRows);
		const errorBudget = Math.min(6, Math.max(0, height - Math.min(height, coreEditorRows.length) - Number(attachmentRows.length > 0)));
		for (const child of this.#pinnedErrorContainer?.children ?? []) child.setMaxHeight?.(Math.max(1, errorBudget));
		this.#statusHost.setMaxHeight(height);
		const records = roots.map(root => ({ root, rows: root === this.#pinnedErrorContainer && errorBudget === 0 ? [] : root === editorRoot ? editorRows : root === this.#attachmentContainer ? attachmentRows : root.render(width) }));
		if (records.reduce((total, record) => total + record.rows.length, 0) <= height) return records.flatMap(record => [...record.rows]);
		const retained = new Map<Component, readonly string[]>();
		let remaining = height;
		const take = (component: Component | undefined, budget: number, cursor = false): void => {
			const record = records.find(record => record.root === component);
			if (!record || remaining <= 0) return;
			const selected = cursor ? this.#rowsAroundCursor(coreEditorRows, Math.min(remaining, budget)) : record.rows.slice(0, Math.min(remaining, budget));
			retained.set(record.root, selected);
			remaining -= selected.length;
		};
		take(editorRoot, height, true);
		take(this.#attachmentContainer, 1);
		take(this.#pinnedErrorContainer, remaining);
		for (const record of records.toReversed()) if (!retained.has(record.root)) take(record.root, remaining);
		return records.flatMap(record => [...(retained.get(record.root) ?? [])]);
	}

	#trimPaddingRows(rows: readonly string[]): readonly string[] {
		const blank = (row: string): boolean => !row.includes(CURSOR_MARKER) && Bun.stripANSI(row).trim().length === 0;
		let start = 0;
		let end = rows.length;
		while (start < end && blank(rows[start]!)) start++;
		while (end > start && blank(rows[end - 1]!)) end--;
		return rows.slice(start, end);
	}

	#rowsAroundCursor(rows: readonly string[], height: number): readonly string[] {
		if (height <= 0) return [];
		if (rows.length <= height) return rows;
		const cursor = rows.findIndex(row => row.includes(CURSOR_MARKER));
		if (cursor < 0) return rows.slice(-height);
		const start = Math.max(0, Math.min(cursor, rows.length - height));
		return rows.slice(start, start + height);
	}

	#workspaceComposerShape(): string {
		return resolveWorkspaceComposerShape(this.#preferences.fullscreen, this.#preferences.composerShape);
	}

	/** Persisted sidebar preference: auto docks when wide, show requests overlay on narrow. */
	get sidebarPreference(): SidebarPreference {
		const raw = this.#preferences.sidebar;
		if (raw === "show" || raw === "hide" || raw === "auto") return raw;
		return "auto";
	}

	setWorkspaceSidebar(sidebar: WorkspaceSidebar | undefined): void {
		this.#workspaceSidebar = sidebar;
		this.ui.requestRender();
	}

	setSidebarOverlayOpen(open: boolean): void {
		if (this.#sidebarOverlayOpen === open) return;
		this.#sidebarOverlayOpen = open;
		this.ui.requestRender();
	}

	get sidebarOverlayOpen(): boolean {
		return this.#sidebarOverlayOpen;
	}

	/** Screen-space mouse routing through final clipped rectangles. */
	routeWorkspaceMouse(column: number): "main" | "sidebar" | "outside" {
		const columns = this.ui.terminal.columns ?? 80;
		const rows = this.ui.terminal.rows ?? 24;
		const transcript = this.#findTranscript();
		const geometry = computeWorkspaceLayout(
			{ columns, rows },
			{
				hasConversation: Boolean(transcript?.children.length) && !this.#forceHome,
				sidebarPreference: this.sidebarPreference,
				sidebarOverlayOpen: this.#sidebarOverlayOpen,
			},
		);
		return geometry.hitTest(column);
	}

	#findTranscript(): TranscriptContainer | undefined {
		for (const child of this.#runtimeChildren) {
			if (child instanceof TranscriptContainer) return child;
		}
		return undefined;
	}

	setWorkspaceTabs(tabs: SessionTabStrip): void {
		this.#workspaceTabs = tabs;
		this.ui.requestRender();
	}

	/**
	 * Screen-space frame of the displayed tab-strip rows from the final
	 * composed frame, for owner-side mouse translation. `undefined` when the
	 * strip shows no rows (hidden, clipped away, or single-tab Home).
	 */
	workspaceStripFrame():
		| { origin: TabStripScreenOrigin; rows: number; coverFromCol: number | undefined }
		| undefined {
		return this.#stripFrame;
	}

	/**
	 * Record strip placement for the frame being composed and clip the
	 * strip's own hit map to the rows actually displayed. Call with the
	 * placed row/column and surviving row count; `rows <= 0` records no
	 * frame and clears all hit targets.
	 */
	#noteStripFrame(row: number, col: number, rows: number, coverFromCol: number | undefined): void {
		const tabs = this.#workspaceTabs;
		const placed = Math.max(0, Math.floor(rows));
		tabs?.clipDisplayedRows(placed);
		if (!tabs || placed <= 0) {
			this.#stripFrame = undefined;
			return;
		}
		this.#stripFrame = { origin: { row, col }, rows: placed, coverFromCol };
	}

	scrollWorkspace(delta: number): void {
		// Positive deltas look back in history (older rows); negative deltas
		// move toward the live tail. Callers must use scrollWorkspaceWheel /
		// scrollWorkspacePage so every input shares this convention.
		this.#workspaceScrollOffset = Math.max(0, this.#workspaceScrollOffset + delta);
		if (this.#workspaceScrollOffset === 0) this.#releaseScrollAnchor();
		this.ui.requestRender();
	}

	/**
	 * Mouse-wheel scroll in row units. Wheel-up (`-1`) goes back in history,
	 * wheel-down (`1`) returns toward the live tail.
	 */
	scrollWorkspaceWheel(wheel: -1 | 1, linesPerNotch = 3): void {
		this.scrollWorkspace(-wheel * linesPerNotch);
	}

	/** Page-key scroll: page-up goes back in history, page-down toward live. */
	scrollWorkspacePage(direction: "up" | "down", rows: number): void {
		const page = Math.max(1, rows - 4);
		this.scrollWorkspace(direction === "up" ? page : -page);
	}

	/** Current transcript scroll-back offset in rows; restored per session on tab switches. */
	get workspaceScrollOffset(): number {
		return this.#workspaceScrollOffset;
	}

	/** Restore a per-session scroll-back offset; negative values clamp to the live tail. */
	setWorkspaceScrollOffset(offset: number): void {
		this.#workspaceScrollOffset = Math.max(0, Math.floor(offset));
		if (this.#workspaceScrollOffset === 0) this.#releaseScrollAnchor();
		this.ui.requestRender();
	}

	resetWorkspaceScroll(): void {
		this.#workspaceScrollOffset = 0;
		this.#releaseScrollAnchor();
		this.ui.requestRender();
	}

	/** Release the pinned reading position and return to the live tail. */
	#releaseScrollAnchor(): void {
		this.#scrollAnchorOffset = undefined;
		this.#scrollAnchorWidth = undefined;
		this.#findTranscript()?.followLiveTail();
	}

	/**
	 * Session transcript window. At offset 0 the live tail paints directly.
	 * Scrolled back, the window paints from a pinned reading anchor
	 * (identity-stable: appended rows below it never move the viewed text;
	 * rewrap/sidebar resize recompute from the same anchor). The anchor is
	 * pinned once per offset/width and re-pinned when its block retires.
	 */
	#renderSessionTranscript(transcript: TranscriptContainer, mainWidth: number, available: number): readonly string[] {
		const offset = this.#workspaceScrollOffset;
		if (offset <= 0 || available <= 0) {
			if (offset <= 0) this.#releaseScrollAnchor();
			const tail = transcript.renderTail(mainWidth, available + Math.max(0, offset));
			this.#workspaceScrollOffset = Math.min(offset, Math.max(0, tail.length - available));
			return offset > 0 ? tail.slice(0, Math.max(0, available)) : tail;
		}
		if (this.#scrollAnchorOffset !== offset || this.#scrollAnchorWidth !== mainWidth) {
			const found = transcript.anchorForOffset(mainWidth, available, offset);
			if (found) transcript.pinReadingAnchor(found.block, found.row);
			else transcript.followLiveTail();
			this.#scrollAnchorOffset = offset;
			this.#scrollAnchorWidth = mainWidth;
		}
		let rows = transcript.renderAnchoredViewport(mainWidth, available);
		if (rows.length === 0 && !transcript.isFollowingTail()) {
			// Pinned block retired mid-read: re-pin once at the same offset.
			const found = transcript.anchorForOffset(mainWidth, available, offset);
			if (found) transcript.pinReadingAnchor(found.block, found.row);
			else transcript.followLiveTail();
			rows = transcript.renderAnchoredViewport(mainWidth, available);
		}
		if (rows.length === 0) {
			// Offset beyond content or anchor released: legacy tail behavior.
			transcript.followLiveTail();
			const tail = transcript.renderTail(mainWidth, available + offset);
			this.#workspaceScrollOffset = Math.min(offset, Math.max(0, tail.length - available));
			return tail.slice(0, Math.max(0, available));
		}
		return rows;
	}

	#renderWorkspace(width: number, rows: number): readonly string[] {
		const safeWidth = Math.max(1, Math.floor(width));
		const safeRows = Math.max(0, Math.floor(rows));
		if (safeRows === 0) return [];
		const roots = this.#runtimeMounted
			? [...this.#runtimeChildren, this.#statusHost]
			: [this.#bootstrapInputGap, this.editor, this.#statusHost];
		const transcriptIndex = roots.findIndex(root => root instanceof TranscriptContainer);
		const transcript = transcriptIndex >= 0 ? (roots[transcriptIndex] as TranscriptContainer) : undefined;
		const hasConversation = Boolean(transcript?.children.length) && !this.#forceHome;
		// Entering narrow `show` requests the overlay once; dismissal sticks
		// until an explicit toggle/focus action. Do not reopen on every render.
		const preference = this.sidebarPreference;
		if (
			preference === "show" &&
			safeWidth <= WORKSPACE_LAYOUT.sidebarDockBreakpoint &&
			!this.#sidebarOverlayRequestedOnce
		) {
			this.#sidebarOverlayRequestedOnce = true;
			this.#sidebarOverlayOpen = true;
		}
		if (preference !== "show") this.#sidebarOverlayRequestedOnce = false;
		if (safeWidth > WORKSPACE_LAYOUT.sidebarDockBreakpoint) this.#sidebarOverlayOpen = false;
		const geometry = computeWorkspaceLayout(
			{ columns: safeWidth, rows: safeRows },
			{ hasConversation, sidebarPreference: preference, sidebarOverlayOpen: this.#sidebarOverlayOpen },
		);
		const mainWidth = geometry.main.width;
		const sidebarDocked = geometry.sidebarDocked && this.#workspaceSidebar !== undefined;
		const overlayActive =
			!sidebarDocked &&
			this.#workspaceSidebar !== undefined &&
			geometry.sidebarOverlayRequested &&
			this.#sidebarOverlayOpen;
		const contentWidth = hasConversation
			? Math.max(1, mainWidth)
			: Math.min(mainWidth, WORKSPACE_LAYOUT.homeCenterCap);
		const contentInset = hasConversation ? 0 : Math.max(0, Math.floor((mainWidth - contentWidth) / 2));
		// Sidebar-overlay cover cutoff for strip hit-testing: cells at or
		// beyond this column are visually hidden (S1). Docked sidebars need
		// no cutoff (main starts at column zero and strip cells end at its
		// width by construction).
		this.#stripFrame = undefined;
		const stripCoverFromCol = overlayActive ? Math.max(0, safeWidth - sidebarOverlayWidth(safeWidth)) : undefined;
		// Clamp multiline input growth to ~1/3 of the screen; reserve metadata.
		try {
			this.editor.setMaxHeight(composerMaxHeight(safeRows, 4));
		} catch {
			// Editor without max-height support keeps its internal budget.
		}
		const chromeWidth = hasConversation
			? Math.max(1, mainWidth - (safeWidth >= 40 ? WORKSPACE_LAYOUT.mainPaddingX * 2 : 0))
			: Math.min(geometry.homePromptWidth, Math.max(1, mainWidth - 2));
		const afterContent = this.#renderFixedRoots(
			roots.filter(root => root !== transcript && root !== this.#workspaceTabs),
			chromeWidth,
			safeRows,
		);
		const padRow = (row: string): string => {
			if (contentInset <= 0) return row;
			return `${" ".repeat(contentInset)}${row}`;
		};
		const after = afterContent.map(padRow);
		// Header extras (config warnings, changelog, notices) are independent
		// attention content, not conversation-state detectors. The welcome scene
		// stays inline-mode only; fullscreen home uses the intentional wordmark
		// below so non-conversation notices never change the layout detection.
		const headerRows =
			this.#headerBefore.length > 0 || this.#headerAfter.length > 0
				? this.#renderRoots([...this.#headerBefore, ...this.#headerAfter], mainWidth)
				: [];
		const centeredHeader = headerRows.map(padRow);
		const tabBudget = Math.max(0, safeRows - after.length - centeredHeader.length);
		const tabRows = this.#workspaceTabs?.renderWorkspace(mainWidth, hasConversation, tabBudget) ?? [];
		const tabs = tabRows.length > 0 ? [...tabRows.map(padRow), padRow("")] : [];
		const blank = (count: number): string[] => Array.from({ length: Math.max(0, count) }, () => "");
		// Height pressure priority: editable composer/cursor + attention first,
		// then conversation rows, then optional header/tabs, then decoration.
		// Never blind-slice the active input away.
		if (after.length >= safeRows) {
			// Tiny viewport: keep one attention row (warnings/tabs) alongside
			// the editable tail instead of dropping header/tabs entirely.
			const attention = [...centeredHeader, ...tabs].slice(0, 1);
			// The surviving attention row is a tab row only when the headers
			// contribute nothing before it; otherwise no strip row is
			// displayed and hit targets clear (S1).
			const tinyTabShown = centeredHeader.length === 0 && tabs.length > 0;
			this.#noteStripFrame(0, contentInset, tinyTabShown ? 1 : 0, stripCoverFromCol);
			const tinyRows =
				attention.length > 0 && safeRows >= 2
					? [...attention, ...this.#rowsAroundCursor(after, safeRows - 1)]
					: this.#rowsAroundCursor(after, safeRows);
			const mainOnly = this.#composeMainWithSidebar(
				tinyRows,
				[],
				mainWidth,
				safeWidth,
				safeRows,
				sidebarDocked,
				overlayActive,
			);
			return mainOnly;
		}
		if (hasConversation && transcript) {
			let headerTabs = [...centeredHeader, ...tabs];
			// Cap attention rows so the transcript keeps >= 1 row when rows >= 6.
			if (safeRows >= 6) {
				const maxHeaderTabs = Math.max(headerTabs.length > 0 ? 1 : 0, safeRows - after.length - 1);
				if (headerTabs.length > maxHeaderTabs) headerTabs = headerTabs.slice(0, Math.max(0, maxHeaderTabs));
			}
			// Strip rows surviving the cap start after the headers; a cap
			// cutting into the headers leaves no displayed strip row (S1).
			this.#noteStripFrame(
				Math.min(centeredHeader.length, headerTabs.length),
				contentInset,
				Math.max(0, headerTabs.length - centeredHeader.length),
				stripCoverFromCol,
			);
			const showHints = !this.#preferences.quiet && safeRows >= 8;
			const available = Math.max(0, safeRows - headerTabs.length - after.length - (showHints ? 1 : 0));
			const visible = this.#renderSessionTranscript(transcript, mainWidth, available);
			const paddedVisible = visible.map(row => {
				if (contentInset > 0) return `${" ".repeat(contentInset)}${row}`;
				if (safeWidth >= 40 && mainWidth > chromeWidth) return `  ${row}`;
				return row;
			});
			const mainRows = [
				...headerTabs,
				...paddedVisible,
				...blank(available - paddedVisible.length),
				...after,
				...(showHints ? [this.#hintRow(mainWidth)] : []),
			].slice(0, safeRows);
			return this.#composeMainWithSidebar(
				mainRows,
				centeredHeader,
				mainWidth,
				safeWidth,
				safeRows,
				sidebarDocked,
				overlayActive,
			);
		}
		// HOME: intentional wordmark, centered prompt, shortcuts, footer.
		const wordmarkRows = this.#homeWordmark(safeWidth);
		const hints = [this.#hintRow(safeWidth)];
		if (!this.#preferences.quiet) {
			// Non-quiet home budgets the whole viewport through #homeIntro
			// (one attention row + editor head + hints + decoration). The full
			// centered header never preempts the intro: at short heights its
			// padding blanks would displace the editable draft.
			const intro = this.#homeIntro(centeredHeader, tabs, after, wordmarkRows, hints, safeRows);
			const before = Math.max(0, Math.floor((safeRows - intro.length) / 2));
			// The intro shows exactly one attention row: the first content
			// row of headers+tabs. Tabs are displayed only when that row
			// comes from the tab block (S1).
			const introSource = [...centeredHeader, ...tabs];
			const introContent = introSource.findIndex(row => Bun.stripANSI(row).trim().length > 0);
			this.#noteStripFrame(
				before + centeredHeader.length,
				contentInset,
				introContent >= centeredHeader.length && introContent >= 0 ? 1 : 0,
				stripCoverFromCol,
			);
			const homeRows = [...blank(before), ...intro, ...blank(safeRows - before - intro.length)].slice(0, safeRows);
			// Version stamp bottom-right when the last row is unused padding.
			if (this.#version && homeRows.length > 0) {
				const last = homeRows.length - 1;
				if (Bun.stripANSI(homeRows[last]!).trim().length === 0) {
					const stamp = theme.fg("muted", `v${this.#version}`);
					const pad = Math.max(0, safeWidth - visibleWidth(stamp));
					homeRows[last] = truncateToWidth(`${" ".repeat(pad)}${stamp}`, Math.max(1, safeWidth));
				}
			}
			return this.#composeMainWithSidebar(
				homeRows,
				centeredHeader,
				mainWidth,
				safeWidth,
				safeRows,
				sidebarDocked,
				overlayActive,
			);
		}
		const intro = [...centeredHeader, ...tabs, ...after];
		// Vertically center the bounded home group when space allows.
		const before = Math.max(0, Math.floor((safeRows - centeredHeader.length - intro.length) / 2));
		// Quiet home lists every tab row; clamp to the rows that survive the
		// final viewport slice (S1).
		const quietTabRow = before + centeredHeader.length;
		this.#noteStripFrame(
			quietTabRow,
			contentInset,
			Math.max(0, Math.min(tabs.length, safeRows - quietTabRow)),
			stripCoverFromCol,
		);
		const homeRows = [
			...blank(before),
			...centeredHeader,
			...intro,
			...blank(safeRows - before - centeredHeader.length - intro.length),
		].slice(0, safeRows);
		return this.#composeMainWithSidebar(
			homeRows,
			centeredHeader,
			mainWidth,
			safeWidth,
			safeRows,
			sidebarDocked,
			overlayActive,
		);
	}

	/**
	 * Home column under height pressure. Priority: required attention, then
	 * the editable composer head (the draft lives on its first rows), then
	 * hints, then wordmark decoration. Decorations never displace the draft.
	 */
	#homeIntro(
		centeredHeader: readonly string[],
		tabs: readonly string[],
		after: readonly string[],
		wordmarkRows: readonly string[],
		hints: readonly string[],
		safeRows: number,
	): readonly string[] {
		const attentionSource = [...centeredHeader, ...tabs];
		// Warnings render with surrounding blank padding; the attention row
		// is the first row with content, never a padding blank.
		const contentIndex = attentionSource.findIndex(row => Bun.stripANSI(row).trim().length > 0);
		const attention =
			contentIndex >= 0 ? attentionSource.slice(contentIndex, contentIndex + 1) : attentionSource.slice(0, 1);
		const afterBudget = Math.max(0, safeRows - attention.length);
		// Leading gap spacers (space-padded blank rows) yield to editable rows first.
		let head = 0;
		while (head < after.length && Bun.stripANSI(after[head]!).trim().length === 0) head++;
		const homeAfter = this.#rowsAroundCursor(after.slice(head), afterBudget);
		const spare = Math.max(0, afterBudget - homeAfter.length);
		const homeHints = hints.slice(0, spare);
		const homeDeco = [...wordmarkRows, ""].slice(0, Math.max(0, spare - homeHints.length));
		return [...attention, ...homeDeco, ...homeAfter, ...homeHints];
	}

	/**
	 * Pixel-block HARVEST wordmark (OpenCode-style terminal glyph art, original
	 * Harvest name): muted left half, bright accent right half, centered. Falls
	 * back to plain text on narrow viewports and the ASCII symbol preset.
	 */
	#homeWordmark(safeWidth: number): readonly string[] {
		const text = "harvest";
		const textRow = `${" ".repeat(Math.max(0, Math.floor((safeWidth - visibleWidth(text)) / 2)))}${theme.bold(theme.fg("accent", text))}`;
		if (theme.getSymbolPreset() === "ascii" || safeWidth < 45) return [textRow];
		const glyphs: Record<string, readonly string[]> = {
			H: ["█   █", "█   █", "█████", "█   █", "█   █"],
			A: ["  █  ", " █ █ ", "█████", "█   █", "█   █"],
			R: ["████ ", "█   █", "████ ", "█  █ ", "█   █"],
			V: ["█   █", "█   █", "█   █", " █ █ ", "  █  "],
			E: ["█████", "█    ", "████ ", "█    ", "█████"],
			S: [" ████", "█    ", " ███ ", "    █", "████ "],
			T: ["█████", "  █  ", "  █  ", "  █  ", "  █  "],
		};
		const letters = "HARVEST".split("").map(letter => glyphs[letter] ?? []);
		const rows: string[] = [];
		for (let row = 0; row < 5; row++) {
			const cells = letters.map(cell => cell[row] ?? "     ");
			// "HARV" muted, "EST" bright (4 cells + 3 separators = 23 columns).
			const left = cells.slice(0, 4).join(" ");
			const right = cells.slice(4).join(" ");
			const art = `${theme.fg("muted", left)} ${theme.bold(theme.fg("accent", right))}`;
			const pad = Math.max(0, Math.floor((safeWidth - 41) / 2));
			rows.push(`${" ".repeat(pad)}${art}`);
		}
		return rows;
	}

	/**
	 * Hint row in the OpenCode `key label` style (bright chord, dim action):
	 * `⏎ send · ⌃J newline · ⌥K commands`. Remap-aware through the key-hint
	 * source; falls back to defaults pre-adoption. Shared by home and the
	 * session composer so both read the same way.
	 */
	#hintRow(safeWidth: number): string {
		const display = (
			key: "tui.input.submit" | "tui.input.newLine" | "app.commands.open",
			fallback: string,
		): string => {
			try {
				return this.#keyHintSource?.getDisplayString(key) || fallback;
			} catch {
				return fallback;
			}
		};
		const pair = (
			key: "tui.input.submit" | "tui.input.newLine" | "app.commands.open",
			fallback: string,
			label: string,
		): string => `${theme.fg("text", display(key, fallback))} ${theme.fg("muted", label)}`;
		const sep = theme.fg("muted", theme.sep.dot);
		const text = `${pair("tui.input.submit", "Enter", "send")}${sep}${pair("tui.input.newLine", "Ctrl+J", "newline")}${sep}${pair("app.commands.open", "Alt+K", "commands")}`;
		const centered = `${" ".repeat(Math.max(0, Math.floor((safeWidth - visibleWidth(text)) / 2)))}${text}`;
		return truncateToWidth(centered, Math.max(1, safeWidth), theme.getSymbolPreset() === "ascii" ? Ellipsis.Ascii : Ellipsis.Unicode);
	}

	#composeMainWithSidebar(
		mainRows: readonly string[],
		_header: readonly string[],
		mainWidth: number,
		totalWidth: number,
		safeRows: number,
		sidebarDocked: boolean,
		overlayActive: boolean,
	): readonly string[] {
		const fitRow = (row: string, width: number): string => {
			const w = visibleWidth(row);
			if (w === width) return row;
			if (w > width) return truncateToWidth(row, Math.max(0, width), theme.getSymbolPreset() === "ascii" ? Ellipsis.Ascii : Ellipsis.Unicode);
			return row + " ".repeat(Math.max(0, width - w));
		};
		// The focused editor emits CURSOR_MARKER (a private APC sentinel the
		// TUI strips after the frame). Width helpers bail on that sequence,
		// so measure/truncate the marker-free text and re-insert the marker
		// at its painted column when it survives the fit.
		const fitRowWithMarker = (row: string, width: number): string => {
			const markerAt = row.indexOf(CURSOR_MARKER);
			if (markerAt === -1) return fitRow(row, width);
			const markerCol = visibleWidth(row.slice(0, markerAt).split(CURSOR_MARKER).join(""));
			const fitted = fitRow(row.split(CURSOR_MARKER).join(""), width);
			if (markerCol >= width) return fitted;
			const before = sliceWithWidth(fitted, 0, markerCol).text;
			const after = sliceWithWidth(fitted, markerCol, Math.max(0, width - markerCol)).text;
			return `${before}${CURSOR_MARKER}${after}`;
		};
		if (sidebarDocked && this.#workspaceSidebar !== undefined) {
			const sidebarWidth = Math.max(1, totalWidth - mainWidth);
			const sidebarRows = this.#workspaceSidebar.render(sidebarWidth, safeRows);
			const height = Math.min(safeRows, Math.max(mainRows.length, sidebarRows.length));
			const out: string[] = [];
			for (let i = 0; i < height; i++) {
				const left = fitRowWithMarker(mainRows[i] ?? "", mainWidth);
				const right = fitRow(sidebarRows[i] ?? "", sidebarWidth);
				out.push(`${left}${right}`);
			}
			return out;
		}
		if (overlayActive && this.#workspaceSidebar !== undefined) {
			const overlayWidth = sidebarOverlayWidth(totalWidth);
			const sidebarRows = this.#workspaceSidebar.render(overlayWidth, safeRows);
			const out = [...mainRows];
			for (let i = 0; i < Math.min(out.length, sidebarRows.length); i++) {
				const base = out[i] ?? "";
				const overlay = sidebarRows[i] ?? "";
				const start = Math.max(0, totalWidth - overlayWidth);
				out[i] = `${fitRowWithMarker(base, start)}${fitRow(overlay, overlayWidth)}`;
			}
			return out;
		}
		return mainRows;
	}
	/**
	 * Mounted-runtime rows for the transient resize buffer. Only the trailing
	 * viewport can survive the caller's bottom slice, so the transcript renders
	 * a bounded tail instead of the full committed ledger, and the chrome above
	 * it renders only when that tail underfills the screen.
	 */
	#renderResizeTail(width: number, rows: number): string[] {
		const roots = [...this.#runtimeChildren, this.#statusHost];
		const transcriptIndex = roots.findIndex(root => root instanceof TranscriptContainer);
		if (transcriptIndex < 0) return this.#renderFixedRoots(roots, width, rows);
		const transcript = roots[transcriptIndex] as TranscriptContainer;
		const after = this.#renderFixedRoots(roots.slice(transcriptIndex + 1), width, rows);
		const transcriptRows = transcript.renderTail(width, Math.max(0, rows - after.length));
		const pre =
			transcriptRows.length + after.length >= rows ? [] : this.#renderRoots(roots.slice(0, transcriptIndex), width);
		return [...pre, ...transcriptRows, ...after];
	}

	/** Reflow accepted hard rows exactly as the restored terminal buffer will. */
	#reflowRetiredHeader(width: number, start: number): string[] {
		const lines = this.#retiredHeaderRows;
		if (!lines) return [];
		if (isInsideTerminalMultiplexer()) return lines.slice(start);
		const reflowed: string[] = [];
		const columns = Math.max(1, width);
		for (let index = start; index < lines.length; index++) {
			const line = lines[index]!;
			const lineWidth = visibleWidth(line);
			if (lineWidth === 0) {
				reflowed.push("");
				continue;
			}
			for (let column = 0; column < lineWidth;) {
				let slice = sliceWithWidth(line, column, columns, true);
				if (slice.width === 0) slice = sliceWithWidth(line, column, columns);
				reflowed.push(slice.text);
				column += Math.max(1, slice.width);
			}
		}
		return reflowed;
	}

	/** Live editor whose draft survives startup and session adoption. */
	get editor(): CustomEditor {
		return this.#editor;
	}

	/** The welcome component currently mounted in the header, if quiet mode is off. */
	get welcome(): WelcomeComponent | undefined {
		return this.#welcome;
	}

	/** Whether this composer already owns the terminal render/input loop. */
	get started(): boolean {
		return this.#started && !this.#stopped;
	}

	/** Start terminal ownership and optionally begin the welcome intro. */
	start(options: ComposerStartOptions = {}): void {
		if (this.#started || this.#stopped) return;
		this.#started = true;
		this.ui.start({ clearScrollback: options.clearScrollback === true, deferInput: options.deferInput === true });
		if (options.playWelcomeIntro !== false) this.playWelcomeIntro();
	}
	/** Take raw-input ownership after a deferred-input start. Idempotent. */
	enableInput(): void {
		if (this.#stopped) return;
		this.ui.enableInput();
	}

	/** Apply settings changes without replacing the editor or welcome component. */
	setKeyHintSource(source: Pick<KeybindingsManager, "getDisplayString"> | undefined): void {
		this.#keyHintSource = source;
	}
	setPreferences(update: Partial<ComposerPreferences>): void {
		if (this.#stopped) return;
		const wasQuiet = this.#preferences.quiet;
		this.#preferences = { ...this.#preferences, ...update };
		if (this.#preferences.fullscreen) this.#welcome?.stopIntro();
		this.ui.setBaseFullscreen(this.#preferences.fullscreen === true);
		this.editor.setTheme(getEditorTheme());
		try {
			this.editor.setBorderStyle(this.#workspaceComposerShape());
		} catch {
			// Extension-defined styles arrive with the session; InteractiveMode reapplies them.
		}
		this.ui.setShowHardwareCursor(this.#preferences.showHardwareCursor);
		this.editor.setUseTerminalCursor(this.ui.getShowHardwareCursor());
		this.ui.setMaxInlineImages(this.#preferences.maxInlineImages);
		if (update.resizeScrollback !== undefined) this.ui.setResizeScrollback(update.resizeScrollback);
		this.editor.setImeSafeCursorLayout(this.#preferences.imeSafeCursor);
		this.editor.setAutocompleteMaxVisible(this.#preferences.autocompleteMaxVisible);
		this.editor.setSpellingFeatures({
			typoDetection: this.#preferences.spellingTypoDetection,
			autocomplete: this.#preferences.spellingAutocomplete,
			autocorrect: this.#preferences.spellingAutocorrect,
		});
		this.#applyStatusSnapshot();
		if (this.#preferences.quiet) {
			this.#welcome?.stopIntro();
			this.#welcome = undefined;
		} else {
			this.#ensureWelcome();
			this.#welcome?.invalidate();
			if (wasQuiet && this.#started) this.playWelcomeIntro();
		}
		if (wasQuiet !== this.#preferences.quiet) this.#rebuildHeader();
		this.ui.requestRender();
	}

	/** Patch welcome data in place as model, session, and project discovery complete. */
	updateWelcome(update: ComposerWelcomeUpdate): void {
		if (this.#stopped) return;
		this.#applyWelcomeUpdate(update);
		if (this.#preferences.quiet) return;
		this.#ensureWelcome();
		const welcome = this.#welcome;
		if (!welcome) return;
		if (update.version !== undefined) welcome.setVersion(this.#version);
		if (update.modelName !== undefined || update.providerName !== undefined) {
			welcome.setModel(this.#modelName, this.#providerName);
		}
		if (update.recentSessions !== undefined) welcome.setRecentSessions(this.#recentSessions);
		if (update.lspServers !== undefined) welcome.setLspServers(this.#lspServers);
		this.ui.requestRender();
	}

	/** Replace optional header content around the stable welcome scene. */
	setHeaderExtras(before: readonly Component[], after: readonly Component[]): void {
		if (this.#stopped) return;
		this.#headerBefore = before;
		this.#headerAfter = after;
		this.#rebuildHeader();
		this.ui.requestRender();
	}

	/** Update the canonical editor reference after InteractiveMode remounts a custom editor. */
	setEditor(editor: CustomEditor): void {
		this.#editor = editor;
	}

	/**
	 * Mount the session-aware status component into the slot below the editor.
	 * Drops the speculative snapshot; the caller installs the real top-border
	 * provider through its composer-shape sync.
	 */
	setStatusComponent(component: Component): void {
		this.#statusHost.setComponent(component);
		this.#statusSnapshot = undefined;
		this.editor.setTopBorderProvider(undefined);
	}

	/** Cached placeholder top-border content fitted to the current editor width. */
	#speculativeTopBorder(availableWidth: number): EditorTopBorder | undefined {
		const border = this.#statusSnapshot?.topBorder;
		if (!border) return undefined;
		if (border.width <= availableWidth) return { content: border.content, width: border.width };
		const content = truncateToWidth(border.content, availableWidth);
		return { content, width: visibleWidth(content) };
	}

	/** Install the cached chrome for the current shape; a shape mismatch clears it. */
	#applyStatusSnapshot(): void {
		if (this.#statusHost.mounted) return;
		const snapshot = this.#statusSnapshot;
		if (!snapshot || snapshot.shape !== this.#preferences.composerShape) {
			this.editor.setTopBorderProvider(undefined);
			this.#statusHost.setLines([]);
			return;
		}
		if (snapshot.borderColor) {
			const { prefix, suffix } = snapshot.borderColor;
			this.editor.borderColor = text => `${prefix}${text}${suffix}`;
		}
		this.editor.setTopBorderProvider(
			snapshot.topBorder ? availableWidth => this.#speculativeTopBorder(availableWidth) : undefined,
		);
		this.#statusHost.setLines(snapshot.bottomLines);
	}

	/** Mount or replace session-aware root children while preserving the header and status hosts. */
	setRuntimeChildren(children: readonly Component[]): void {
		if (this.#stopped) return;
		this.ui.removeChild(this.#statusHost);
		if (this.#runtimeMounted) {
			for (const child of this.#runtimeChildren) this.ui.removeChild(child);
		} else {
			this.ui.removeChild(this.#bootstrapInputGap);
			this.ui.removeChild(this.editor);
			this.#runtimeMounted = true;
		}
		this.#runtimeChildren = children;
		for (const child of children) this.ui.addChild(child);
		this.ui.addChild(this.#statusHost);
		this.ui.requestRender();
	}

	/** Play or replay the welcome intro against the stable header render target. */
	playWelcomeIntro(): void {
		if (this.#preferences.fullscreen) return;
		this.#welcome?.playIntro(() => this.ui.requestComponentRender(this.#header));
	}

	/** Transfer terminal ownership to InteractiveMode without stopping the composer. */
	transfer(): void {
		if (!this.#started || this.#stopped || this.#transferred) {
			throw new Error("Composer is not available for transfer");
		}
		this.#transferred = true;
	}

	/** Stop a composer that has not transferred terminal ownership. */
	stop(): void {
		if (!this.#started || this.#stopped || this.#transferred) return;
		this.#welcome?.stopIntro();
		this.ui.stop();
		this.#stopped = true;
	}

	#applyWelcomeUpdate(update: ComposerWelcomeUpdate): void {
		if (update.version !== undefined) this.#version = update.version;
		if (update.modelName !== undefined) this.#modelName = update.modelName;
		if (update.providerName !== undefined) this.#providerName = update.providerName;
		if (update.recentSessions !== undefined) this.#recentSessions = [...update.recentSessions];
		if (update.lspServers !== undefined) this.#lspServers = [...update.lspServers];
	}

	#ensureWelcome(): void {
		this.#welcome ??= new WelcomeComponent(
			this.#version,
			this.#modelName,
			this.#providerName,
			this.#recentSessions,
			this.#lspServers,
		);
	}

	#rebuildHeader(): void {
		this.#header.clear();
		for (const component of this.#headerBefore) this.#header.addChild(component);
		if (this.#welcome) {
			this.#header.addChild(new Spacer(1));
			this.#header.addChild(this.#welcome);
			this.#header.addChild(new Spacer(1));
		}
		for (const component of this.#headerAfter) this.#header.addChild(component);
	}

	#handleInterrupt(): void {
		const now = this.#now();
		if (now - this.#lastInterruptAt < DOUBLE_INTERRUPT_MS) {
			this.#requestExit(130);
			return;
		}
		this.editor.setText("");
		this.#lastInterruptAt = now;
	}

	#requestExit(code: number): void {
		// Remains live after transfer until InteractiveMode installs its configured handlers.
		if (this.#stopped) return;
		this.#welcome?.stopIntro();
		if (this.#started) this.ui.stop();
		this.#stopped = true;
		this.#exit(code);
	}
}
