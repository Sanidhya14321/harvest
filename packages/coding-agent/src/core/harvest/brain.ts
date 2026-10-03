import type { AgentMessage } from "@harvest/pi-agent-core";
import { isEnoent, logger, parseFrontmatter, prompt } from "@harvest/pi-utils";
import * as fs from "node:fs/promises";
import * as path from "node:path";
import contextTemplate from "../../prompts/brain/context.md" with { type: "text" };
import relevanceInstructions from "../../prompts/brain/relevance.md" with { type: "text" };
import type { GraphEdge } from "./graph";
import { getLayaClient, type LayaClient, type LayaQuestionDefinition } from "./laya-client";
import {
	createScoringExcerpt,
	extractMessageText,
	RELEVANCE_CRITERIA,
	type LayaContextBudget,
} from "./laya-pruning";
import { KnowledgeRetriever, type SectionPage, splitMarkdownSections, tokenize } from "./retrieval";
import { SecuritySandbox } from "./security";

export interface BrainRoot {
	readonly directory: string;
	readonly scope: "project" | "user";
	readonly jailRoot?: string;
}

export interface BrainPage extends SectionPage {
	readonly scope: BrainRoot["scope"];
	readonly documentId: string;
}

interface IndexedDocument {
	readonly signature: string;
	readonly pages: BrainPage[];
	readonly links: GraphEdge[];
}

export interface BrainRetrievalOptions {
	readonly client?: LayaClient;
	readonly rerank?: boolean;
	/** Per-round-trip latency budget for reranking; falls back to lexical order on timeout. */
	readonly rerankTimeoutMs?: number;
	/**
	 * Shared prune+rerank latency budget from `sdk.ts` transformContext.
	 * When present, the rerank round trip is capped at the budget remainder
	 * (and skipped entirely when exhausted) so prune+rerank never exceed
	 * their joint allowance. Fail-open: exhaustion keeps lexical order.
	 */
	readonly contextBudget?: LayaContextBudget;
	/** Override the prompt revision embedded in the rerank cache key (tests only). */
	readonly promptRevision?: string;
	/** Knowledge scopes eligible for retrieval; omitted means project + user. */
	readonly scopes?: readonly BrainRoot["scope"][];
	readonly signal?: AbortSignal;
	readonly sessionId?: string;
	readonly maxChars?: number;
	readonly sanitize?: (text: string) => string;
}

const MAX_FILES = 512;
const MAX_FILE_BYTES = 256 * 1024;
const MAX_PAGES = 4096;
const MAX_CANDIDATES = 8;
const MAX_CONTEXT_CHARS = 8000;
/** Default rerank budget: one sidecar round trip on the shared decision timescale. */
export const DEFAULT_BRAIN_RERANK_TIMEOUT_MS = 300;
/**
 * Prompt revision binding rerank decisions to the exact instructions that
 * produced them. Embedded in the rerank cache key so a prompt-asset edit
 * never replays a ranking judged under different instructions.
 */
export const BRAIN_RERANK_PROMPT_REVISION: string = (() => {
	try {
		return Bun.hash(
			`${relevanceInstructions}\n${RELEVANCE_CRITERIA.join("\n")}\n${contextTemplate}`,
		).toString(36);
	} catch {
		return "brain-prompt-rev-unknown";
	}
})();
/** Bounded rerank-result cache: unchanged query/pages reuse ranking without another round trip. */
const MAX_RERANK_CACHE_ENTRIES = 32;

/** Markdown remains authoritative; the page/postings/edge index is disposable session state. */
export class MarkdownBrain {
	readonly #roots: readonly BrainRoot[];
	readonly #documents = new Map<string, IndexedDocument>();
	readonly #pages = new Map<string, BrainPage>();
	readonly #postings = new Map<string, Set<string>>();
	readonly #retriever = new KnowledgeRetriever();
	#edges: GraphEdge[] = [];
	#refreshing?: Promise<void>;
	#refreshedAt = 0;
	/** Rerank cache keyed by normalized query + candidate content signatures. */
	readonly #rerankCache = new Map<string, readonly string[]>();

	constructor(roots: readonly BrainRoot[]) {
		this.#roots = roots;
	}

	async refresh(): Promise<void> {
		if (this.#refreshing) return this.#refreshing;
		this.#refreshing = this.#refresh().finally(() => {
			this.#refreshing = undefined;
		});
		return this.#refreshing;
	}

	async #refresh(): Promise<void> {
		const seen = new Set<string>();
		let scanned = 0;
		let totalBytes = 0;
		for (const root of this.#roots) {
			const sandbox = new SecuritySandbox(root.jailRoot ?? root.directory);
			const queue = [root.directory];
			let directories = 0;
			while (queue.length && scanned < MAX_FILES && directories++ < MAX_FILES) {
				const directory = queue.shift()!;
				const checkedDirectory = sandbox.assertPathJailed(directory);
				if (!checkedDirectory.jailed) continue;
				try {
					const entries = await fs.readdir(checkedDirectory.resolvedPath, { withFileTypes: true });
					entries.sort((a, b) => a.name.localeCompare(b.name));
					for (const entry of entries) {
						if (scanned >= MAX_FILES) break;
						const filePath = path.join(directory, entry.name);
						// Do not follow symlinks/junctions out of an explicitly selected brain root.
						if (entry.isSymbolicLink()) continue;
						if (entry.isDirectory()) {
							if (queue.length < MAX_FILES) queue.push(filePath);
							continue;
						}
						if (!entry.isFile() || !entry.name.endsWith(".md")) continue;
						scanned++;
						const checked = sandbox.assertPathJailed(filePath);
						if (!checked.jailed) continue;
						const key = `${root.scope}:${filePath}`;
						try {
							const stat = await fs.stat(checked.resolvedPath);
							if (stat.size > MAX_FILE_BYTES) continue;
							if (totalBytes + stat.size > 8 * 1024 * 1024) continue;
							totalBytes += stat.size;
							const signature = `${stat.mtimeMs}:${stat.ctimeMs}:${stat.size}`;
							seen.add(key);
							if (this.#documents.get(key)?.signature === signature) continue;
							const file = Bun.file(checked.resolvedPath);
							const content = await file.text();
							if (content.length > MAX_FILE_BYTES) {
								seen.delete(key);
								continue;
							}
							const { frontmatter, body } = parseFrontmatter(content, { source: filePath, rawKeys: true });
							const relativePath = path.relative(root.directory, filePath).replaceAll("\\", "/");
							const rawId = typeof frontmatter.id === "string" ? frontmatter.id : relativePath;
							const documentId = `${root.scope}:${rawId}`;
							const title =
								typeof frontmatter.title === "string" ? frontmatter.title : path.basename(filePath, ".md");
							const pages =
								frontmatter.superseded === true
									? []
									: splitMarkdownSections(filePath, body, root.scope)
											.slice(0, 128)
											.map(page => ({
												...page,
												title,
												scope: root.scope,
												documentId,
												id: `${root.scope}:${page.id}`,
												tokens: tokenize(`${title} ${page.heading} ${page.content}`),
											}));
							const links: GraphEdge[] = [];
							for (const relation of ["depends_on", "relates_to", "supersedes", "contradicts"] as const) {
								const targets = frontmatter[relation];
								for (const target of Array.isArray(targets)
									? targets
									: typeof targets === "string"
										? [targets]
										: []) {
									if (typeof target !== "string") continue;
									links.push({
										source: documentId,
										target: /^(project|user):/.test(target) ? target : `${root.scope}:${target}`,
										relation,
									});
								}
							}
							this.#documents.set(key, { signature, pages, links });
						} catch (error) {
							seen.delete(key);
							logger.debug("Brain page could not be indexed", { filePath, error: String(error) });
						}
					}
				} catch (error) {
					if (!isEnoent(error))
						logger.debug("Brain root could not be scanned", { directory, error: String(error) });
				}
			}
		}
		for (const key of this.#documents.keys()) if (!seen.has(key)) this.#documents.delete(key);
		this.#pages.clear();
		this.#postings.clear();
		this.#edges = [];
		for (const document of this.#documents.values()) {
			this.#edges.push(...document.links);
			for (const page of document.pages) {
				if (this.#pages.size >= MAX_PAGES) break;
				this.#pages.set(page.id, page);
				for (const token of new Set(page.tokens)) {
					let posting = this.#postings.get(token);
					if (!posting) this.#postings.set(token, (posting = new Set()));
					posting.add(page.id);
				}
			}
		}
		this.#refreshedAt = Date.now();
	}

	/**
	 * Cache key binding a ranking to its exact inputs: prompt revision plus
	 * normalized query plus the candidate set's content signatures
	 * (order-independent, so retrieval order never affects hits). Edits,
	 * deletions, task changes, and prompt-asset changes all miss naturally;
	 * scope is enforced before reranking so excluded knowledge never enters
	 * the key.
	 */
	#rerankCacheKey(query: string, pages: readonly BrainPage[], promptRev?: string): string {
		const revision = promptRev ?? BRAIN_RERANK_PROMPT_REVISION;
		const normalizedQuery = query.trim().replace(/\s+/g, " ").toLowerCase();
		const signatures = pages
			.map(page => `${page.id}:${Bun.hash(`${page.title}\n${page.heading}\n${page.content}`).toString(36)}`)
			.sort();
		return `${revision}\n${normalizedQuery}\n${signatures.join("\n")}`;
	}

	#cachedRerankOrder(
		query: string,
		pages: readonly BrainPage[],
		promptRev?: string,
	): readonly string[] | undefined {
		return this.#rerankCache.get(this.#rerankCacheKey(query, pages, promptRev));
	}

	#storeRerankOrder(
		query: string,
		pages: readonly BrainPage[],
		order: readonly string[],
		promptRev?: string,
	): void {
		if (this.#rerankCache.size >= MAX_RERANK_CACHE_ENTRIES) {
			const oldest = this.#rerankCache.keys().next();
			if (!oldest.done) this.#rerankCache.delete(oldest.value);
		}
		this.#rerankCache.set(this.#rerankCacheKey(query, pages, promptRev), order);
	}

	/** Drop cached rankings (history rewrite); next rerank scores afresh. Fail-open. */
	clearRerankCache(): void {
		try {
			this.#rerankCache.clear();
		} catch (error) {
			logger.debug("Brain rerank cache clear failed open", { error: String(error) });
		}
	}

	/** Effective rerank timeout: shared-budget remainder wins over the explicit option. */
	#effectiveRerankTimeoutMs(options: BrainRetrievalOptions): number {
		const explicit = options.rerankTimeoutMs ?? DEFAULT_BRAIN_RERANK_TIMEOUT_MS;
		if (!options.contextBudget) return explicit;
		try {
			return Math.min(explicit, Math.max(0, options.contextBudget.remainingMs()));
		} catch {
			return explicit;
		}
	}

	async #rerankPages(query: string, pages: BrainPage[], options: BrainRetrievalOptions): Promise<void> {
		const questions: Record<string, LayaQuestionDefinition> = {};
		const state: Record<string, unknown> = {};
		pages.forEach((page, index) => {
			const key = `brain_${index}`;
			questions[key] = { type: "score", instructions: relevanceInstructions, criteria: RELEVANCE_CRITERIA };
			const sanitize = options.sanitize ?? (text => text);
			state[key] = {
				query: sanitize(query.slice(0, 2000)),
				scope: page.scope,
				title: sanitize(page.title),
				content: sanitize(createScoringExcerpt(page.content)),
			};
		});
		try {
			const effectiveTimeoutMs = this.#effectiveRerankTimeoutMs(options);
			// Shared-budget fail-open: no remainder means no round trip.
			if (options.contextBudget && effectiveTimeoutMs <= 0) {
				logger.debug("Brain reranking skipped: shared prune+rerank budget exhausted");
				return;
			}
			const result = await (options.client ?? getLayaClient()).decide(state, questions, {
				callSite: "brain_retrieval",
				signal: options.signal,
				sessionId: options.sessionId,
				timeoutMs: effectiveTimeoutMs,
			});
			const scores = pages.map((_page, index) => result.data?.[`brain_${index}`]?.score);
			// Incomplete/malformed decisions keep the deterministic BM25/graph ordering.
			if (
				!result.fallback &&
				scores.every(score => typeof score === "number" && Number.isFinite(score) && score >= 0 && score <= 3)
			) {
				const order = new Map(pages.map((page, index) => [page.id, scores[index]!]));
				pages.sort((a, b) => order.get(b.id)! - order.get(a.id)!);
				this.#storeRerankOrder(
					query,
					pages,
					pages.map(page => page.id),
					options.promptRevision,
				);
				logger.debug("Brain reranking rescored pages", { pages: pages.length, latencyMs: result.latencyMs });
			}
		} catch (error) {
			logger.debug("Brain reranking failed open", { error: String(error) });
		}
	}

	async retrieve(query: string, options: BrainRetrievalOptions = {}): Promise<BrainPage[]> {
		if (options.signal?.aborted || !query.trim()) return [];
		if (Date.now() - this.#refreshedAt > 2000) await this.refresh();
		if (options.signal?.aborted) return [];
		const ids = new Set<string>();
		for (const token of tokenize(query)) for (const id of this.#postings.get(token) ?? []) ids.add(id);
		const candidates = Array.from(ids, id => this.#pages.get(id)!).filter(Boolean);
		const ranked = this.#retriever.searchPages(candidates, query, MAX_CANDIDATES / 2, Number.MAX_SAFE_INTEGER);
		const unscoped = ranked.map(result => this.#pages.get(result.page.id)!);
		// Scope exclusion is user policy, not relevance: drop out-of-scope
		// pages before graph expansion and reranking so excluded knowledge
		// can never leak back in through neighbors or model scores.
		const pages = options.scopes ? unscoped.filter(page => options.scopes!.includes(page.scope)) : unscoped;
		// Expand only positive graph relationships, one hop, with bounded fan-out.
		const documents = new Set(pages.map(page => page.documentId));
		const neighbors = new Set(
			this.#edges
				.filter(
					edge => documents.has(edge.source) && (edge.relation === "depends_on" || edge.relation === "relates_to"),
				)
				.map(edge => edge.target),
		);
		for (const page of this.#pages.values()) {
			if (pages.length >= MAX_CANDIDATES) break;
			if (neighbors.has(page.documentId) && !pages.some(candidate => candidate.id === page.id)) pages.push(page);
		}
		if (options.rerank && pages.length > 1 && !options.signal?.aborted) {
			const promptRev = options.promptRevision ?? BRAIN_RERANK_PROMPT_REVISION;
			const cachedOrder = this.#cachedRerankOrder(query, pages, promptRev);
			if (cachedOrder) {
				logger.debug("Brain reranking served from cache", { pages: pages.length });
				const rank = new Map(cachedOrder.map((id, index) => [id, index]));
				pages.sort((a, b) => rank.get(a.id)! - rank.get(b.id)!);
			} else {
				await this.#rerankPages(query, pages, options);
			}
		}
		if (options.signal?.aborted) return [];
		let remaining = Math.max(0, Math.min(MAX_CONTEXT_CHARS, options.maxChars ?? MAX_CONTEXT_CHARS));
		const bounded: BrainPage[] = [];
		for (const page of pages) {
			const overhead = prompt.render(contextTemplate, { pages: [{ ...page, content: "" }] }).length;
			if (remaining <= overhead) break;
			const content = page.content.slice(0, remaining - overhead);
			bounded.push({ ...page, content });
			remaining -= overhead + content.length;
		}
		return bounded;
	}

	/** Ephemeral transform: does not persist retrieved data or replace protocol blocks. */
	async transform(messages: AgentMessage[], options: BrainRetrievalOptions = {}): Promise<AgentMessage[]> {
		let index = messages.length - 1;
		while (index >= 0 && messages[index].role !== "user") index--;
		if (index < 0) return messages;
		const user = messages[index];
		if (user.role !== "user") return messages;
		const pages = await this.retrieve(extractMessageText(user), options);
		if (!pages.length) return messages;
		const budget = Math.max(0, Math.min(MAX_CONTEXT_CHARS, options.maxChars ?? MAX_CONTEXT_CHARS));
		const context = prompt.render(contextTemplate, { pages }).slice(0, budget);
		const content = typeof user.content === "string" ? [{ type: "text" as const, text: user.content }] : user.content;
		const transformed = messages.slice();
		transformed[index] = { ...user, content: [...content, { type: "text", text: context }] };
		return transformed;
	}
}

export function createMarkdownBrain(workspaceRoot: string, agentDir: string, includeSkills = true): MarkdownBrain {
	return new MarkdownBrain([
		{ scope: "project", jailRoot: workspaceRoot, directory: path.join(workspaceRoot, ".harvest", "brain") },
		{ scope: "user", jailRoot: agentDir, directory: path.join(agentDir, "brain") },
		...(includeSkills
			? [
					{
						scope: "project" as const,
						jailRoot: workspaceRoot,
						directory: path.join(workspaceRoot, ".harvest", "skills"),
					},
					{
						scope: "project" as const,
						jailRoot: workspaceRoot,
						directory: path.join(workspaceRoot, ".agents", "skills"),
					},
					{ scope: "user" as const, jailRoot: agentDir, directory: path.join(agentDir, "skills") },
				]
			: []),
	]);
}
