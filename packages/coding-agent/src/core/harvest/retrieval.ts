/**
 * Vectorless Dual-Stage Section Retrieval with Okapi BM25.
 *
 * Stage 1: Role scoping (.harvest/<role>/ and .harvest/shared/).
 * Stage 2: Section-level markdown chunking (preserving code blocks)
 *          ranked with Okapi BM25 (k1 = 1.5, b = 0.75) and Robertson IDF.
 */

import * as fs from "node:fs";
import * as path from "node:path";
import type { HarvestKnowledgeGraph } from "./graph";
import type { LocalMemoryStore, LocalSessionMemory } from "./memory";
import type { LoadedSkill } from "./skill-context";

export interface SectionPage {
	readonly id: string;
	readonly filePath: string;
	readonly role: string;
	readonly title: string;
	readonly heading: string;
	readonly content: string;
	readonly tokens: readonly string[];
}

export interface SearchResult {
	readonly page: SectionPage;
	readonly score: number;
}

const K1 = 1.5;
const B = 0.75;

/** Tokenize text into lowercase alphanumeric keywords */
export function tokenize(text: string): string[] {
	return text
		.toLowerCase()
		.replace(/[^a-z0-9_\-\s]/g, " ")
		.split(/\s+/)
		.filter(t => t.length >= 2);
}

/**
 * Split markdown content into discrete section pages by headings (##, ###).
 * Preserves code blocks with internal '#' characters without creating false section breaks.
 */
export function splitMarkdownSections(filePath: string, content: string, role: string): SectionPage[] {
	const lines = content.split("\n");
	const sections: SectionPage[] = [];

	let currentHeading = path.basename(filePath, ".md");
	let currentTitle = currentHeading;
	let currentLines: string[] = [];
	let inCodeBlock = false;
	let sectionIndex = 0;

	// Extract title from YAML frontmatter if present
	if (content.startsWith("---")) {
		const endFm = content.indexOf("\n---", 3);
		if (endFm !== -1) {
			const fm = content.slice(3, endFm);
			const titleMatch = fm.match(/^title:\s*(.*)$/m);
			if (titleMatch) {
				currentTitle = titleMatch[1].trim();
				currentHeading = currentTitle;
			}
		}
	}

	for (let i = 0; i < lines.length; i++) {
		const line = lines[i];

		// Track code fence toggle (``` or ~~~)
		if (/^```|^~~~/.test(line.trim())) {
			inCodeBlock = !inCodeBlock;
			currentLines.push(line);
			continue;
		}

		if (!inCodeBlock) {
			const headingMatch = line.match(/^(#{2,3})\s+(.*)$/);
			if (headingMatch) {
				// Flush previous section
				if (currentLines.length > 0) {
					const text = currentLines.join("\n").trim();
					if (text.length > 0) {
						const tokens = tokenize(text);
						sections.push({
							id: `${filePath}#section-${sectionIndex++}`,
							filePath,
							role,
							title: currentTitle,
							heading: currentHeading,
							content: text,
							tokens,
						});
					}
					currentLines = [];
				}
				currentHeading = headingMatch[2].trim();
				currentLines.push(line);
				continue;
			}
		}

		currentLines.push(line);
	}

	if (currentLines.length > 0) {
		const text = currentLines.join("\n").trim();
		if (text.length > 0) {
			const tokens = tokenize(text);
			sections.push({
				id: `${filePath}#section-${sectionIndex++}`,
				filePath,
				role,
				title: currentTitle,
				heading: currentHeading,
				content: text,
				tokens,
			});
		}
	}

	return sections;
}

export class KnowledgeRetriever {
	readonly #harvestDir: string;

	constructor(workspaceRoot: string = process.cwd()) {
		this.#harvestDir = path.join(path.resolve(workspaceRoot), ".harvest");
	}

	/**
	 * Load section pages scoped to the active role and shared directory.
	 */
	loadScopedSections(activeRole: string): SectionPage[] {
		const sections: SectionPage[] = [];
		if (!fs.existsSync(this.#harvestDir)) {
			return sections;
		}

		const roleDirs = [activeRole, "shared", "patterns", "anti-patterns"];

		for (const sub of roleDirs) {
			const dirPath = path.join(this.#harvestDir, sub);
			if (!fs.existsSync(dirPath) || !fs.statSync(dirPath).isDirectory()) {
				continue;
			}

			const files = fs.readdirSync(dirPath).filter(f => f.endsWith(".md"));
			for (const file of files) {
				const fullPath = path.join(dirPath, file);
				try {
					const content = fs.readFileSync(fullPath, "utf8");
					const fileSections = splitMarkdownSections(fullPath, content, sub);
					sections.push(...fileSections);
				} catch {}
			}
		}

		return sections;
	}

	/**
	 * Search an arbitrary array of section pages using Okapi BM25.
	 * Enforces working memory deduplication and cache token bounding.
	 */
	searchPages(pages: SectionPage[], query: string, limit: number = 5, maxTokens: number = 4000): SearchResult[] {
		if (pages.length === 0) return [];

		// Memory deduplication: filter out pages with identical trimmed content
		const uniquePages = new Map<string, SectionPage>();
		for (const p of pages) {
			const key = p.content.trim();
			if (!uniquePages.has(key)) {
				uniquePages.set(key, p);
			}
		}
		const dedupedPages = Array.from(uniquePages.values());

		const queryTokens = tokenize(query);
		if (queryTokens.length === 0) {
			// Fallback: return top pages bounded by limit and maxTokens
			const fallbackResults: SearchResult[] = [];
			let currentTokens = 0;
			for (const page of dedupedPages.slice(0, limit)) {
				if (currentTokens + page.tokens.length > maxTokens && fallbackResults.length > 0) break;
				fallbackResults.push({ page, score: 0 });
				currentTokens += page.tokens.length;
			}
			return fallbackResults;
		}

		const N = dedupedPages.length;
		const totalLength = dedupedPages.reduce((sum, p) => sum + p.tokens.length, 0);
		const avgdl = totalLength / N || 1;

		// Fast document frequency for query tokens using token sets
		const pageTokenSets = dedupedPages.map(p => new Set(p.tokens));
		const df: Record<string, number> = {};
		for (const q of queryTokens) {
			let count = 0;
			for (const set of pageTokenSets) {
				if (set.has(q)) count++;
			}
			df[q] = count;
		}

		// Compute non-negative Robertson IDF
		const idf: Record<string, number> = {};
		for (const q of queryTokens) {
			const nq = df[q] || 0;
			// Robertson IDF with non-negative lower bound
			idf[q] = Math.max(0, Math.log((N - nq + 0.5) / (nq + 0.5) + 1));
		}

		const scoredResults: SearchResult[] = [];

		for (const page of dedupedPages) {
			let score = 0;
			const termFreq: Record<string, number> = {};
			for (const t of page.tokens) {
				termFreq[t] = (termFreq[t] || 0) + 1;
			}

			const docLen = page.tokens.length;

			for (const q of queryTokens) {
				const f = termFreq[q] || 0;
				if (f > 0) {
					const numerator = f * (K1 + 1);
					const denominator = f + K1 * (1 - B + B * (docLen / avgdl));
					score += idf[q] * (numerator / denominator);
				}
			}

			if (score > 0) {
				scoredResults.push({ page, score });
			}
		}

		scoredResults.sort((a, b) => b.score - a.score);

		// Cache bounding & context token preservation
		const finalResults: SearchResult[] = [];
		let currentTokens = 0;
		for (const res of scoredResults) {
			if (finalResults.length >= limit) break;
			if (currentTokens + res.page.tokens.length > maxTokens) {
				if (finalResults.length === 0) {
					finalResults.push(res);
					break;
				}
				continue;
			}
			finalResults.push(res);
			currentTokens += res.page.tokens.length;
		}

		return finalResults;
	}

	/**
	 * Search scoped sections using Okapi BM25.
	 */
	search(query: string, activeRole: string = "backend", limit: number = 5): SearchResult[] {
		const pages = this.loadScopedSections(activeRole);
		return this.searchPages(pages, query, limit);
	}
}

/**
 * Index typed knowledge graph nodes and directed relationships into searchable SectionPages.
 */
export function indexGraphToPages(graph: HarvestKnowledgeGraph): SectionPage[] {
	const pages: SectionPage[] = [];
	const nodes = graph.nodes;

	for (const [nodeId, node] of Object.entries(nodes)) {
		const outgoing = graph.getOutgoingEdges(nodeId);
		const incoming = graph.getIncomingEdges(nodeId);

		const relLines: string[] = [];
		if (outgoing.length > 0) {
			relLines.push("Outgoing relations:");
			for (const e of outgoing) {
				const targetNode = nodes[e.target];
				const targetTitle = targetNode ? ` ("${targetNode.title}")` : "";
				relLines.push(`- ${e.relation} -> ${e.target}${targetTitle}`);
			}
		}
		if (incoming.length > 0) {
			relLines.push("Incoming relations:");
			for (const e of incoming) {
				const sourceNode = nodes[e.source];
				const sourceTitle = sourceNode ? ` ("${sourceNode.title}")` : "";
				relLines.push(`- ${e.relation} <- ${e.source}${sourceTitle}`);
			}
		}

		const metadataText = node.metadata ? JSON.stringify(node.metadata) : "";
		const contentLines = [
			`# Node: ${node.title}`,
			`- ID: ${node.id}`,
			`- Role: ${node.role}`,
			`- Kind: ${node.kind}`,
			`- Path: ${node.path}`,
			...(relLines.length > 0 ? ["", ...relLines] : []),
			...(metadataText ? ["", `Metadata: ${metadataText}`] : []),
		];
		const content = contentLines.join("\n").trim();
		const tokens = tokenize(
			`${node.title} ${node.id} ${node.role} ${node.kind} ${relLines.join(" ")} ${metadataText}`,
		);

		pages.push({
			id: `graph://node/${node.id}`,
			filePath: node.path || ".harvest/graph.json",
			role: node.role,
			title: `[Knowledge Graph] ${node.title}`,
			heading: node.title,
			content,
			tokens,
		});
	}

	return pages;
}

/**
 * Index local session working memory (hypotheses, files read/modified, verifications) into SectionPages.
 */
export function indexLocalMemoryToPages(memoryInput: LocalSessionMemory | LocalMemoryStore): SectionPage[] {
	const mem =
		"snapshot" in memoryInput && typeof memoryInput.snapshot === "function"
			? memoryInput.snapshot()
			: (memoryInput as LocalSessionMemory);

	const pages: SectionPage[] = [];

	// 1. Working memory items (hypotheses, goals, decisions, findings, notes)
	for (const item of mem.workingMemoryItems) {
		const tagsText = item.tags && item.tags.length > 0 ? `Tags: ${item.tags.join(", ")}` : "";
		const content = [
			`# Working Memory: ${item.title}`,
			`Category: ${item.category}`,
			...(tagsText ? [tagsText] : []),
			"",
			item.content,
		]
			.join("\n")
			.trim();

		pages.push({
			id: `local://wm/${item.id}`,
			filePath: "session://working-memory",
			role: "local",
			title: `[Local ${item.category}] ${item.title}`,
			heading: item.title,
			content,
			tokens: tokenize(`${item.title} ${item.category} ${item.content} ${item.tags?.join(" ") ?? ""}`),
		});
	}

	// 2. Read files
	if (mem.readFiles.length > 0) {
		const content = [
			`# Session Inspected Files`,
			`The following files were inspected in the active session:`,
			...mem.readFiles.map(f => `- ${f}`),
		].join("\n");

		pages.push({
			id: "local://files/read",
			filePath: "session://read-files",
			role: "local",
			title: "[Session Read Files]",
			heading: "Inspected Files",
			content,
			tokens: tokenize(mem.readFiles.join(" ")),
		});
	}

	// 3. Modified files
	if (mem.modifiedFiles.length > 0) {
		const content = [
			`# Session Modified Files`,
			`The following files were modified in the active session:`,
			...mem.modifiedFiles.map(f => `- ${f}`),
		].join("\n");

		pages.push({
			id: "local://files/modified",
			filePath: "session://modified-files",
			role: "local",
			title: "[Session Modified Files]",
			heading: "Modified Files",
			content,
			tokens: tokenize(mem.modifiedFiles.join(" ")),
		});

		// Fine-grained modified file pages
		for (const f of mem.modifiedFiles) {
			pages.push({
				id: `local://file/modified/${f}`,
				filePath: f,
				role: "local",
				title: `[Modified File] ${path.basename(f)}`,
				heading: path.basename(f),
				content: `Modified file: ${f}`,
				tokens: tokenize(f),
			});
		}
	}

	// 4. Verification audit records
	if (mem.verificationRecords.length > 0) {
		const content = [
			`# Session Verification Records`,
			...mem.verificationRecords.map(v => {
				const badge = `[${v.status}]`;
				const reason = v.stalenessReason ? ` (${v.stalenessReason})` : "";
				return `- ${badge} \`${v.command}\` (exit: ${v.exitCode})${reason}`;
			}),
		].join("\n");

		pages.push({
			id: "local://verifications/audit",
			filePath: "session://verifications",
			role: "local",
			title: "[Session Verification Audit]",
			heading: "Verification Audit Trail",
			content,
			tokens: tokenize(content),
		});

		// Fine-grained verification pages
		for (const v of mem.verificationRecords) {
			pages.push({
				id: `local://verification/${encodeURIComponent(v.command)}`,
				filePath: "session://verifications",
				role: "local",
				title: `[Verification] ${v.command}`,
				heading: v.command,
				content: `Verification: \`${v.command}\` [${v.status}] (exit: ${v.exitCode})${v.stalenessReason ? ` (${v.stalenessReason})` : ""}`,
				tokens: tokenize(`${v.command} ${v.status} ${v.stalenessReason ?? ""}`),
			});
		}
	}

	// 5. Touched symbols
	if (mem.touchedSymbols && mem.touchedSymbols.length > 0) {
		const content = [`# Session Touched Symbols`, ...mem.touchedSymbols.map(s => `- ${s}`)].join("\n");

		pages.push({
			id: "local://symbols/touched",
			filePath: "session://symbols",
			role: "local",
			title: "[Session Touched Symbols]",
			heading: "Touched Symbols",
			content,
			tokens: tokenize(mem.touchedSymbols.join(" ")),
		});
	}

	// 6. Active skills
	if (mem.activeSkills && mem.activeSkills.length > 0) {
		const content = [`# Session Active Skills`, ...mem.activeSkills.map(s => `- ${s}`)].join("\n");

		pages.push({
			id: "local://skills/active",
			filePath: "session://active-skills",
			role: "local",
			title: "[Session Active Skills]",
			heading: "Active Skills",
			content,
			tokens: tokenize(mem.activeSkills.join(" ")),
		});
	}

	return pages;
}

/**
 * Index approved skills into SectionPages.
 */
export function indexSkillsToPages(skills: readonly LoadedSkill[]): SectionPage[] {
	const pages: SectionPage[] = [];
	for (const skill of skills) {
		const content = [
			`# Skill: ${skill.name}`,
			skill.description ? `Description: ${skill.description}` : "",
			"",
			skill.content,
		]
			.filter(Boolean)
			.join("\n");

		pages.push({
			id: `skill://${skill.name}`,
			filePath: skill.filePath,
			role: "shared",
			title: `[Skill] ${skill.name}`,
			heading: skill.name,
			content,
			tokens: tokenize(`${skill.name} ${skill.description} ${skill.content}`),
		});
	}
	return pages;
}

export interface UnifiedRetrievalOptions {
	readonly activeRole?: string;
	readonly limit?: number;
	readonly maxTokens?: number;
	readonly includeGlobal?: boolean;
	readonly includeLocal?: boolean;
	readonly includeGraph?: boolean;
	readonly includeSkills?: boolean;
}

export interface UnifiedRetrievalResult {
	readonly allResults: readonly SearchResult[];
	readonly localResults: readonly SearchResult[];
	readonly globalResults: readonly SearchResult[];
	readonly retrievedPages: readonly SectionPage[];
	readonly formattedContext: string;
}

/**
 * Unified Vectorless Memory Retriever with Page Indexing.
 *
 * Simultaneously indexes and searches across Global Memory (.harvest/ patterns,
 * anti-patterns, knowledge graph, skills) and Local Memory (working memory notes,
 * inspected/modified files, verification records) using Okapi BM25 & Robertson IDF.
 */
export class UnifiedMemoryRetriever {
	readonly #workspaceRoot: string;
	readonly #knowledgeRetriever: KnowledgeRetriever;
	readonly #graph?: HarvestKnowledgeGraph;
	readonly #localMemory?: LocalSessionMemory | LocalMemoryStore;
	readonly #skills?: readonly LoadedSkill[];

	constructor(
		options: {
			readonly workspaceRoot?: string;
			readonly graph?: HarvestKnowledgeGraph;
			readonly localMemory?: LocalSessionMemory | LocalMemoryStore;
			readonly skills?: readonly LoadedSkill[];
		} = {},
	) {
		this.#workspaceRoot = options.workspaceRoot ?? process.cwd();
		this.#knowledgeRetriever = new KnowledgeRetriever(this.#workspaceRoot);
		this.#graph = options.graph;
		this.#localMemory = options.localMemory;
		this.#skills = options.skills;
	}

	get knowledgeRetriever(): KnowledgeRetriever {
		return this.#knowledgeRetriever;
	}

	retrieve(query: string, options: UnifiedRetrievalOptions = {}): UnifiedRetrievalResult {
		const activeRole = options.activeRole ?? "backend";
		const limit = options.limit ?? 5;
		const maxTokens = options.maxTokens ?? 4000;
		const includeGlobal = options.includeGlobal !== false;
		const includeLocal = options.includeLocal !== false;
		const includeGraph = options.includeGraph !== false;
		const includeSkills = options.includeSkills !== false;

		const allPages: SectionPage[] = [];
		const globalPages: SectionPage[] = [];
		const localPages: SectionPage[] = [];

		// 1. Global markdown sections (.harvest/<role>, shared, patterns, anti-patterns)
		if (includeGlobal) {
			const scopedGlobal = this.#knowledgeRetriever.loadScopedSections(activeRole);
			globalPages.push(...scopedGlobal);
			allPages.push(...scopedGlobal);
		}

		// 2. Global Knowledge Graph
		if (includeGraph && this.#graph) {
			const graphPages = indexGraphToPages(this.#graph);
			globalPages.push(...graphPages);
			allPages.push(...graphPages);
		}

		// 3. Skills
		if (includeSkills && this.#skills && this.#skills.length > 0) {
			const skillPages = indexSkillsToPages(this.#skills);
			globalPages.push(...skillPages);
			allPages.push(...skillPages);
		}

		// 4. Local Session Memory
		if (includeLocal && this.#localMemory) {
			const localMemoryPages = indexLocalMemoryToPages(this.#localMemory);
			localPages.push(...localMemoryPages);
			allPages.push(...localMemoryPages);
		}

		// Search across all pages with Okapi BM25 & Robertson IDF
		const searchResults = this.#knowledgeRetriever.searchPages(allPages, query, limit, maxTokens);

		// Segregate local vs global results
		const localSet = new Set(localPages.map(p => p.id));
		const localResults: SearchResult[] = [];
		const globalResults: SearchResult[] = [];

		for (const res of searchResults) {
			if (localSet.has(res.page.id)) {
				localResults.push(res);
			} else {
				globalResults.push(res);
			}
		}

		const retrievedPages = searchResults.map(r => r.page);
		const formattedContext = this.#formatRetrievedContext(localResults, globalResults);

		return {
			allResults: searchResults,
			localResults,
			globalResults,
			retrievedPages,
			formattedContext,
		};
	}

	#formatRetrievedContext(localResults: readonly SearchResult[], globalResults: readonly SearchResult[]): string {
		if (localResults.length === 0 && globalResults.length === 0) {
			return "";
		}

		const lines: string[] = ["<harvest-retrieved-context>"];

		if (localResults.length > 0) {
			lines.push("<local-memory>");
			for (const r of localResults) {
				lines.push(`### ${r.page.title}`);
				lines.push(r.page.content);
				lines.push("");
			}
			lines.push("</local-memory>");
		}

		if (globalResults.length > 0) {
			lines.push("<global-memory>");
			for (const r of globalResults) {
				lines.push(`### ${r.page.title}`);
				lines.push(r.page.content);
				lines.push("");
			}
			lines.push("</global-memory>");
		}

		lines.push("</harvest-retrieved-context>");
		return lines.join("\n");
	}
}
