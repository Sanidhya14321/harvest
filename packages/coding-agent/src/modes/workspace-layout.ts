/**
 * Single geometry contract for the fullscreen workspace.
 * All dimensions, breakpoints, and spacing live here; components must not
 * spread arithmetic and magic numbers.
 */
export const WORKSPACE_LAYOUT = {
	/** Outer horizontal padding for the main column at normal sizes. */
	mainPaddingX: 2,
	/** Sidebar width in columns (OpenCode reference measurement). */
	sidebarWidth: 42,
	/** Inner padding inside the sidebar surface. */
	sidebarInnerPaddingX: 2,
	/** Auto-dock the sidebar when terminal width is greater than this. */
	sidebarDockBreakpoint: 120,
	/** Home prompt maximum width, clamped to available width. */
	homePromptMaxWidth: 75,
	/** Fullscreen empty-home centering cap (legacy 76 kept compatible). */
	homeCenterCap: 76,
	/** Dialog width caps for small/medium/large bounded panels. */
	dialogCaps: [60, 88, 116] as const,
	/** Minimum usable main content width when the sidebar is docked. */
	minMainContentWidth: 40,
	/** Maximum multiline input growth as a fraction of screen rows. */
	composerMaxFraction: 1 / 3,
	/** Minimum terminal sizes we qualify. */
	minUsableColumns: 20,
	minUsableRows: 4,
} as const;

export type SidebarPreference = "auto" | "show" | "hide";

export interface WorkspaceViewport {
	readonly columns: number;
	readonly rows: number;
}

export interface PaneRect {
	readonly x: number;
	readonly y: number;
	readonly width: number;
	readonly height: number;
}

export interface WorkspaceGeometry {
	readonly viewport: WorkspaceViewport;
	readonly hasConversation: boolean;
	/** True when the sidebar is painted docked on the right. */
	readonly sidebarDocked: boolean;
	/** True when the sidebar should be offered as a temporary overlay. */
	readonly sidebarOverlayRequested: boolean;
	readonly main: PaneRect;
	readonly sidebar: PaneRect | undefined;
	/** Width available to the composer/editor pane. */
	readonly composerWidth: number;
	/** Width available to transcript rows. */
	readonly transcriptWidth: number;
	/** Home prompt width clamped to the viewport. */
	readonly homePromptWidth: number;
	/** Clamp a dialog width to the viewport using 60/88/116 caps. */
	dialogWidth(preferred: 60 | 88 | 116): number;
	/** Translate a zero-based SGR mouse coordinate into main/sidebar ownership. */
	hitTest(column: number): "main" | "sidebar" | "outside";
}

function clampInt(value: number, min: number, max: number): number {
	if (!Number.isFinite(value)) return min;
	return Math.max(min, Math.min(max, Math.floor(value)));
}

/**
 * Centralize effective composer-shape resolution. Fullscreen maps the legacy
 * default `band` to `rail` so first paint, adoption, previews, runtime
 * changes, and cached chrome agree. Startup status caching previously used
 * the raw `band` value; callers must use this helper.
 */
export function resolveWorkspaceComposerShape(fullscreen: boolean, composerShape: string): string {
	if (fullscreen && composerShape === "band") return "rail";
	return composerShape;
}

/** Clamp a dialog width to the viewport using the shared caps. */
export function clampDialogWidth(columns: number, preferred: 60 | 88 | 116): number {
	const safe = Math.max(1, Math.floor(columns));
	const cap = Math.min(preferred, Math.max(0, safe - 2));
	return Math.max(1, cap);
}

/** Pick the largest dialog cap that fits, falling back to the smallest viable width. */
export function pickDialogCap(columns: number): 60 | 88 | 116 {
	if (columns >= 118) return 116;
	if (columns >= 90) return 88;
	return 60;
}

/** True when the sidebar overlay panel is actually painted (narrow `show` + open). */
export function sidebarOverlayVisible(
	columns: number,
	preference: SidebarPreference,
	overlayOpen: boolean,
): boolean {
	if (!overlayOpen) return false;
	if (preference !== "show") return false;
	const safe = Math.max(1, Math.floor(columns));
	const dockPossible =
		safe > WORKSPACE_LAYOUT.sidebarDockBreakpoint &&
		safe - WORKSPACE_LAYOUT.sidebarWidth >= WORKSPACE_LAYOUT.minMainContentWidth;
	return !dockPossible;
}

export interface ComputeWorkspaceLayoutOptions {
	readonly hasConversation: boolean;
	readonly sidebarPreference?: SidebarPreference;
	readonly sidebarOverlayOpen?: boolean;
}

export function computeWorkspaceLayout(
	viewport: WorkspaceViewport,
	options: ComputeWorkspaceLayoutOptions,
): WorkspaceGeometry {
	const columns = Math.max(1, Math.floor(viewport.columns));
	const rows = Math.max(0, Math.floor(viewport.rows));
	const preference = options.sidebarPreference ?? "auto";
	const overlayOpen = options.sidebarOverlayOpen ?? false;

	const sidebarWidth = WORKSPACE_LAYOUT.sidebarWidth;
	const minMain = WORKSPACE_LAYOUT.minMainContentWidth;
	let sidebarDocked = false;
	let sidebarOverlayRequested = false;

	if (preference === "hide") {
		sidebarDocked = false;
	} else if (preference === "show") {
		if (columns > WORKSPACE_LAYOUT.sidebarDockBreakpoint && columns - sidebarWidth >= minMain) {
			sidebarDocked = true;
		} else {
			// Narrow `show` requests an overlay instead of crushing the transcript.
			sidebarOverlayRequested = true;
		}
	} else {
		// auto: dock when wide, hide when narrow.
		if (columns > WORKSPACE_LAYOUT.sidebarDockBreakpoint && columns - sidebarWidth >= minMain) {
			sidebarDocked = true;
		}
	}

	const mainWidth = sidebarDocked ? Math.max(1, columns - sidebarWidth) : columns;
	const padX = columns < 40 ? 0 : WORKSPACE_LAYOUT.mainPaddingX;
	const transcriptWidth = Math.max(1, mainWidth - padX * 2);
	const composerWidth = Math.max(1, mainWidth - padX * 2);
	const homePromptWidth = Math.max(1, Math.min(WORKSPACE_LAYOUT.homePromptMaxWidth, Math.max(1, columns - padX * 2)));

	const main: PaneRect = { x: 0, y: 0, width: mainWidth, height: rows };
	let sidebar: PaneRect | undefined;
	if (sidebarDocked) {
		sidebar = { x: mainWidth, y: 0, width: columns - mainWidth, height: rows };
	}

	const geometry: WorkspaceGeometry = {
		viewport: { columns, rows },
		hasConversation: options.hasConversation,
		sidebarDocked,
		sidebarOverlayRequested: sidebarOverlayRequested && (overlayOpen || preference === "show"),
		main,
		sidebar,
		composerWidth,
		transcriptWidth,
		homePromptWidth,
		dialogWidth(preferred: 60 | 88 | 116): number {
			return clampDialogWidth(columns, preferred);
		},
		hitTest(column: number): "main" | "sidebar" | "outside" {
			if (column < 0 || column >= columns) return "outside";
			if (sidebarDocked && sidebar !== undefined && column >= sidebar.x) return "sidebar";
			if (sidebarOverlayVisible(columns, preference, overlayOpen)) {
				const overlayWidth = sidebarOverlayWidth(columns);
				if (column >= Math.max(0, columns - overlayWidth)) return "sidebar";
			}
			return "main";
		},
	};
	return geometry;
}

/** Overlay panel width: min(42, columns-2), full-width bounded panel on tiny screens. */
export function sidebarOverlayWidth(columns: number): number {
	const safe = Math.max(1, Math.floor(columns));
	if (safe <= 24) return Math.max(1, safe - 2);
	return Math.min(WORKSPACE_LAYOUT.sidebarWidth, Math.max(1, safe - 2));
}

/** Row budget helper: bound viewport rows and reserve chrome without negative sizes. */
export function budgetRows(total: number, reserved: number): number {
	return Math.max(0, Math.floor(total) - Math.max(0, Math.floor(reserved)));
}

/** Clamp editor max height to ~1/3 of the screen while keeping metadata room. */
export function composerMaxHeight(rows: number, reservedChrome = 4): number {
	const safe = Math.max(0, Math.floor(rows));
	const third = Math.floor(safe * WORKSPACE_LAYOUT.composerMaxFraction);
	const capped = Math.max(1, Math.min(third, Math.max(1, safe - reservedChrome)));
	if (safe <= 6) return Math.max(1, Math.min(2, safe - 1));
	return clampInt(capped, 1, Math.max(1, safe));
}
