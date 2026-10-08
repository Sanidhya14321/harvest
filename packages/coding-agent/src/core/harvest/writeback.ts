/**
 * Post-Task Writeback Engine.
 *
 * Automatically records successful Patterns and corrected Anti-Patterns to .harvest/.
 * Resolves duplicates and superseding rules via token overlap coefficient.
 * Updates graph.json and invokes contradiction detection.
 */

import * as fs from "node:fs";
import * as path from "node:path";
import { HarvestKnowledgeGraph } from "./graph";
import { ContradictionEngine, extractKeywords, type KnowledgeEntry } from "./memory";

export interface PatternPayload {
	readonly role: string;
	readonly title: string;
	readonly taskId: string;
	readonly description: string;
	readonly codeSnippet?: string;
	readonly groundingValidated?: boolean;
}

export interface AntiPatternPayload {
	readonly role: string;
	readonly title: string;
	readonly taskId: string;
	readonly attemptedAction: string;
	readonly whyItFailed: string;
	readonly correctiveRule: string;
	readonly groundingValidated?: boolean;
}

export interface WritebackResult {
	readonly action: "created" | "refreshed" | "superseded";
	readonly filePath: string;
	readonly entryId: string;
	readonly supersededPath?: string;
}

/** Compute token overlap similarity (Jaccard similarity) between two titles */
export function titleOverlapCoefficient(titleA: string, titleB: string): number {
	const tokensA = new Set(extractKeywords(titleA));
	const tokensB = new Set(extractKeywords(titleB));

	if (tokensA.size === 0 || tokensB.size === 0) return 0;

	let intersection = 0;
	for (const t of tokensA) {
		if (tokensB.has(t)) intersection++;
	}

	const union = new Set([...tokensA, ...tokensB]).size;
	return union === 0 ? 0 : intersection / union;
}

function slugify(title: string): string {
	return title
		.toLowerCase()
		.replace(/[^a-z0-9]+/g, "-")
		.replace(/^-|-$/g, "")
		.slice(0, 50);
}

export class HarvestWriteback {
	readonly #workspaceRoot: string;
	readonly #harvestDir: string;
	readonly #graph: HarvestKnowledgeGraph;
	readonly #contradictionEngine: ContradictionEngine;

	constructor(workspaceRoot: string = process.cwd()) {
		this.#workspaceRoot = path.resolve(workspaceRoot);
		this.#harvestDir = path.join(this.#workspaceRoot, ".harvest");
		this.#graph = new HarvestKnowledgeGraph(this.#workspaceRoot);
		this.#contradictionEngine = new ContradictionEngine(this.#workspaceRoot, this.#graph);
	}

	/**
	 * Find most similar existing entry in the same role and kind.
	 */
	#findSimilarEntry(
		role: string,
		kind: "pattern" | "anti-pattern",
		title: string,
	): { entry: KnowledgeEntry; score: number } | null {
		const entries = this.#contradictionEngine
			.loadAllEntries()
			.filter(e => e.role === role && e.kind === kind && !e.superseded);

		let bestEntry: KnowledgeEntry | null = null;
		let bestScore = 0;

		for (const entry of entries) {
			const score = titleOverlapCoefficient(title, entry.title);
			if (score > bestScore) {
				bestScore = score;
				bestEntry = entry;
			}
		}

		return bestEntry && bestScore >= 0.5 ? { entry: bestEntry, score: bestScore } : null;
	}

	/**
	 * Write a verified successful pattern.
	 */
	async writePattern(payload: PatternPayload): Promise<WritebackResult> {
		const role = payload.role || "backend";
		const kind = "pattern";
		const targetDir = path.join(this.#harvestDir, role, "patterns");
		await fs.promises.mkdir(targetDir, { recursive: true });

		const match = this.#findSimilarEntry(role, kind, payload.title);

		if (match && match.score >= 0.85) {
			// Refresh existing file timestamp
			const now = new Date();
			await fs.promises.utimes(match.entry.filePath, now, now);
			return {
				action: "refreshed",
				filePath: match.entry.filePath,
				entryId: match.entry.id,
			};
		}

		let supersedesRelPath: string | undefined;
		let supersededPath: string | undefined;

		if (match && match.score >= 0.5) {
			// Supersede existing entry: mark old entry superseded
			const oldContent = await fs.promises.readFile(match.entry.filePath, "utf8");
			const updatedOld = oldContent.replace(/^superseded:\s*false/m, "superseded: true");
			const withFlag = updatedOld.includes("superseded:")
				? updatedOld
				: updatedOld.replace(/^---\n/, "---\nsuperseded: true\n");

			await fs.promises.writeFile(match.entry.filePath, withFlag, "utf8");
			supersedesRelPath = path.relative(this.#harvestDir, match.entry.filePath).replace(/\\/g, "/");
			supersededPath = match.entry.filePath;
		}

		const fileName = `${slugify(payload.title)}.md`;
		const filePath = path.join(targetDir, fileName);

		const frontmatter = [
			`---`,
			`kind: pattern`,
			`role: ${role}`,
			`title: ${payload.title}`,
			`produced_by: ${payload.taskId}`,
			`grounding_validated: ${payload.groundingValidated !== false}`,
			`retrieval_count: 0`,
			`superseded: false`,
			...(supersedesRelPath ? [`supersedes: ${supersedesRelPath}`] : []),
			`---`,
			``,
			`# ${payload.title}`,
			``,
			`## Overview`,
			payload.description,
			``,
			...(payload.codeSnippet ? [`## Implementation Example`, `\`\`\``, payload.codeSnippet, `\`\`\``] : []),
		].join("\n");

		await fs.promises.writeFile(filePath, frontmatter, "utf8");

		const entryId = `${role}/pattern/${path.basename(fileName, ".md")}`;
		this.#graph.addNode({
			id: entryId,
			role,
			kind: "pattern",
			title: payload.title,
			path: filePath,
			metadata: { taskId: payload.taskId },
		});

		if (match && match.score >= 0.5) {
			this.#graph.addEdge(entryId, match.entry.id, "supersedes");
		}

		// Re-run contradiction engine
		this.#contradictionEngine.detectContradictions();

		return {
			action: match && match.score >= 0.5 ? "superseded" : "created",
			filePath,
			entryId,
			supersededPath,
		};
	}

	/**
	 * Write an anti-pattern (recording mistakes and the corrective rule).
	 */
	async writeAntiPattern(payload: AntiPatternPayload): Promise<WritebackResult> {
		const role = payload.role || "backend";
		const kind = "anti-pattern";
		const targetDir = path.join(this.#harvestDir, role, "anti-patterns");
		await fs.promises.mkdir(targetDir, { recursive: true });

		const match = this.#findSimilarEntry(role, kind, payload.title);

		if (match && match.score >= 0.85) {
			const now = new Date();
			await fs.promises.utimes(match.entry.filePath, now, now);
			return {
				action: "refreshed",
				filePath: match.entry.filePath,
				entryId: match.entry.id,
			};
		}

		let supersedesRelPath: string | undefined;
		let supersededPath: string | undefined;

		if (match && match.score >= 0.5) {
			const oldContent = await fs.promises.readFile(match.entry.filePath, "utf8");
			const updatedOld = oldContent.includes("superseded:")
				? oldContent.replace(/^superseded:\s*false/m, "superseded: true")
				: oldContent.replace(/^---\n/, "---\nsuperseded: true\n");

			await fs.promises.writeFile(match.entry.filePath, updatedOld, "utf8");
			supersedesRelPath = path.relative(this.#harvestDir, match.entry.filePath).replace(/\\/g, "/");
			supersededPath = match.entry.filePath;
		}

		const fileName = `${slugify(payload.title)}.md`;
		const filePath = path.join(targetDir, fileName);

		const frontmatter = [
			`---`,
			`kind: anti-pattern`,
			`role: ${role}`,
			`title: ${payload.title}`,
			`produced_by: ${payload.taskId}`,
			`grounding_validated: ${payload.groundingValidated !== false}`,
			`retrieval_count: 0`,
			`superseded: false`,
			...(supersedesRelPath ? [`supersedes: ${supersedesRelPath}`] : []),
			`---`,
			``,
			`# ${payload.title}`,
			``,
			`## Attempted Action`,
			payload.attemptedAction,
			``,
			`## Why It Failed`,
			payload.whyItFailed,
			``,
			`## Corrective Rule`,
			payload.correctiveRule,
		].join("\n");

		await fs.promises.writeFile(filePath, frontmatter, "utf8");

		const entryId = `${role}/anti-pattern/${path.basename(fileName, ".md")}`;
		this.#graph.addNode({
			id: entryId,
			role,
			kind: "anti-pattern",
			title: payload.title,
			path: filePath,
			metadata: { taskId: payload.taskId },
		});

		if (match && match.score >= 0.5) {
			this.#graph.addEdge(entryId, match.entry.id, "supersedes");
		}

		// Re-run contradiction engine
		this.#contradictionEngine.detectContradictions();

		return {
			action: match && match.score >= 0.5 ? "superseded" : "created",
			filePath,
			entryId,
			supersededPath,
		};
	}
}
