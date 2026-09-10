/**
 * Institutional Memory & Contradiction Detection Engine.
 *
 * Scans patterns and anti-patterns within each role for conflicting directives.
 * Flags pairwise keyword overlaps (>= 2 keywords > 3 chars) as contradictions.
 * Records 'contradicts' edges in graph.json and updates .harvest/pending_review.md.
 */

import * as fs from "node:fs";
import * as path from "node:path";
import { extractTranscriptEvidence, type TranscriptToolCall, type VerificationAuditRecord } from "../compaction/utils";
import { HarvestKnowledgeGraph } from "./graph";

export interface KnowledgeEntry {
	readonly id: string;
	readonly role: string;
	readonly kind: "pattern" | "anti-pattern";
	readonly title: string;
	readonly filePath: string;
	readonly content: string;
	readonly keywords: readonly string[];
	readonly supersedes?: string;
	readonly superseded?: boolean;
	readonly producedBy?: string;
}

export interface Contradiction {
	readonly id: string;
	readonly role: string;
	readonly patternId: string;
	readonly patternTitle: string;
	readonly antiPatternId: string;
	readonly antiPatternTitle: string;
	readonly sharedKeywords: readonly string[];
	readonly detectedAt: number;
}

const STOP_WORDS = new Set([
	"this",
	"that",
	"with",
	"from",
	"when",
	"then",
	"have",
	"been",
	"make",
	"will",
	"should",
	"could",
	"would",
	"about",
	"where",
	"which",
	"there",
	"their",
	"other",
	"avoid",
	"using",
	"always",
	"never",
]);

/**
 * Extract salient keywords (> 3 chars, non-stopwords) from text.
 */
export function extractKeywords(text: string): string[] {
	return text
		.toLowerCase()
		.replace(/[^a-z0-9_\-\s]/g, " ")
		.split(/\s+/)
		.filter(w => w.length > 3 && !STOP_WORDS.has(w));
}

export class ContradictionEngine {
	readonly #workspaceRoot: string;
	readonly #harvestDir: string;
	readonly #graph: HarvestKnowledgeGraph;
	readonly #pendingReviewPath: string;

	constructor(workspaceRoot: string = process.cwd(), graph?: HarvestKnowledgeGraph) {
		this.#workspaceRoot = path.resolve(workspaceRoot);
		this.#harvestDir = path.join(this.#workspaceRoot, ".harvest");
		this.#pendingReviewPath = path.join(this.#harvestDir, "pending_review.md");
		this.#graph = graph ?? new HarvestKnowledgeGraph(this.#workspaceRoot);
	}

	/**
	 * Parse a single markdown file into a KnowledgeEntry.
	 */
	parseKnowledgeFile(filePath: string): KnowledgeEntry | null {
		try {
			const content = fs.readFileSync(filePath, "utf8");
			let kind: "pattern" | "anti-pattern" = filePath.includes("anti-pattern") ? "anti-pattern" : "pattern";
			let role = "backend";
			let title = path.basename(filePath, ".md");
			let supersedes: string | undefined;
			let superseded = false;
			let producedBy: string | undefined;

			if (content.startsWith("---")) {
				const end = content.indexOf("\n---", 3);
				if (end !== -1) {
					const fm = content.slice(3, end);
					const kindMatch = fm.match(/^kind:\s*(.*)$/m);
					if (kindMatch) {
						kind = kindMatch[1].trim() === "anti-pattern" ? "anti-pattern" : "pattern";
					}
					const roleMatch = fm.match(/^role:\s*(.*)$/m);
					if (roleMatch) role = roleMatch[1].trim();
					const titleMatch = fm.match(/^title:\s*(.*)$/m);
					if (titleMatch) title = titleMatch[1].trim();
					const superMatch = fm.match(/^supersedes:\s*(.*)$/m);
					if (superMatch) supersedes = superMatch[1].trim();
					const superBoolMatch = fm.match(/^superseded:\s*(true|false)$/m);
					if (superBoolMatch) superseded = superBoolMatch[1] === "true";
					const prodMatch = fm.match(/^produced_by:\s*(.*)$/m);
					if (prodMatch) producedBy = prodMatch[1].trim();
				}
			}

			const keywords = Array.from(new Set([...extractKeywords(title), ...extractKeywords(content).slice(0, 30)]));
			const id = `${role}/${kind}/${path.basename(filePath, ".md")}`;

			return {
				id,
				role,
				kind,
				title,
				filePath,
				content,
				keywords,
				supersedes,
				superseded,
				producedBy,
			};
		} catch {
			return null;
		}
	}

	/**
	 * Scan all knowledge files under .harvest/ across roles.
	 */
	loadAllEntries(): KnowledgeEntry[] {
		const entries: KnowledgeEntry[] = [];
		if (!fs.existsSync(this.#harvestDir)) return entries;

		const scanDir = (dir: string) => {
			if (!fs.existsSync(dir)) return;
			const items = fs.readdirSync(dir);
			for (const item of items) {
				const full = path.join(dir, item);
				const stat = fs.statSync(full);
				if (stat.isDirectory()) {
					scanDir(full);
				} else if (item.endsWith(".md") && !item.startsWith("pending_review")) {
					const entry = this.parseKnowledgeFile(full);
					if (entry) entries.push(entry);
				}
			}
		};

		scanDir(this.#harvestDir);
		return entries;
	}

	/**
	 * Detect pairwise contradictions between patterns and anti-patterns in the same role.
	 * Returns list of detected contradictions and updates .harvest/pending_review.md.
	 */
	detectContradictions(): Contradiction[] {
		const entries = this.loadAllEntries().filter(e => !e.superseded);
		const byRole: Record<string, { patterns: KnowledgeEntry[]; antiPatterns: KnowledgeEntry[] }> = {};

		for (const e of entries) {
			if (!byRole[e.role]) {
				byRole[e.role] = { patterns: [], antiPatterns: [] };
			}
			if (e.kind === "pattern") {
				byRole[e.role].patterns.push(e);
			} else {
				byRole[e.role].antiPatterns.push(e);
			}
		}

		const contradictions: Contradiction[] = [];

		for (const [role, groups] of Object.entries(byRole)) {
			for (const pat of groups.patterns) {
				for (const anti of groups.antiPatterns) {
					// Check keyword overlap (>= 2 keywords > 3 chars)
					const patSet = new Set(pat.keywords);
					const shared = anti.keywords.filter(k => patSet.has(k));

					if (shared.length >= 2) {
						const contradictionId = `contra-${pat.id}-${anti.id}`;
						contradictions.push({
							id: contradictionId,
							role,
							patternId: pat.id,
							patternTitle: pat.title,
							antiPatternId: anti.id,
							antiPatternTitle: anti.title,
							sharedKeywords: shared,
							detectedAt: Date.now(),
						});

						// Record 'contradicts' edge in knowledge graph
						this.#graph.addEdge(pat.id, anti.id, "contradicts", { sharedKeywords: shared });
						this.#graph.addEdge(anti.id, pat.id, "contradicts", { sharedKeywords: shared });
					}
				}
			}
		}

		this.#writePendingReview(contradictions);
		return contradictions;
	}

	#writePendingReview(contradictions: Contradiction[]): void {
		try {
			if (contradictions.length === 0) {
				if (fs.existsSync(this.#pendingReviewPath)) {
					fs.writeFileSync(
						this.#pendingReviewPath,
						`# Harvest Knowledge Review Queue\n\n*No unresolved contradictions detected.*\n`,
						"utf8",
					);
				}
				return;
			}

			const lines: string[] = [
				`# Harvest Knowledge Review Queue`,
				``,
				`The contradiction detection engine flagged the following conflicting rules. Use \`/review\` or resolve them by marking one as \`superseded: true\`.`,
				``,
			];

			for (const c of contradictions) {
				lines.push(`## Contradiction: [${c.role.toUpperCase()}]`);
				lines.push(`- **Pattern**: "${c.patternTitle}" (\`${c.patternId}\`)`);
				lines.push(`- **Anti-Pattern**: "${c.antiPatternTitle}" (\`${c.antiPatternId}\`)`);
				lines.push(`- **Shared Keywords**: \`${c.sharedKeywords.join(", ")}\``);
				lines.push(`- **Status**: \`PENDING_REVIEW\``);
				lines.push(``);
			}

			fs.mkdirSync(this.#harvestDir, { recursive: true });
			fs.writeFileSync(this.#pendingReviewPath, lines.join("\n"), "utf8");
		} catch {}
	}

	get pendingReviewPath(): string {
		return this.#pendingReviewPath;
	}
}

export interface WorkingMemoryItem {
	readonly id: string;
	readonly category: "hypothesis" | "goal" | "decision" | "finding" | "note";
	readonly title: string;
	readonly content: string;
	readonly timestamp: number;
	readonly tags?: readonly string[];
}

export interface LocalSessionMemory {
	readonly sessionId: string;
	readonly readFiles: readonly string[];
	readonly modifiedFiles: readonly string[];
	readonly verificationRecords: readonly VerificationAuditRecord[];
	readonly workingMemoryItems: readonly WorkingMemoryItem[];
	readonly activeSkills?: readonly string[];
	readonly touchedSymbols?: readonly string[];
}

export class LocalMemoryStore {
	readonly #sessionId: string;
	readonly #readFiles: Set<string>;
	readonly #modifiedFiles: Map<string, number>;
	#verificationRecords: VerificationAuditRecord[];
	#workingMemoryItems: WorkingMemoryItem[];
	readonly #activeSkills: Set<string>;
	readonly #touchedSymbols: Set<string>;

	constructor(sessionId?: string, initial?: Partial<LocalSessionMemory>) {
		this.#sessionId = sessionId ?? `session-${Date.now()}`;
		this.#readFiles = new Set(initial?.readFiles ?? []);
		this.#modifiedFiles = new Map();
		if (initial?.modifiedFiles) {
			for (const f of initial.modifiedFiles) {
				this.#modifiedFiles.set(f, Date.now());
			}
		}
		this.#verificationRecords = [...(initial?.verificationRecords ?? [])];
		this.#workingMemoryItems = [...(initial?.workingMemoryItems ?? [])];
		this.#activeSkills = new Set(initial?.activeSkills ?? []);
		this.#touchedSymbols = new Set(initial?.touchedSymbols ?? []);
	}

	get sessionId(): string {
		return this.#sessionId;
	}

	recordReadFile(filePath: string): void {
		if (filePath) this.#readFiles.add(filePath);
	}

	recordModifiedFile(filePath: string, timestamp: number = Date.now()): void {
		if (filePath) {
			this.#modifiedFiles.set(filePath, timestamp);
			this.#updateVerificationStaleness(filePath, timestamp);
		}
	}

	recordVerification(record: VerificationAuditRecord): void {
		let finalRecord = record;
		if (record.status === "PASS") {
			for (const [modFile, modTime] of this.#modifiedFiles.entries()) {
				if (modTime > record.timestamp) {
					finalRecord = {
						...record,
						status: "STALE",
						stalenessReason: `Modified after test passed: '${modFile}'`,
					};
					break;
				}
			}
		}
		this.#verificationRecords.push(finalRecord);
	}

	recordSkill(skillName: string): void {
		if (skillName) this.#activeSkills.add(skillName);
	}

	recordSymbol(symbol: string): void {
		if (symbol) this.#touchedSymbols.add(symbol);
	}

	addWorkingMemoryItem(
		category: WorkingMemoryItem["category"],
		title: string,
		content: string,
		tags?: readonly string[],
	): WorkingMemoryItem {
		const item: WorkingMemoryItem = {
			id: `wm-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`,
			category,
			title,
			content,
			timestamp: Date.now(),
			tags,
		};
		this.#workingMemoryItems.push(item);
		return item;
	}

	updateWorkingMemoryItem(
		id: string,
		updates: Partial<Pick<WorkingMemoryItem, "category" | "title" | "content" | "tags">>,
	): boolean {
		const idx = this.#workingMemoryItems.findIndex(item => item.id === id);
		if (idx === -1) return false;
		const current = this.#workingMemoryItems[idx];
		this.#workingMemoryItems[idx] = {
			...current,
			...updates,
			timestamp: Date.now(),
		};
		return true;
	}

	removeWorkingMemoryItem(id: string): boolean {
		const initialLen = this.#workingMemoryItems.length;
		this.#workingMemoryItems = this.#workingMemoryItems.filter(item => item.id !== id);
		return this.#workingMemoryItems.length < initialLen;
	}

	getWorkingMemoryItems(category?: WorkingMemoryItem["category"]): readonly WorkingMemoryItem[] {
		if (!category) return [...this.#workingMemoryItems];
		return this.#workingMemoryItems.filter(item => item.category === category);
	}

	ingestToolCalls(toolCalls: readonly TranscriptToolCall[]): void {
		const evidence = extractTranscriptEvidence(toolCalls);
		for (const r of evidence.readFiles) {
			this.recordReadFile(r);
		}

		// Preserve tool call timestamps for modified files if available
		const fileTimestamps = new Map<string, number>();
		for (const call of toolCalls) {
			const args = call.args ?? {};
			const rawPath =
				(typeof args.path === "string" && args.path) ||
				(typeof args.filePath === "string" && args.filePath) ||
				(typeof args.targetFile === "string" && args.targetFile) ||
				(typeof args.TargetFile === "string" && args.TargetFile) ||
				(typeof args.target_file === "string" && args.target_file) ||
				(typeof args.file === "string" && args.file) ||
				(typeof args.file_path === "string" && args.file_path) ||
				(typeof args.AbsolutePath === "string" && args.AbsolutePath) ||
				null;
			if (rawPath && call.timestamp) {
				fileTimestamps.set(rawPath, Math.max(fileTimestamps.get(rawPath) ?? 0, call.timestamp));
			}
		}

		for (const m of evidence.modifiedFiles) {
			this.recordModifiedFile(m, fileTimestamps.get(m) ?? Date.now());
		}
		for (const v of evidence.verificationRecords) {
			this.recordVerification(v);
		}
	}

	#updateVerificationStaleness(modifiedFile: string, modTimestamp: number): void {
		const updated: VerificationAuditRecord[] = [];
		for (const v of this.#verificationRecords) {
			if (v.status === "PASS" && modTimestamp > v.timestamp) {
				updated.push({
					...v,
					status: "STALE",
					stalenessReason: `Modified after test passed: '${modifiedFile}'`,
				});
			} else {
				updated.push(v);
			}
		}
		this.#verificationRecords = updated;
	}

	snapshot(): LocalSessionMemory {
		return {
			sessionId: this.#sessionId,
			readFiles: Array.from(this.#readFiles),
			modifiedFiles: Array.from(this.#modifiedFiles.keys()),
			verificationRecords: [...this.#verificationRecords],
			workingMemoryItems: [...this.#workingMemoryItems],
			activeSkills: Array.from(this.#activeSkills),
			touchedSymbols: Array.from(this.#touchedSymbols),
		};
	}

	prune(maxItems: number = 20): void {
		if (this.#workingMemoryItems.length > maxItems) {
			this.#workingMemoryItems = this.#workingMemoryItems.slice(-maxItems);
		}
	}

	clear(): void {
		this.#readFiles.clear();
		this.#modifiedFiles.clear();
		this.#verificationRecords = [];
		this.#workingMemoryItems = [];
		this.#activeSkills.clear();
		this.#touchedSymbols.clear();
	}
}

export class GlobalMemoryStore {
	readonly #contradictionEngine: ContradictionEngine;
	readonly #graph: HarvestKnowledgeGraph;

	constructor(workspaceRoot: string = process.cwd(), graph?: HarvestKnowledgeGraph) {
		this.#graph = graph ?? new HarvestKnowledgeGraph(workspaceRoot);
		this.#contradictionEngine = new ContradictionEngine(workspaceRoot, this.#graph);
	}

	get contradictionEngine(): ContradictionEngine {
		return this.#contradictionEngine;
	}

	get graph(): HarvestKnowledgeGraph {
		return this.#graph;
	}

	loadAllEntries(): KnowledgeEntry[] {
		return this.#contradictionEngine.loadAllEntries();
	}

	loadActiveEntries(): KnowledgeEntry[] {
		return this.#contradictionEngine.loadAllEntries().filter(e => !e.superseded);
	}

	detectContradictions(): Contradiction[] {
		return this.#contradictionEngine.detectContradictions();
	}
}
