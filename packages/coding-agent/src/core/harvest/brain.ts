import type { AgentMessage } from "@harvest/pi-agent-core";
import { NativeMarkdownBrain } from "@harvest/pi-natives";
import { logger, prompt } from "@harvest/pi-utils";
import * as path from "node:path";
import contextTemplate from "../../prompts/brain/context.md" with { type: "text" };
import type { SectionPage } from "./retrieval";
import { extractMessageText } from "./text-extract";

export interface BrainRoot {
	readonly directory: string;
	readonly scope: "project" | "user";
	readonly jailRoot?: string;
}

export interface BrainPage extends SectionPage {
	readonly scope: BrainRoot["scope"];
	readonly documentId: string;
}

export interface BrainRetrievalOptions {
	/**
	 * Removed Laya model reranking now resolves as lexical/graph order.
	 * Kept so older callers keep compiling; ignored at runtime.
	 * @deprecated Reranking was removed with Laya; lexical/graph order is authoritative.
	 */
	readonly rerank?: boolean;
	/** @deprecated Ignored since Laya removal. */
	readonly rerankTimeoutMs?: number;
	/** Knowledge scopes eligible for retrieval; omitted means project + user. */
	readonly scopes?: readonly BrainRoot["scope"][];
	/** Applicability conditions selected by the host. */
	readonly tags?: readonly string[];
	readonly signal?: AbortSignal;
	readonly sessionId?: string;
	readonly maxChars?: number;
	readonly sanitize?: (text: string) => string;
}

const MAX_CONTEXT_CHARS = 8000;

/** Compatibility adapter; indexing, ranking and graph policy are owned by Rust. */
export class MarkdownBrain {
	readonly #native: NativeMarkdownBrain;

	constructor(roots: readonly BrainRoot[]) {
		this.#native = new NativeMarkdownBrain(JSON.stringify(roots));
	}

	async refresh(): Promise<void> {
		await this.#native.refresh();
	}

	/** @deprecated Model reranking was removed. */
	clearRerankCache(): void {}

	async retrieve(query: string, options: BrainRetrievalOptions = {}): Promise<BrainPage[]> {
		if (options.signal?.aborted || !query.trim() || options.scopes?.length === 0) return [];
		try {
			const result = await this.#native.retrieve(
				query,
				JSON.stringify({
					scopes: options.scopes,
					tags: options.tags,
					maxChars: options.maxChars,
				}),
			);
			if (options.signal?.aborted) return [];
			const pages: BrainPage[] = JSON.parse(result);
			return pages;
		} catch (error) {
			logger.warn("Rust brain retrieval unavailable", { error: String(error) });
			return [];
		}
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
