/**
 * Context Compression Engine with Local and Global Memory.
 *
 * Optimizes context compression by performing vectorless dual-stage RAG
 * over both local memory (session working memory, read/modified files, verifications)
 * and global memory (.harvest/ patterns, anti-patterns, graph relationships),
 * bounded dynamically with respect to model tier prompt budgets.
 */

import {
	DeterministicCompactor,
	type CompactedSummary,
	type ExtractedTranscriptOperations,
	type TranscriptToolCall,
	type VerificationAuditRecord,
} from "../compaction/compaction";
import type { HarvestKnowledgeGraph } from "./graph";
import type { LocalMemoryStore, LocalSessionMemory } from "./memory";
import { type ContextTier, type TierBudgetConfig, getTierBudget } from "./model-tier";
import { type SectionPage, type UnifiedRetrievalResult, tokenize, UnifiedMemoryRetriever } from "./retrieval";

export interface CompressContextOptions {
	readonly taskQuery?: string;
	readonly targetBudget?: TierBudgetConfig | ContextTier | number;
	readonly role?: string;
	readonly maxLocalChars?: number;
	readonly maxGlobalChars?: number;
	readonly compactedNodeIds?: readonly string[];
	readonly summaryId?: string;
	readonly generalSummaryText?: string;
}

export interface CompressedContextResult {
	readonly summaryId: string;
	readonly readFiles: readonly string[];
	readonly modifiedFiles: readonly string[];
	readonly verificationRecords: readonly VerificationAuditRecord[];
	readonly localMemoryPages: readonly SectionPage[];
	readonly globalMemoryPages: readonly SectionPage[];
	readonly formattedContext: string;
	readonly totalChars: number;
	readonly withinBudget: boolean;
	readonly compactedSummary: CompactedSummary;
}

/**
 * Safely truncates an XML/tagged string to fit strictly within maxChars while
 * ensuring all opened XML tags are closed in reverse (LIFO) order.
 */
export function safeTruncateXml(content: string, maxChars: number, defaultTagName?: string): string {
	if (content.length <= maxChars) return content;
	if (maxChars <= 0) return "";

	const notice = "\n...[budget truncated]";
	if (maxChars < 30) {
		return content.slice(0, maxChars);
	}

	const tagRegex = /<(\/)?([a-zA-Z0-9_-]+)(?:\s+[^>]*)?(\/)?>/g;
	const safetyMargin = 50;
	const cutLimit = Math.max(1, maxChars - safetyMargin);
	let truncated = content.slice(0, cutLimit);

	// Strip trailing incomplete tag like "<some-tag"
	const lastOpenBracket = truncated.lastIndexOf("<");
	const lastCloseBracket = truncated.lastIndexOf(">");
	if (lastOpenBracket > lastCloseBracket) {
		truncated = truncated.slice(0, lastOpenBracket);
	}

	const lastNewline = truncated.lastIndexOf("\n");
	if (lastNewline > 20) {
		truncated = truncated.slice(0, lastNewline);
	}

	const getOpenStack = (str: string): string[] => {
		const stack: string[] = [];
		tagRegex.lastIndex = 0;
		let m: RegExpExecArray | null;
		while ((m = tagRegex.exec(str)) !== null) {
			const isClosing = m[1] === "/";
			const tagName = m[2];
			const isSelfClosing = m[3] === "/";
			if (isSelfClosing) continue;
			if (isClosing) {
				const idx = stack.lastIndexOf(tagName);
				if (idx !== -1) stack.splice(idx, 1);
			} else {
				stack.push(tagName);
			}
		}
		return stack;
	};

	let openStack = getOpenStack(truncated);
	if (defaultTagName && !openStack.includes(defaultTagName) && content.includes(`<${defaultTagName}`)) {
		openStack.unshift(defaultTagName);
	}

	let closingTags = openStack
		.slice()
		.reverse()
		.map(t => `</${t}>`)
		.join("\n");
	let suffix = closingTags ? `${notice}\n${closingTags}` : notice;

	// Loop to back off cleanly if truncated + suffix exceeds maxChars
	while (truncated.length + suffix.length > maxChars && truncated.length > 0) {
		const prevOpen = truncated.lastIndexOf("<");
		if (prevOpen > 0) {
			truncated = truncated.slice(0, prevOpen).trimEnd();
		} else {
			truncated = truncated.slice(0, Math.max(0, maxChars - suffix.length)).trimEnd();
			break;
		}

		openStack = getOpenStack(truncated);
		if (defaultTagName && !openStack.includes(defaultTagName) && content.includes(`<${defaultTagName}`)) {
			openStack.unshift(defaultTagName);
		}
		closingTags = openStack
			.slice()
			.reverse()
			.map(t => `</${t}>`)
			.join("\n");
		suffix = closingTags ? `${notice}\n${closingTags}` : notice;
	}

	return `${truncated.trimEnd()}${suffix}`;
}

export class HarvestContextCompactor {
	readonly #graph?: HarvestKnowledgeGraph;
	readonly #workspaceRoot: string;
	readonly #deterministicCompactor: DeterministicCompactor;

	constructor(workspaceRoot: string = process.cwd(), graph?: HarvestKnowledgeGraph) {
		this.#workspaceRoot = workspaceRoot;
		this.#graph = graph;
		this.#deterministicCompactor = new DeterministicCompactor(graph);
	}

	get graph(): HarvestKnowledgeGraph | undefined {
		return this.#graph;
	}

	/**
	 * Compress local and global memory with respect to prompt budgeting.
	 * Uses vectorless RAG with page index to rank and select relevant context.
	 */
	compressContext(
		memoryInput: LocalSessionMemory | LocalMemoryStore,
		options: CompressContextOptions = {},
	): CompressedContextResult {
		const mem: LocalSessionMemory =
			"snapshot" in memoryInput && typeof memoryInput.snapshot === "function"
				? memoryInput.snapshot()
				: (memoryInput as LocalSessionMemory);

		const summaryId = options.summaryId ?? `harvest-compaction-${Date.now()}`;
		const role = options.role ?? "backend";

		// Repoint graph edges if requested
		if (this.#graph && options.compactedNodeIds && options.compactedNodeIds.length > 0) {
			this.#graph.repointEdges(options.compactedNodeIds, summaryId);
		}

		// Resolve character budgets
		let maxAllowedChars = 4000;
		if (typeof options.targetBudget === "number") {
			maxAllowedChars = options.targetBudget;
		} else if (typeof options.targetBudget === "string") {
			const cfg = getTierBudget(options.targetBudget);
			maxAllowedChars = cfg.maxKnowledgeChars + cfg.maxInlineSkillsChars;
		} else if (options.targetBudget && typeof options.targetBudget === "object") {
			maxAllowedChars = options.targetBudget.maxKnowledgeChars + options.targetBudget.maxInlineSkillsChars;
		}

		// Account for outer wrapper: <harvest-context-compression id="...">\n...\n</harvest-context-compression>
		const wrapperOverhead = 60 + summaryId.length;
		const innerCapacity = Math.max(100, maxAllowedChars - wrapperOverhead);
		const localBudgetChars = options.maxLocalChars ?? Math.floor(innerCapacity * 0.55);
		const globalBudgetChars = options.maxGlobalChars ?? Math.floor(innerCapacity * 0.45);

		// Use vectorless RAG with page index
		const retriever = new UnifiedMemoryRetriever({
			workspaceRoot: this.#workspaceRoot,
			graph: this.#graph,
			localMemory: mem,
		});

		const query = options.taskQuery && options.taskQuery.trim().length > 0 ? options.taskQuery.trim() : "";

		const retrieval: UnifiedRetrievalResult = retriever.retrieve(query, {
			activeRole: role,
			limit: 8,
			maxTokens: Math.floor(innerCapacity / 4),
			includeGlobal: true,
			includeLocal: true,
			includeGraph: true,
			includeSkills: false,
		});

		const localPages = retrieval.localResults.map(r => r.page);
		const globalPages = retrieval.globalResults.map(r => r.page);

		// Build formatted local memory block with priority ordering:
		// 1. Summary
		// 2. Verification Records (ground truth for verification-before-done)
		// 3. Modified Files (active mutations)
		// 4. Active Working Memory (retrieved via RAG)
		// 5. Inspected Files (scoped/bounded)
		const localLines: string[] = ["<local-memory>"];
		if (options.generalSummaryText) {
			localLines.push(`## Summary`, options.generalSummaryText, "");
		}

		localLines.push(`## Verification Audit Records`);
		if (mem.verificationRecords.length > 0) {
			for (const v of mem.verificationRecords) {
				const badge = `[${v.status}]`;
				const reason = v.stalenessReason ? ` (${v.stalenessReason})` : "";
				localLines.push(`- ${badge} \`${v.command}\`${reason}`);
			}
		} else {
			localLines.push(`*(No verification records)*`);
		}
		localLines.push("");

		localLines.push(`## Modified Files`);
		if (mem.modifiedFiles.length > 0) {
			for (const f of mem.modifiedFiles) {
				localLines.push(`- ${f}`);
			}
		} else {
			localLines.push(`*(None)*`);
		}
		localLines.push("");

		// Working memory items retrieved via vectorless RAG
		const retrievedLocalNotes = localPages.filter(p => p.id.startsWith("local://wm/"));
		if (retrievedLocalNotes.length > 0) {
			localLines.push(`## Active Working Memory`);
			for (const p of retrievedLocalNotes) {
				localLines.push(`### ${p.heading}`);
				localLines.push(p.content);
				localLines.push("");
			}
		}

		localLines.push(`## Inspected Files`);
		if (mem.readFiles.length > 0) {
			// If many read files, prioritize files matching query tokens or show up to 15
			const queryTokens = new Set(tokenize(query));
			const sortedFiles = [...mem.readFiles];
			if (queryTokens.size > 0) {
				sortedFiles.sort((a, b) => {
					const aMatch = tokenize(a).filter(t => queryTokens.has(t)).length;
					const bMatch = tokenize(b).filter(t => queryTokens.has(t)).length;
					return bMatch - aMatch;
				});
			}

			const maxFilesToShow = 15;
			const displayFiles = sortedFiles.slice(0, maxFilesToShow);
			for (const f of displayFiles) {
				localLines.push(`- ${f}`);
			}
			if (sortedFiles.length > maxFilesToShow) {
				localLines.push(`*(+ ${sortedFiles.length - maxFilesToShow} more files)*`);
			}
		} else {
			localLines.push(`*(None)*`);
		}
		localLines.push("");

		localLines.push("</local-memory>");
		let localText = localLines.join("\n");
		if (localText.length > localBudgetChars) {
			localText = safeTruncateXml(localText, localBudgetChars, "local-memory");
		}

		// Build formatted global memory block
		const globalLines: string[] = ["<global-memory>"];
		if (globalPages.length > 0) {
			for (const p of globalPages) {
				globalLines.push(`### ${p.title}`);
				globalLines.push(p.content);
				globalLines.push("");
			}
		} else {
			globalLines.push(`*(No matching global patterns or graph entities)*`);
		}
		globalLines.push("</global-memory>");
		let globalText = globalLines.join("\n");
		if (globalText.length > globalBudgetChars) {
			globalText = safeTruncateXml(globalText, globalBudgetChars, "global-memory");
		}

		let formattedContext = [
			`<harvest-context-compression id="${summaryId}">`,
			localText,
			globalText,
			`</harvest-context-compression>`,
		].join("\n");

		// Final check to guarantee within budget
		if (formattedContext.length > maxAllowedChars) {
			formattedContext = safeTruncateXml(formattedContext, maxAllowedChars, "harvest-context-compression");
		}

		// Build deterministic compacted summary for compatibility
		const pseudoCalls: TranscriptToolCall[] = [
			...mem.readFiles.map(f => ({ name: "read", args: { path: f } })),
			...mem.modifiedFiles.map(f => ({ name: "write", args: { path: f } })),
			...mem.verificationRecords.map(v => ({
				name: "bash",
				args: { command: v.command },
				result: { exitCode: v.exitCode, success: v.status === "PASS" },
				timestamp: v.timestamp,
			})),
		];

		const compactedSummary = this.#deterministicCompactor.compact(pseudoCalls, {
			summaryId,
			compactedNodeIds: options.compactedNodeIds,
			generalSummaryText: options.generalSummaryText,
		});

		return {
			summaryId,
			readFiles: mem.readFiles,
			modifiedFiles: mem.modifiedFiles,
			verificationRecords: mem.verificationRecords,
			localMemoryPages: localPages,
			globalMemoryPages: globalPages,
			formattedContext,
			totalChars: formattedContext.length,
			withinBudget: formattedContext.length <= maxAllowedChars,
			compactedSummary,
		};
	}
}

export * from "../compaction/compaction";
export type { ExtractedTranscriptOperations };
