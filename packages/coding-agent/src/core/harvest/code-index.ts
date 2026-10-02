/**
 * Symbol-Level Source Code Index & Column-0 AST Declaration Parser.
 *
 * Scans 20+ file extensions across 12 languages.
 * Extracts functions, classes, interfaces, types, and structs with column-0 regexes.
 * Splits identifiers on CamelCase/PascalCase boundaries for subword search.
 * Caches file fingerprints (mtimeMs, size) in .harvest/code-index.json.
 */

import * as fs from "node:fs";
import * as path from "node:path";
import { globPaths, isRecord, logger, untilAborted } from "@harvest/pi-utils";
import { SecuritySandbox } from "./security";

export interface SymbolDeclaration {
	readonly name: string;
	readonly kind: "function" | "class" | "interface" | "type" | "struct" | "enum";
	readonly subwords: readonly string[];
	readonly lineNumber: number;
	readonly column: number;
	readonly signature: string;
	readonly filePath: string;
}

export interface CachedFileFingerprint {
	readonly mtimeMs: number;
	readonly size: number;
	readonly symbols: readonly SymbolDeclaration[];
}

export interface CodeIndexData {
	readonly version: number;
	readonly files: Record<string, CachedFileFingerprint>;
}

export const SUPPORTED_EXTENSIONS = new Set([
	".ts",
	".tsx",
	".js",
	".jsx",
	".py",
	".rs",
	".go",
	".java",
	".c",
	".cpp",
	".h",
	".hpp",
	".cs",
	".rb",
	".php",
	".swift",
	".kt",
	".scala",
	".sh",
	".sql",
]);

/**
 * Split CamelCase, PascalCase, or snake_case identifiers into individual words.
 * e.g. "parseUserAuthToken" -> ["parse", "user", "auth", "token"]
 */
export function splitIdentifierSubwords(ident: string): string[] {
	return ident
		.replace(/([a-z0-9])([A-Z])/g, "$1 $2")
		.replace(/([A-Z]+)([A-Z][a-z])/g, "$1 $2")
		.replace(/[_\-.]+/g, " ")
		.toLowerCase()
		.split(/\s+/)
		.filter(w => w.length > 0);
}

const DECLARATION_REGEXES = [
	// Functions (TS/JS, Go, Rust, Python, Kotlin, Swift, PHP)
	{
		kind: "function" as const,
		re: /^(?:export\s+)?(?:async\s+)?(?:public\s+|private\s+|protected\s+|static\s+)?function\s+([a-zA-Z_$][a-zA-Z0-9_$]*)/m,
	},
	{
		kind: "function" as const,
		re: /^(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+([a-zA-Z_$][a-zA-Z0-9_$]*)/m,
	},
	{
		kind: "function" as const,
		re: /^(?:func\s+(?:\([^)]*\)\s+)?)([a-zA-Z_$][a-zA-Z0-9_$]*)/m,
	},
	{
		kind: "function" as const,
		re: /^def\s+([a-zA-Z_$][a-zA-Z0-9_$]*)/m,
	},

	// Classes
	{
		kind: "class" as const,
		re: /^(?:export\s+)?(?:abstract\s+)?class\s+([a-zA-Z_$][a-zA-Z0-9_$]*)/m,
	},

	// Interfaces / Types / Structs / Enums
	{
		kind: "interface" as const,
		re: /^(?:export\s+)?interface\s+([a-zA-Z_$][a-zA-Z0-9_$]*)/m,
	},
	{
		kind: "type" as const,
		re: /^(?:export\s+)?type\s+([a-zA-Z_$][a-zA-Z0-9_$]*)\s*=/m,
	},
	{
		kind: "struct" as const,
		re: /^(?:pub(?:\([^)]*\))?\s+)?(?:struct|enum|trait)\s+([a-zA-Z_$][a-zA-Z0-9_$]*)/m,
	},
	{
		kind: "enum" as const,
		re: /^(?:export\s+)?enum\s+([a-zA-Z_$][a-zA-Z0-9_$]*)/m,
	},
];

/**
 * Extract column-0 symbol declarations from source code text.
 */
export function extractSymbolsFromContent(content: string, filePath: string): SymbolDeclaration[] {
	const lines = content.split("\n");
	const symbols: SymbolDeclaration[] = [];

	for (let i = 0; i < lines.length; i++) {
		const line = lines[i];
		// Only check lines that start at column 0 (no leading whitespace)
		if (!line || line.startsWith(" ") || line.startsWith("\t") || line.startsWith("//") || line.startsWith("#")) {
			continue;
		}

		for (const { kind, re } of DECLARATION_REGEXES) {
			const m = line.match(re);
			if (m && m[1]) {
				const name = m[1];
				const subwords = splitIdentifierSubwords(name);
				symbols.push({
					name,
					kind,
					subwords,
					lineNumber: i + 1,
					column: 0,
					signature: line.trim().slice(0, 120),
					filePath,
				});
				break;
			}
		}
	}

	return symbols;
}

export class CodeIndex {
	readonly #workspaceRoot: string;
	readonly #cacheFilePath: string;
	readonly #sandbox: SecuritySandbox;
	#index: CodeIndexData = { version: 1, files: {} };
	#loadPromise: Promise<void> | undefined;
	#refreshPromise: Promise<void> | undefined;
	#refreshController: AbortController | undefined;
	#refreshUsers = 0;
	#cacheRevision = 0;
	#savedCacheRevision = 0;

	constructor(workspaceRoot: string = process.cwd()) {
		this.#sandbox = new SecuritySandbox(workspaceRoot);
		this.#workspaceRoot = this.#sandbox.assertPathJailed(".").resolvedPath;
		const harvestDir = path.join(this.#workspaceRoot, ".harvest");
		this.#cacheFilePath = path.join(harvestDir, "code-index.json");
	}

	async #loadCache(): Promise<void> {
		try {
			const jail = this.#sandbox.assertPathJailed(this.#cacheFilePath);
			if (!jail.jailed) return;
			const file = Bun.file(jail.resolvedPath);
			if ((await fs.promises.stat(jail.resolvedPath)).size > 16 * 1024 * 1024) return;
			const value: unknown = await file.json();
			if (!isRecord(value) || value.version !== 1 || !isRecord(value.files)) return;
			// Cached declarations are untrusted. Refresh from source before serving
			// workspace searches; malformed records must not crash ranking.
			const files: Record<string, CachedFileFingerprint> = Object.create(null);
			for (const [name, entry] of Object.entries(value.files)) {
				if (
					!isRecord(entry) ||
					typeof entry.mtimeMs !== "number" ||
					typeof entry.size !== "number" ||
					!Array.isArray(entry.symbols)
				)
					continue;
				const symbols = entry.symbols.filter(
					(symbol): symbol is SymbolDeclaration =>
						isRecord(symbol) &&
						typeof symbol.name === "string" &&
						typeof symbol.signature === "string" &&
						typeof symbol.filePath === "string" &&
						symbol.filePath === name &&
						Array.isArray(symbol.subwords) &&
						symbol.subwords.every(word => typeof word === "string") &&
						Number.isInteger(symbol.lineNumber) &&
						typeof symbol.column === "number" &&
						["function", "class", "interface", "type", "struct", "enum"].includes(String(symbol.kind)),
				);
				if (symbols.length !== entry.symbols.length) continue;
				files[name] = { mtimeMs: entry.mtimeMs, size: entry.size, symbols };
			}
			this.#index = { version: 1, files };
		} catch {}
	}

	async #ensureLoaded(): Promise<void> {
		this.#loadPromise ??= this.#loadCache();
		await this.#loadPromise;
	}

	async #saveCache(): Promise<void> {
		if (this.#cacheRevision === this.#savedCacheRevision) return;
		const revision = this.#cacheRevision;
		try {
			const jail = this.#sandbox.assertPathJailed(this.#cacheFilePath);
			if (!jail.jailed) {
				logger.warn("Code index cache path rejected", { reason: jail.error });
				return;
			}
			const dir = path.dirname(jail.resolvedPath);
			await fs.promises.mkdir(dir, { recursive: true });
			const current = this.#sandbox.assertPathJailed(jail.resolvedPath);
			if (!current.jailed) return;
			await Bun.write(current.resolvedPath, JSON.stringify(this.#index));
			this.#savedCacheRevision = revision;
		} catch {}
	}

	/**
	 * Index a single file, using cached symbols if mtime and size match.
	 */
	async indexFile(relPath: string, signal?: AbortSignal): Promise<SymbolDeclaration[]> {
		await this.#ensureLoaded();
		signal?.throwIfAborted();
		const jail = this.#sandbox.assertPathJailed(path.resolve(this.#workspaceRoot, relPath));
		if (!jail.jailed) {
			if (Object.hasOwn(this.#index.files, relPath)) this.#cacheRevision++;
			delete this.#index.files[relPath];
			return [];
		}
		const fullPath = jail.resolvedPath;
		try {
			const stat = await fs.promises.stat(fullPath);
			signal?.throwIfAborted();
			const cached = this.#index.files[relPath];

			if (!stat.isFile() || stat.size > 2 * 1024 * 1024) {
				if (Object.hasOwn(this.#index.files, relPath)) this.#cacheRevision++;
				delete this.#index.files[relPath];
				return [];
			}
			if (cached && cached.mtimeMs === stat.mtimeMs && cached.size === stat.size) {
				return cached.symbols as SymbolDeclaration[];
			}
			const content = await Bun.file(fullPath).text();
			signal?.throwIfAborted();
			const symbols = extractSymbolsFromContent(content, relPath);

			this.#index.files[relPath] = {
				mtimeMs: stat.mtimeMs,
				size: stat.size,
				symbols,
			};
			this.#cacheRevision++;

			return symbols;
		} catch {
			signal?.throwIfAborted();
			if (Object.hasOwn(this.#index.files, relPath)) this.#cacheRevision++;
			delete this.#index.files[relPath];
			return [];
		}
	}

	/**
	 * Scan and update index for given paths (or scanned workspace files).
	 */
	async updateIndex(files: string[], signal?: AbortSignal): Promise<void> {
		await this.#ensureLoaded();
		for (let offset = 0; offset < files.length; offset += 4) {
			signal?.throwIfAborted();
			await Promise.all(
				files.slice(offset, offset + 4).map(async file => {
					if (SUPPORTED_EXTENSIONS.has(path.extname(file).toLowerCase())) await this.indexFile(file, signal);
				}),
			);
		}
		await this.#saveCache();
	}

	/** Coalesce concurrent queries, refresh fingerprints and evict removed files. */
	async refreshWorkspace(signal?: AbortSignal): Promise<void> {
		signal?.throwIfAborted();
		if (this.#refreshController?.signal.aborted && this.#refreshPromise) {
			await untilAborted(
				signal,
				this.#refreshPromise.catch(() => {}),
			);
		}
		if (!this.#refreshPromise) {
			this.#refreshController = new AbortController();
			const operationSignal = AbortSignal.any([this.#refreshController.signal, AbortSignal.timeout(15_000)]);
			this.#refreshPromise = (async () => {
				await this.#ensureLoaded();
				operationSignal.throwIfAborted();
				const files = await globPaths(
					`**/*.{${[...SUPPORTED_EXTENSIONS].map(extension => extension.slice(1)).join(",")}}`,
					{
						cwd: this.#workspaceRoot,
						timeoutMs: 5000,
						dot: true,
						signal: operationSignal,
						exclude: ["**/.harvest/**", "**/dist/**", "**/target/**", "**/coverage/**"],
					},
				);
				const present = new Set(files);
				for (const name of Object.keys(this.#index.files)) {
					if (!present.has(name)) {
						delete this.#index.files[name];
						this.#cacheRevision++;
					}
				}
				await this.updateIndex(files, operationSignal);
			})();
			void this.#refreshPromise
				.finally(() => {
					this.#refreshPromise = undefined;
				})
				.catch(() => {});
		}
		this.#refreshUsers++;
		try {
			await untilAborted(signal, this.#refreshPromise);
		} finally {
			if (--this.#refreshUsers === 0) this.#refreshController?.abort();
		}
	}

	/**
	 * Search indexed symbols by name or subwords.
	 * e.g. Query "auth token" matches "parseUserAuthToken".
	 */
	async searchSymbols(query: string, limit: number = 10): Promise<SymbolDeclaration[]> {
		await this.#ensureLoaded();
		const qSubwords = splitIdentifierSubwords(query);
		if (qSubwords.length === 0) return [];

		const matches: Array<{ symbol: SymbolDeclaration; score: number }> = [];

		for (const fileData of Object.values(this.#index.files)) {
			for (const sym of fileData.symbols) {
				let score = 0;

				// Exact name match
				if (sym.name.toLowerCase() === query.toLowerCase().trim()) {
					score += 100;
				}

				// Subword overlap
				let matchedSubwords = 0;
				for (const qw of qSubwords) {
					if (sym.subwords.includes(qw)) {
						matchedSubwords++;
					}
				}

				if (matchedSubwords > 0) {
					score += matchedSubwords * 10;
					if (matchedSubwords === qSubwords.length) {
						score += 20; // all query words matched
					}
				}

				if (score > 0) {
					matches.push({ symbol: sym, score });
				}
			}
		}

		matches.sort((a, b) => b.score - a.score);
		return matches.slice(0, limit).map(m => m.symbol);
	}
}
