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
		.filter((w) => w.length > 0);
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
	#index: CodeIndexData;

	constructor(workspaceRoot: string = process.cwd()) {
		this.#workspaceRoot = path.resolve(workspaceRoot);
		const harvestDir = path.join(this.#workspaceRoot, ".harvest");
		this.#cacheFilePath = path.join(harvestDir, "code-index.json");
		this.#index = this.#loadCache();
	}

	#loadCache(): CodeIndexData {
		try {
			if (fs.existsSync(this.#cacheFilePath)) {
				const raw = fs.readFileSync(this.#cacheFilePath, "utf8");
				return JSON.parse(raw);
			}
		} catch {}
		return { version: 1, files: {} };
	}

	#saveCache(): void {
		try {
			const dir = path.dirname(this.#cacheFilePath);
			fs.mkdirSync(dir, { recursive: true });
			fs.writeFileSync(this.#cacheFilePath, JSON.stringify(this.#index, null, 2), "utf8");
		} catch {}
	}

	/**
	 * Index a single file, using cached symbols if mtime and size match.
	 */
	indexFile(relPath: string): SymbolDeclaration[] {
		const fullPath = path.join(this.#workspaceRoot, relPath);
		try {
			const stat = fs.statSync(fullPath);
			const cached = this.#index.files[relPath];

			if (cached && cached.mtimeMs === stat.mtimeMs && cached.size === stat.size) {
				return cached.symbols as SymbolDeclaration[];
			}

			const content = fs.readFileSync(fullPath, "utf8");
			const symbols = extractSymbolsFromContent(content, relPath);

			this.#index = {
				...this.#index,
				files: {
					...this.#index.files,
					[relPath]: {
						mtimeMs: stat.mtimeMs,
						size: stat.size,
						symbols,
					},
				},
			};

			return symbols;
		} catch {
			return [];
		}
	}

	/**
	 * Scan and update index for given paths (or scanned workspace files).
	 */
	updateIndex(files: string[]): void {
		for (const file of files) {
			const ext = path.extname(file).toLowerCase();
			if (SUPPORTED_EXTENSIONS.has(ext)) {
				this.indexFile(file);
			}
		}
		this.#saveCache();
	}

	/**
	 * Search indexed symbols by name or subwords.
	 * e.g. Query "auth token" matches "parseUserAuthToken".
	 */
	searchSymbols(query: string, limit: number = 10): SymbolDeclaration[] {
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
		return matches.slice(0, limit).map((m) => m.symbol);
	}
}
