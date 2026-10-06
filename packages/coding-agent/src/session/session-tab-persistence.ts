import * as path from "node:path";
import { logger } from "@harvest/pi-utils";
import type { SessionStorage } from "./session-storage";
import type { SessionTabs } from "./session-tabs";

/** Versioned on-disk shape for a project's open-tab references. */
export interface PersistedSessionTabs {
	version: typeof SESSION_TABS_VERSION;
	/** Canonical project key these references belong to; mismatches are discarded. */
	projectKey: string;
	tabs: { path: string; label?: string; sessionId?: string }[];
	activePath?: string;
	activeSessionId?: string;
	recentlyClosed: { path: string; label?: string; sessionId?: string }[];
}

export const SESSION_TABS_VERSION = 2;
export const SESSION_TABS_FILENAME = "tabs.json";
/** Recently-closed entries retained across restarts. */
export const MAX_PERSISTED_CLOSED_TABS = 10;
/** Restored tabs beyond this count are dropped oldest-first. */
export const MAX_RESTORED_TABS = 50;

export function sessionTabsFile(sessionDir: string): string {
	return path.join(sessionDir, SESSION_TABS_FILENAME);
}

/**
 * Snapshot live tab state for persistence. Every tab carries its stable
 * session ID when the owner has noted one (via `SessionTabs.noteId`); the
 * active tab additionally falls back to the explicit `activeSessionId`.
 */
export function snapshotSessionTabs(
	tabs: SessionTabs,
	options: { projectKey: string; activePath?: string; activeSessionId?: string },
): PersistedSessionTabs {
	return {
		version: SESSION_TABS_VERSION,
		projectKey: options.projectKey,
		tabs: tabs.paths.map(tabPath => {
			const entry: { path: string; label?: string; sessionId?: string } = { path: tabPath };
			const label = tabs.label(tabPath);
			if (label) entry.label = label;
			const sessionId =
				tabs.idForPath(tabPath) ??
				(options.activePath && options.activeSessionId && tabPath === options.activePath
					? options.activeSessionId
					: undefined);
			if (sessionId) entry.sessionId = sessionId;
			return entry;
		}),
		activePath: options.activePath,
		activeSessionId: options.activeSessionId,
		recentlyClosed: tabs.recentlyClosed.slice(-MAX_PERSISTED_CLOSED_TABS),
	};
}

/**
 * Atomically persist tab references with private permissions. Failures are
 * logged, never thrown: tab navigation must not break because a sidecar
 * write hiccuped.
 */
export function saveSessionTabs(
	storage: Pick<SessionStorage, "writeTextSync">,
	file: string,
	snapshot: PersistedSessionTabs,
): void {
	try {
		storage.writeTextSync(file, JSON.stringify(snapshot));
	} catch (error) {
		logger.warn("Failed to persist session tabs", {
			file,
			error: error instanceof Error ? error.message : String(error),
		});
	}
}

/** Read and validate persisted tab references; undefined when absent or invalid. */
export async function loadSessionTabs(
	storage: Pick<SessionStorage, "readText">,
	file: string,
	projectKey: string,
): Promise<PersistedSessionTabs | undefined> {
	let raw: string;
	try {
		raw = await storage.readText(file);
	} catch {
		return undefined;
	}
	let parsed: unknown;
	try {
		parsed = JSON.parse(raw);
	} catch (error) {
		logger.warn("Ignoring corrupt session tabs file", {
			file,
			error: error instanceof Error ? error.message : String(error),
		});
		return undefined;
	}
	if (!isObject(parsed) || (parsed.version !== SESSION_TABS_VERSION && parsed.version !== 1)) return undefined;
	if (parsed.projectKey !== projectKey || !Array.isArray(parsed.tabs)) return undefined;
	const tabs = parsed.tabs.filter(isTabRef).slice(-MAX_RESTORED_TABS);
	const recentlyClosed = Array.isArray(parsed.recentlyClosed)
		? parsed.recentlyClosed.filter(isTabRef).slice(-MAX_PERSISTED_CLOSED_TABS)
		: [];
	const activePath = typeof parsed.activePath === "string" ? parsed.activePath : undefined;
	const activeSessionId = typeof parsed.activeSessionId === "string" ? parsed.activeSessionId : undefined;
	// v1 payloads normalize forward: missing IDs stay absent, never fabricated.
	return { version: SESSION_TABS_VERSION, projectKey, tabs, activePath, activeSessionId, recentlyClosed };
}

function isObject(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null;
}

function isTabRef(value: unknown): value is { path: string; label?: string; sessionId?: string } {
	if (!isObject(value) || typeof value.path !== "string" || !value.path) return false;
	if (value.label !== undefined && typeof value.label !== "string") return false;
	if (value.sessionId !== undefined && typeof value.sessionId !== "string") return false;
	return true;
}
