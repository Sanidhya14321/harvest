/**
 * Search Code Tool - Dual-Stage Column-0 AST Declarations and Okapi BM25 Index Search.
 *
 * Exposes Harvest's dual-stage codebase retrieval:
 * - Extracts and queries column-0 declarations across 20+ file formats.
 * - Queries token-efficient section pages with Okapi BM25 ranking.
 */

import * as fs from "node:fs";
import * as path from "node:path";
import { type as arkType } from "@harvest/omptype";
import type { AgentTool, AgentToolContext, AgentToolResult } from "@harvest/pi-agent-core";
import { CodeIndex } from "../core/harvest/code-index";
import { KnowledgeRetriever } from "../core/harvest/retrieval";
import type { ToolSession } from "./index";

const searchCodeSchema = arkType({
	query: arkType("string").describe("Search query for symbol names, function signatures, or keywords in documentation"),
	"mode?": arkType("'symbols' | 'docs' | 'all'").describe("Search mode: 'symbols' for code declarations, 'docs' for section retrieval, or 'all' (default)"),
	"limit?": arkType("number").describe("Maximum number of results to return (default: 20)"),
});

export type SearchCodeInput = typeof searchCodeSchema.infer;

export interface SearchCodeDetails {
	readonly query: string;
	readonly totalMatches: number;
}

export class SearchCodeTool implements AgentTool<typeof searchCodeSchema, SearchCodeDetails> {
	readonly name = "search_code";
	readonly label = "Search Code";
	readonly description = "Search symbol declarations (functions, classes, interfaces) or documentation sections using Harvest's dual-stage retrieval engine.";
	readonly parameters = searchCodeSchema;
	readonly concurrency = "shared";
	readonly strict = true;

	readonly #codeIndex: CodeIndex;
	readonly #knowledgeRetriever: KnowledgeRetriever;
	readonly #workspaceRoot: string;

	constructor(private readonly session: ToolSession) {
		this.#workspaceRoot = session.cwd;
		this.#codeIndex = new CodeIndex(this.#workspaceRoot);
		this.#knowledgeRetriever = new KnowledgeRetriever(this.#workspaceRoot);
	}

	async execute(
		_toolCallId: string,
		params: SearchCodeInput,
		_signal?: AbortSignal,
		_onUpdate?: unknown,
		_context?: AgentToolContext,
	): Promise<AgentToolResult<SearchCodeDetails>> {
		const query = params.query.trim();
		const mode = params.mode ?? "all";
		const limit = params.limit ?? 20;

		const outputLines: string[] = [];
		let totalMatches = 0;

		if (mode === "symbols" || mode === "all") {
			let symbols = this.#codeIndex.searchSymbols(query, limit);
			if (symbols.length === 0) {
				// Scan workspace files on initial query
				try {
					const entries = fs.readdirSync(this.#workspaceRoot, { recursive: true }) as string[];
					this.#codeIndex.updateIndex(entries);
					symbols = this.#codeIndex.searchSymbols(query, limit);
				} catch {}
			}
			if (symbols.length > 0) {
				outputLines.push(`### Code Symbols (${symbols.length} matches):`);
				for (const sym of symbols.slice(0, limit)) {
					const relPath = path.relative(this.#workspaceRoot, sym.filePath) || sym.filePath;
					outputLines.push(`- **${sym.name}** (${sym.kind}) at \`${relPath}:${sym.lineNumber}\`\n  \`${sym.signature}\``);
				}
				totalMatches += symbols.length;
			}
		}

		if (mode === "docs" || mode === "all") {
			const docs = this.#knowledgeRetriever.search(query, undefined, limit);
			if (docs.length > 0) {
				if (outputLines.length > 0) outputLines.push("");
				outputLines.push(`### Documentation Sections (${docs.length} matches):`);
				for (const res of docs) {
					const relPath = path.relative(this.#workspaceRoot, res.page.filePath) || res.page.filePath;
					outputLines.push(`- **${res.page.title}** (\`${relPath}\`, score: ${res.score.toFixed(2)})\n  ${res.page.heading}\n  > ${res.page.content.slice(0, 150).replace(/\n/g, " ")}...`);
				}
				totalMatches += docs.length;
			}
		}

		if (outputLines.length === 0) {
			outputLines.push(`No matches found for query '${query}'.`);
		}

		return {
			content: [{ type: "text", text: outputLines.join("\n") }],
			details: { query, totalMatches },
		};
	}
}
