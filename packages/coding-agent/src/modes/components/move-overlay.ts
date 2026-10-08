/**
 * `/move` overlay: a path input with live directory autocomplete.
 *
 * Rendered as a centered modal via `showHookCustom(..., { overlay: true })`.
 * The user types a path, Tab autocomtes the highlighted directory, and Enter
 * confirms — yielding the resolved directory string (or `undefined` on cancel).
 */
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { type Component, type Focusable, Input, Key, matchesKey } from "@harvest/pi-tui";
import { theme } from "../theme/theme";
import { matchesSelectCancel, matchesSelectDown, matchesSelectUp } from "../utils/keybinding-matchers";
import { editorKey } from "./keybinding-hints";
import { fit, renderDialog } from "./overlay-box";

export interface MoveOverlayResult {
	directory: string;
}

interface DirEntry {
	/** Full absolute path. */
	value: string;
	/** Display label (basename + trailing slash). */
	label: string;
}

const MAX_RESULTS = 15;

/** TTL for the directory listing cache (ms). */
const DIR_CACHE_TTL = 500;
const dirCache = new Map<string, { time: number; entries: fs.Dirent[] }>();

function readDirCached(dir: string): fs.Dirent[] {
	const now = Date.now();
	const cached = dirCache.get(dir);
	if (cached && now - cached.time < DIR_CACHE_TTL) return cached.entries;
	try {
		const entries = fs.readdirSync(dir, { withFileTypes: true });
		dirCache.set(dir, { time: now, entries });
		return entries;
	} catch {
		return [];
	}
}

/**
 * `Dirent.isDirectory()` reports the entry type, not the link target, so a
 * `statSync` fallback is still needed for symlinks that point at a directory.
 * Some filesystems (NFS, FUSE, older SMB) report `UV_DIRENT_UNKNOWN` — every
 * `isX()` returns false — so those entries also fall back to `statSync` rather
 * than being silently dropped from the results.
 */
function entryIsDirectory(dir: string, entry: fs.Dirent): boolean {
	if (entry.isDirectory()) return true;
	// Fast reject only for entry types we can confidently identify as non-directory.
	if (entry.isFile() || entry.isBlockDevice() || entry.isCharacterDevice() || entry.isFIFO() || entry.isSocket()) {
		return false;
	}
	// Symlink (need target type) or unknown (filesystem didn't provide a type) — stat to find out.
	try {
		return fs.statSync(path.join(dir, entry.name)).isDirectory();
	} catch {
		return false;
	}
}

/** Resolve a user-typed path (`~`, absolute, or relative to `cwd`) to an absolute path. */
export function resolveMovePath(input: string, cwd: string): string {
	const trimmed = input.trim();
	if (trimmed === "~") return os.homedir();
	if (trimmed.startsWith("~/")) return path.join(os.homedir(), trimmed.slice(2));
	if (path.isAbsolute(trimmed)) return path.normalize(trimmed);
	return path.resolve(cwd, trimmed);
}

/** If `input` resolves to an existing directory, return it; otherwise `null`. */
export function resolveExistingDirectory(input: string, cwd: string): string | null {
	const resolved = resolveMovePath(input, cwd);
	try {
		return fs.statSync(resolved).isDirectory() ? resolved : null;
	} catch {
		return null;
	}
}

function listChildDirectories(dirPath: string, max: number, includeHidden = false): DirEntry[] {
	const results: DirEntry[] = [];
	const entries = readDirCached(dirPath);
	for (const entry of entries) {
		if (results.length >= max) break;
		const { name } = entry;
		if (!includeHidden && name.startsWith(".")) continue;
		if (!entryIsDirectory(dirPath, entry)) continue;
		results.push({ value: path.join(dirPath, name), label: `${name}/` });
	}
	results.sort((a, b) => a.label.localeCompare(b.label));
	return results;
}

function searchDirectories(prefix: string, cwd: string, max: number): DirEntry[] {
	if (!prefix) return listChildDirectories(cwd, max);

	// Split into base dir + query so dot-prefixed segments can reveal hidden directories.
	const norm = prefix.replace(/\\/g, "/");
	const slashIdx = norm.lastIndexOf("/");
	let baseDir: string;
	let query: string;
	if (slashIdx === -1) {
		baseDir = cwd;
		query = prefix;
	} else {
		const base = norm.slice(0, slashIdx + 1);
		query = norm.slice(slashIdx + 1);
		baseDir = resolveMovePath(base, cwd);
	}

	const includeHidden = query.startsWith(".");

	// If the prefix already resolves to an existing directory, list its children.
	// A dot-prefixed query is treated as a filter so hidden directories become reachable.
	const resolved = includeHidden ? null : resolveExistingDirectory(prefix, cwd);
	if (resolved) return listChildDirectories(resolved, max);

	const lower = query.toLowerCase();
	const results: DirEntry[] = [];
	const entries = readDirCached(baseDir);
	for (const entry of entries) {
		if (results.length >= max) break;
		const { name } = entry;
		if (!includeHidden && name.startsWith(".")) continue;
		if (query && !name.toLowerCase().includes(lower)) continue;
		if (!entryIsDirectory(baseDir, entry)) continue;
		results.push({ value: path.join(baseDir, name), label: `${name}/` });
	}
	return results;
}

/**
 * Overlay component for `/move`: a single-line path input with a live-filtered
 * list of matching directories. Tab accepts the highlighted suggestion; Enter
 * confirms the current input (or the highlighted suggestion if the input is
 * empty); Escape cancels.
 */
export class MoveOverlay implements Component, Focusable {
	#focused = false;
	#input = new Input();
	#maxHeight: number | undefined;
	#selectedIndex = 0;
	#results: DirEntry[] = [];
	#cwd: string;
	#done: (result: MoveOverlayResult | undefined) => void;

	constructor(cwd: string, done: (result: MoveOverlayResult | undefined) => void) {
		this.#cwd = cwd;
		this.#done = done;
		this.#input.prompt = theme.fg("muted", "Path: ");
		// Warm the cache for the current directory so the first keystroke is instant.
		readDirCached(cwd);
		this.#updateResults();
	}

	get focused(): boolean {
		return this.#focused;
	}

	set focused(value: boolean) {
		this.#focused = value;
	}

	handleInput(data: string): void {
		if (matchesSelectCancel(data) || matchesKey(data, Key.escape) || matchesKey(data, Key.ctrl("c"))) {
			this.#done(undefined);
			return;
		}
		if (matchesKey(data, Key.enter) || matchesKey(data, Key.return)) {
			this.#confirm();
			return;
		}
		if (matchesSelectUp(data) || matchesKey(data, Key.up)) {
			if (this.#results.length > 0) this.#selectedIndex = Math.max(0, this.#selectedIndex - 1);
			return;
		}
		if (matchesSelectDown(data) || matchesKey(data, Key.down)) {
			if (this.#results.length > 0)
				this.#selectedIndex = Math.min(this.#results.length - 1, this.#selectedIndex + 1);
			return;
		}
		if (matchesKey(data, Key.tab)) {
			const selected = this.#results[this.#selectedIndex];
			if (selected) {
				this.#input.setValue(selected.value);
				this.#selectedIndex = 0;
				this.#updateResults();
			}
			return;
		}
		const previous = this.#input.getValue();
		this.#input.handleInput(data);
		if (this.#input.getValue() !== previous) {
			this.#selectedIndex = 0;
			this.#updateResults();
		}
	}

	setMaxHeight(height: number): void {
		const next = Math.max(1, Math.floor(height));
		if (this.#maxHeight === next) return;
		this.#maxHeight = next;
	}

	pasteText(text: string): void {
		this.#input.pasteText(text);
		this.#selectedIndex = 0;
		this.#updateResults();
	}

	render(width: number): readonly string[] {
		const height = this.#maxHeight ?? Math.max(1, process.stdout.rows || 40);
		const chrome = Number(height >= 3) + Number(height >= 2) + Number(height >= 6);
		const bodyBudget = Math.max(1, height - chrome);
		const innerWidth = Math.max(1, width - 4);
		this.#input.focused = this.#focused;
		const input = this.#input.render(innerWidth)[0] ?? "";
		const visible = Math.max(1, Math.min(MAX_RESULTS, bodyBudget - 1));
		const start = Math.max(
			0,
			Math.min(this.#selectedIndex - Math.floor(visible / 2), this.#results.length - visible),
		);
		const choices = this.#results.slice(start, start + visible).map((item, index) => {
			const selected = start + index === this.#selectedIndex;
			const label = `${selected ? theme.nav.cursor : " "} ${item.label}`;
			return selected ? theme.bgFill("selectedBg", theme.fg("accent", fit(label, innerWidth))) : label;
		});
		const body = bodyBudget === 1 ? [choices[this.#selectedIndex - start] ?? input] : [input, ...choices];
		if (choices.length === 0 && bodyBudget > 1) body.push(theme.fg("muted", "No matching directories"));
		const footer = `${editorKey("tui.select.cancel") || "Esc"} cancel · Enter confirm · Tab accept`;
		return renderDialog("Move to directory", body, width, height, footer).lines;
	}

	invalidate(): void {}

	#updateResults(): void {
		this.#results = searchDirectories(this.#input.getValue(), this.#cwd, MAX_RESULTS + 5);
		if (this.#selectedIndex >= this.#results.length) {
			this.#selectedIndex = Math.max(0, this.#results.length - 1);
		}
	}

	#confirm(): void {
		const selected = this.#results[this.#selectedIndex];
		if (selected) {
			this.#done({ directory: selected.value });
			return;
		}
		if (this.#input.getValue().trim().length > 0) {
			this.#done({ directory: this.#input.getValue().trim() });
			return;
		}
		this.#done(undefined);
	}
}
