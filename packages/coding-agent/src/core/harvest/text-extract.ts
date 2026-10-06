/**
 * Plain-text extraction helpers for message scoring and retrieval.
 *
 * Relocated from the removed Laya pruning module: MarkdownBrain still needs
 * text extraction and excerpt sizing for lexical/graph retrieval. No model
 * scoring, pruning, or gating lives here.
 */
import type { AgentMessage } from "@harvest/pi-agent-core";

export const MAX_SCORING_CHUNK_TOKENS = 800;

/** Fast estimation of token count from string content (~4 chars per token). */
export function estimateTextTokens(text: string): number {
	if (!text) return 0;
	return Math.ceil(text.length / 4);
}

/** Extract clean string representation of an AgentMessage's content. */
export function extractMessageText(message: AgentMessage): string {
	const content = (message as { content?: unknown }).content;
	if (typeof content === "string") {
		return content;
	}
	if (Array.isArray(content)) {
		return (content as Array<{ text?: string; thinking?: string }>)
			.map((part: { text?: string; thinking?: string }) => {
				if ("text" in part && typeof part.text === "string") return part.text;
				if ("thinking" in part && typeof part.thinking === "string") return part.thinking;
				return "";
			})
			.join("\n");
	}
	return "";
}

/**
 * Truncate a chunk to a representative scoring window (first ~400 + last
 * ~400 tokens) for retrieval sizing.
 */
export function createScoringExcerpt(text: string, maxTokens: number = MAX_SCORING_CHUNK_TOKENS): string {
	const estimated = estimateTextTokens(text);
	if (estimated <= maxTokens) {
		return text;
	}

	const halfChars = Math.floor((maxTokens / 2) * 4);
	const first = text.slice(0, halfChars);
	const last = text.slice(-halfChars);
	const omittedTokens = Math.max(0, estimated - maxTokens);

	return `${first}\n...[${omittedTokens} tokens omitted for relevance scoring]...\n${last}`;
}
