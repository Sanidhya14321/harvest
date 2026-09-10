/**
 * Multi-Disjoint 3-Tier Edit Tool & Atomic Mutator.
 *
 * Enforces pre-read, asserts freshness, matches multi-disjoint replacements
 * in reverse offset order, checkpoints turn state, and writes atomically.
 */

import * as fs from "node:fs";
import * as path from "node:path";
import { generateUnifiedDiff } from "./edit-diff";
import { atomicWriteFile, type FileSession } from "./file-session";
import type { FreshnessTracker } from "./freshness";
import { findEditMatch, type MatchSuccess } from "./matcher";

export interface SingleEdit {
	readonly oldText: string;
	readonly newText: string;
}

export interface EditToolInput {
	readonly path: string;
	readonly edits?: readonly SingleEdit[];
	readonly oldText?: string;
	readonly newText?: string;
	readonly turnId?: string;
}

export interface AppliedEditRecord {
	readonly start: number;
	readonly end: number;
	readonly tier: 1 | 2 | 3;
	readonly confidence: number;
	readonly oldText: string;
	readonly newText: string;
}

export interface EditToolResult {
	readonly success: boolean;
	readonly path: string;
	readonly appliedCount: number;
	readonly diff: string;
	readonly appliedEdits: readonly AppliedEditRecord[];
	readonly error?: string;
}

export interface EditDependencies {
	readonly freshnessTracker?: FreshnessTracker;
	readonly fileSession?: FileSession;
	readonly workspaceRoot?: string;
}

export class EditMutator {
	readonly #freshnessTracker?: FreshnessTracker;
	readonly #fileSession?: FileSession;
	readonly #workspaceRoot: string;

	constructor(dependencies: EditDependencies = {}) {
		this.#freshnessTracker = dependencies.freshnessTracker;
		this.#fileSession = dependencies.fileSession;
		this.#workspaceRoot = path.resolve(dependencies.workspaceRoot ?? process.cwd());
	}

	#canonicalize(filePath: string): string {
		const absolute = path.isAbsolute(filePath) ? filePath : path.resolve(this.#workspaceRoot, filePath);
		try {
			return fs.realpathSync.native ? fs.realpathSync.native(absolute) : fs.realpathSync(absolute);
		} catch {
			return path.normalize(absolute);
		}
	}

	/**
	 * Execute multi-disjoint atomic edit on target file.
	 */
	async execute(input: EditToolInput): Promise<EditToolResult> {
		const canonical = this.#canonicalize(input.path);
		const displayPath = path.relative(this.#workspaceRoot, canonical).replace(/\\/g, "/") || input.path;

		// Normalize edit inputs
		const edits: SingleEdit[] = [];
		if (Array.isArray(input.edits) && input.edits.length > 0) {
			for (const e of input.edits) {
				if (typeof e.oldText === "string" && typeof e.newText === "string") {
					edits.push({ oldText: e.oldText, newText: e.newText });
				}
			}
		} else if (typeof input.oldText === "string" && typeof input.newText === "string") {
			edits.push({ oldText: input.oldText, newText: input.newText });
		}

		if (edits.length === 0) {
			return {
				success: false,
				path: displayPath,
				appliedCount: 0,
				diff: "",
				appliedEdits: [],
				error: "No edits specified: provide either `edits: [{ oldText, newText }]` or `oldText` and `newText`.",
			};
		}

		// Verify file existence
		if (!fs.existsSync(canonical)) {
			return {
				success: false,
				path: displayPath,
				appliedCount: 0,
				diff: "",
				appliedEdits: [],
				error: `Cannot edit '${displayPath}': file does not exist on disk.`,
			};
		}

		// Freshness assertion
		if (this.#freshnessTracker) {
			try {
				this.#freshnessTracker.assertCurrent(canonical);
			} catch (err: unknown) {
				const msg = err instanceof Error ? err.message : String(err);
				return {
					success: false,
					path: displayPath,
					appliedCount: 0,
					diff: "",
					appliedEdits: [],
					error: msg,
				};
			}
		}

		// Read current on-disk content
		const originalContent = await fs.promises.readFile(canonical, "utf8");
		const canonicalContent = originalContent.replace(/\r\n/g, "\n");

		// Find matches for each edit block against the ORIGINAL content
		interface ResolvedMatch {
			readonly editIndex: number;
			readonly match: MatchSuccess;
			readonly newText: string;
		}

		const resolvedMatches: ResolvedMatch[] = [];

		for (let i = 0; i < edits.length; i++) {
			const { oldText, newText } = edits[i];
			const matchResult = findEditMatch(canonicalContent, oldText);

			if (!matchResult.matched) {
				const diag = matchResult.diagnostics;
				return {
					success: false,
					path: displayPath,
					appliedCount: 0,
					diff: "",
					appliedEdits: [],
					error: `Edit ${i + 1}/${edits.length} failed to match in '${displayPath}'.\nReason: ${diag.reason}\nClosest candidate line: ${diag.closestLineNumber} (similarity: ${diag.similarityPercentage}%)\nCandidate excerpt:\n${diag.excerpt}`,
				};
			}

			resolvedMatches.push({
				editIndex: i,
				match: matchResult,
				newText: newText.replace(/\r\n/g, "\n"),
			});
		}

		// Verify that matches are disjoint (do not overlap)
		resolvedMatches.sort((a, b) => a.match.start - b.match.start);
		for (let i = 0; i < resolvedMatches.length - 1; i++) {
			const curr = resolvedMatches[i];
			const next = resolvedMatches[i + 1];
			if (curr.match.end > next.match.start) {
				return {
					success: false,
					path: displayPath,
					appliedCount: 0,
					diff: "",
					appliedEdits: [],
					error: `Overlapping edits detected in '${displayPath}': edit at range [${curr.match.start}, ${curr.match.end}] overlaps with edit at [${next.match.start}, ${next.match.end}]. Edits must target disjoint regions.`,
				};
			}
		}

		// Apply replacements in reverse order of character offset
		const sortedReverse = [...resolvedMatches].sort((a, b) => b.match.start - a.match.start);
		let modifiedContent = canonicalContent;
		const appliedRecords: AppliedEditRecord[] = [];

		for (const resolved of sortedReverse) {
			const { match, newText } = resolved;
			modifiedContent =
				modifiedContent.slice(0, match.start) + newText + modifiedContent.slice(match.end);

			appliedRecords.push({
				start: match.start,
				end: match.end,
				tier: match.tier,
				confidence: match.confidence,
				oldText: match.matchedText,
				newText,
			});
		}

		// Turn checkpointing
		const turnId = input.turnId ?? `turn-${Date.now()}`;
		if (this.#fileSession) {
			this.#fileSession.recordMutation(turnId, canonical, originalContent, modifiedContent);
		}

		// Atomic write with lock resilience
		await atomicWriteFile(canonical, modifiedContent);

		// Update freshness tracker
		if (this.#freshnessTracker) {
			this.#freshnessTracker.recordMutation(canonical, modifiedContent);
		}

		const diff = generateUnifiedDiff(displayPath, canonicalContent, modifiedContent);

		return {
			success: true,
			path: displayPath,
			appliedCount: appliedRecords.length,
			diff,
			appliedEdits: appliedRecords.reverse(),
		};
	}
}
