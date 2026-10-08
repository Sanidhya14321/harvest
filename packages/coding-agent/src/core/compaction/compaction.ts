/**
 * Deterministic Compaction with Verification Preservation.
 *
 * Preserves exact lists of read/modified files and test outcomes.
 * Repoints knowledge graph edges to the compacted summary node.
 */

import type { HarvestKnowledgeGraph } from "../harvest/graph";
import { extractTranscriptEvidence, type TranscriptToolCall, type VerificationAuditRecord } from "./utils";

export interface CompactedSummary {
	readonly summaryId: string;
	readonly readFiles: readonly string[];
	readonly modifiedFiles: readonly string[];
	readonly verificationRecords: readonly VerificationAuditRecord[];
	readonly formattedSummary: string;
}

export class DeterministicCompactor {
	readonly #graph?: HarvestKnowledgeGraph;

	constructor(graph?: HarvestKnowledgeGraph) {
		this.#graph = graph;
	}

	/**
	 * Perform deterministic compaction over a segment of transcript tool calls.
	 * Preserves read/modified files, test outcomes, and staleness evidence.
	 * Optionally repoints graph edges from compacted node IDs to summaryId.
	 */
	compact(
		toolCalls: readonly TranscriptToolCall[],
		options: {
			readonly summaryId?: string;
			readonly compactedNodeIds?: readonly string[];
			readonly generalSummaryText?: string;
		} = {},
	): CompactedSummary {
		const summaryId = options.summaryId ?? `compaction-${Date.now()}`;
		const evidence = extractTranscriptEvidence(toolCalls);

		const lines: string[] = [`<compacted-session-evidence id="${summaryId}">`];

		if (options.generalSummaryText) {
			lines.push(`## Summary`, options.generalSummaryText, ``);
		}

		lines.push(`## Files Inspected`);
		if (evidence.readFiles.length > 0) {
			for (const f of evidence.readFiles) {
				lines.push(`- ${f}`);
			}
		} else {
			lines.push(`*(None)*`);
		}
		lines.push(``);

		lines.push(`## Files Modified`);
		if (evidence.modifiedFiles.length > 0) {
			for (const f of evidence.modifiedFiles) {
				lines.push(`- ${f}`);
			}
		} else {
			lines.push(`*(None)*`);
		}
		lines.push(``);

		lines.push(`## Verification Records`);
		if (evidence.verificationRecords.length > 0) {
			for (const v of evidence.verificationRecords) {
				const badge = `[${v.status}]`;
				const reason = v.stalenessReason ? ` (${v.stalenessReason})` : "";
				lines.push(`- ${badge} \`${v.command}\`${reason}`);
			}
		} else {
			lines.push(`*(No verification commands executed)*`);
		}

		lines.push(`</compacted-session-evidence>`);

		// Repoint graph edges if graph and compactedNodeIds are provided
		if (this.#graph && options.compactedNodeIds && options.compactedNodeIds.length > 0) {
			this.#graph.repointEdges(options.compactedNodeIds, summaryId);
		}

		return {
			summaryId,
			readFiles: evidence.readFiles,
			modifiedFiles: evidence.modifiedFiles,
			verificationRecords: evidence.verificationRecords,
			formattedSummary: lines.join("\n"),
		};
	}
}

export * from "./utils";
