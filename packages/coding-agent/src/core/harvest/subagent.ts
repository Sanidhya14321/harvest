/**
 * Subagent Delegation & Git Claim Anti-Hallucination Verification.
 *
 * Captures git HEAD before delegation.
 * Parses completion summaries for commit assertions.
 * Mechanically asserts that git rev-parse HEAD advanced before accepting claims.
 */

import { safeTruncateXml } from "./compaction";
import type { HarvestKnowledgeGraph } from "./graph";
import type { LocalMemoryStore, LocalSessionMemory } from "./memory";
import { type ContextTier, type TierBudgetConfig, allocateDynamicPromptBudget, getTierBudget } from "./model-tier";
import { type SectionPage, tokenize, UnifiedMemoryRetriever } from "./retrieval";
import { getRoleDefinition, type SpecialistRole } from "./roles";
import type { LoadedSkill } from "./skill-context";

export interface GitClaimVerificationResult {
	readonly hasClaim: boolean;
	readonly verified: boolean;
	readonly preHead?: string;
	readonly postHead?: string;
	readonly error?: string;
}

export const GIT_COMMIT_CLAIM_REGEX =
	/\b(?:committed\b|git\s+commit|staged\s+and\s+committed|created\s+(?:a\s+)?commit|committed\s+the\s+changes)\b/i;

/** Get current git HEAD hash using Bun APIs */
export function getGitHead(cwd: string = process.cwd()): string | null {
	try {
		const proc = Bun.spawnSync(["git", "rev-parse", "HEAD"], {
			cwd,
			stdout: "pipe",
			stderr: "pipe",
		});
		if (proc.exitCode === 0) {
			return proc.stdout.toString().trim();
		}
		return null;
	} catch {
		return null;
	}
}

/** Check if text claims that a git commit was made */
export function hasGitCommitClaim(text: string): boolean {
	return GIT_COMMIT_CLAIM_REGEX.test(text);
}

export class HarvestSubagentRunner {
	readonly #workspaceRoot: string;
	#preGitHead: string | null = null;

	constructor(workspaceRoot: string = process.cwd()) {
		this.#workspaceRoot = workspaceRoot;
	}

	/** Record git HEAD prior to starting subagent execution or delegation */
	prepareDelegation(): { preHead: string | null } {
		this.#preGitHead = getGitHead(this.#workspaceRoot);
		return { preHead: this.#preGitHead };
	}

	/**
	 * Verify whether a git commit was claimed, and if so, whether HEAD actually advanced.
	 */
	verifyGitClaim(summary: string): GitClaimVerificationResult {
		const claimsCommit = hasGitCommitClaim(summary);
		if (!claimsCommit) {
			return { hasClaim: false, verified: true };
		}

		const postHead = getGitHead(this.#workspaceRoot);

		// If not a git repo or git failed
		if (this.#preGitHead === null && postHead === null) {
			return {
				hasClaim: true,
				verified: false,
				error: "Git commit claimed, but workspace is not a git repository or git command failed. Hallucinated commit detected.",
			};
		}

		if (this.#preGitHead !== null && postHead === this.#preGitHead) {
			return {
				hasClaim: true,
				verified: false,
				preHead: this.#preGitHead,
				postHead: postHead ?? undefined,
				error: `Git commit claim failed: Agent claimed to have committed changes, but git HEAD did not advance (pre-head: ${this.#preGitHead}, current: ${postHead}). Hallucinated git commit detected.`,
			};
		}

		return {
			hasClaim: true,
			verified: true,
			preHead: this.#preGitHead ?? undefined,
			postHead: postHead ?? undefined,
		};
	}

	get preGitHead(): string | null {
		return this.#preGitHead;
	}
}

export interface SubagentDelegationRequest {
	readonly parentRole: SpecialistRole;
	readonly targetRole: SpecialistRole;
	readonly assignment: string;
	readonly contextWindowTokens?: number;
	readonly modelName?: string;
	readonly targetTier?: ContextTier;
	readonly parentLocalMemory?: LocalSessionMemory | LocalMemoryStore;
	readonly graph?: HarvestKnowledgeGraph;
	readonly workspaceRoot?: string;
	readonly sharedSkills?: readonly LoadedSkill[];
	readonly customDirectives?: readonly string[];
}

export interface ScopedSubagentContext {
	readonly targetRole: SpecialistRole;
	readonly assignment: string;
	readonly tierBudget: TierBudgetConfig;
	readonly scopedPromptContext: string;
	readonly retrievedPages: readonly SectionPage[];
	readonly relevantFiles: readonly string[];
	readonly preGitHead: string | null;
}

export interface DelegationCompletionOptions {
	readonly subagentModifiedFiles?: readonly string[];
	readonly subagentReadFiles?: readonly string[];
	readonly parentLocalMemory?: LocalMemoryStore;
	readonly subagentRole?: SpecialistRole;
}

export interface DelegationCompletionResult {
	readonly gitClaim: GitClaimVerificationResult;
	readonly success: boolean;
	readonly synthesizedSummary: string;
	readonly stagedModifications: readonly string[];
}

export interface MultiAgentDelegationReport {
	readonly totalDelegations: number;
	readonly successful: number;
	readonly failed: number;
	readonly hasConflicts: boolean;
	readonly fileConflicts: readonly {
		readonly filePath: string;
		readonly modifiedBy: readonly SpecialistRole[];
	}[];
	readonly stagedModifications: readonly string[];
	readonly synthesizedSummary: string;
}

export class HarvestSubagentCoordinator {
	readonly #workspaceRoot: string;
	readonly #runner: HarvestSubagentRunner;

	constructor(workspaceRoot: string = process.cwd()) {
		this.#workspaceRoot = workspaceRoot;
		this.#runner = new HarvestSubagentRunner(this.#workspaceRoot);
	}

	get runner(): HarvestSubagentRunner {
		return this.#runner;
	}

	prepareScopedDelegation(request: SubagentDelegationRequest): ScopedSubagentContext {
		const { preHead } = this.#runner.prepareDelegation();

		// Dynamic tier budgeting for the subagent
		const tierBudget = request.targetTier
			? getTierBudget(request.targetTier)
			: allocateDynamicPromptBudget({
					contextWindowTokens: request.contextWindowTokens,
					modelName: request.modelName,
				});

		const targetRoleDef = getRoleDefinition(request.targetRole);
		const queryTokens = new Set(tokenize(request.assignment));

		const GENERIC_EXTENSIONS = new Set([
			"ts",
			"js",
			"tsx",
			"jsx",
			"json",
			"md",
			"html",
			"css",
			"scss",
			"py",
			"rs",
			"go",
			"c",
			"cpp",
			"h",
			"yaml",
			"yml",
			"toml",
			"sql",
			"sh",
			"bash",
		]);
		const GENERIC_DIR_NAMES = new Set([
			"src",
			"lib",
			"test",
			"tests",
			"dist",
			"build",
			"packages",
			"package",
			"app",
			"apps",
		]);

		// Extract relevant files from parent memory matching assignment keywords
		const relevantFiles: string[] = [];
		let scopedParentMemory: LocalSessionMemory | undefined;

		if (request.parentLocalMemory) {
			const mem =
				"snapshot" in request.parentLocalMemory && typeof request.parentLocalMemory.snapshot === "function"
					? request.parentLocalMemory.snapshot()
					: (request.parentLocalMemory as LocalSessionMemory);

			const allCandidateFiles = Array.from(new Set([...mem.readFiles, ...mem.modifiedFiles]));
			const scoredFiles: Array<{ file: string; score: number }> = [];

			for (const file of allCandidateFiles) {
				const fileTokens = tokenize(file);
				let score = 0;
				for (const t of fileTokens) {
					if (queryTokens.has(t) && !GENERIC_EXTENSIONS.has(t) && !GENERIC_DIR_NAMES.has(t)) {
						score += 5;
					}
				}
				if (score > 0) {
					scoredFiles.push({ file, score });
				}
			}

			scoredFiles.sort((a, b) => b.score - a.score);
			for (const sf of scoredFiles) {
				relevantFiles.push(sf.file);
				if (relevantFiles.length >= tierBudget.maxCodePages) break;
			}

			// If no direct token match, include recently modified files up to maxCodePages
			if (relevantFiles.length === 0 && mem.modifiedFiles.length > 0) {
				relevantFiles.push(...mem.modifiedFiles.slice(-tierBudget.maxCodePages));
			}

			// Scope parent memory so retriever does not dump the entire parent session
			scopedParentMemory = {
				sessionId: mem.sessionId,
				readFiles: relevantFiles.filter(f => mem.readFiles.includes(f)),
				modifiedFiles: relevantFiles.filter(f => mem.modifiedFiles.includes(f)),
				verificationRecords: mem.verificationRecords.filter(v => {
					const vTokens = tokenize(v.command);
					return vTokens.some(t => queryTokens.has(t));
				}),
				workingMemoryItems: mem.workingMemoryItems,
				activeSkills: mem.activeSkills,
				touchedSymbols: mem.touchedSymbols?.filter(s => {
					const sTokens = tokenize(s);
					return sTokens.some(t => queryTokens.has(t));
				}),
			};
		}

		// Scoped vectorless RAG with page index
		const retriever = new UnifiedMemoryRetriever({
			workspaceRoot: this.#workspaceRoot,
			graph: request.graph,
			localMemory: scopedParentMemory,
			skills: request.sharedSkills,
		});

		// Query RAG for pages specifically relevant to the subagent's assignment and role
		const retrieval = retriever.retrieve(request.assignment, {
			activeRole: request.targetRole,
			limit: tierBudget.maxKnowledgeHits,
			maxTokens: Math.floor(tierBudget.maxKnowledgeChars / 4),
			includeGlobal: true,
			includeLocal: true,
			includeGraph: true,
			includeSkills: true,
		});

		// Build scoped subagent context XML block
		const lines: string[] = [
			`<subagent-scoped-context role="${request.targetRole}" tier="${tierBudget.tier}">`,
			`# Delegation Directive: ${targetRoleDef.name}`,
			`Target Role: ${request.targetRole}`,
			`Assigned Task: ${request.assignment}`,
			"",
			`Role Capabilities:`,
			...targetRoleDef.capabilities.map(c => `- ${c}`),
			"",
			`Role Directives:`,
			...targetRoleDef.directives.map(d => `- ${d}`),
			"",
		];

		if (relevantFiles.length > 0) {
			lines.push(`## Scoped Relevant Files`);
			for (const f of relevantFiles) {
				lines.push(`- ${f}`);
			}
			lines.push("");
		}

		if (retrieval.formattedContext.length > 0) {
			lines.push(retrieval.formattedContext);
			lines.push("");
		}

		if (request.customDirectives && request.customDirectives.length > 0) {
			lines.push(`## Specific Delegation Instructions:`);
			for (const d of request.customDirectives) {
				lines.push(`- ${d}`);
			}
			lines.push("");
		}

		lines.push(`</subagent-scoped-context>`);

		let scopedPromptContext = lines.join("\n");

		// Enforce subagent's tier budget limits with well-formed XML tag balancing
		const maxAllowedPromptChars = tierBudget.maxSystemPromptChars;
		if (scopedPromptContext.length > maxAllowedPromptChars) {
			scopedPromptContext = safeTruncateXml(scopedPromptContext, maxAllowedPromptChars, "subagent-scoped-context");
		}

		return {
			targetRole: request.targetRole,
			assignment: request.assignment,
			tierBudget,
			scopedPromptContext,
			retrievedPages: retrieval.retrievedPages,
			relevantFiles,
			preGitHead: preHead,
		};
	}

	completeDelegation(summary: string, options: DelegationCompletionOptions = {}): DelegationCompletionResult {
		const gitClaim = this.#runner.verifyGitClaim(summary);

		const subagentRole = options.subagentRole ?? "specialist";
		const modifiedFiles = options.subagentModifiedFiles ?? [];
		const readFiles = options.subagentReadFiles ?? [];

		// Ingest into parent local memory without transcript dumps
		if (options.parentLocalMemory) {
			for (const f of readFiles) {
				options.parentLocalMemory.recordReadFile(f);
			}
			for (const f of modifiedFiles) {
				options.parentLocalMemory.recordModifiedFile(f);
			}

			// Add structured working memory item
			options.parentLocalMemory.addWorkingMemoryItem(
				"finding",
				`Subagent [${subagentRole}] Completion`,
				summary.slice(0, 1000),
				[subagentRole, "subagent-delegation"],
			);
		}

		const success = gitClaim.verified;

		const synthesizedSummary = [
			`[Subagent Delegation Completed: ${subagentRole}]`,
			`Status: ${success ? "SUCCESS" : "FAILED_GIT_VERIFICATION"}`,
			...(gitClaim.hasClaim ? [`Git Head Advanced: ${gitClaim.verified}`] : []),
			...(gitClaim.error ? [`Verification Error: ${gitClaim.error}`] : []),
			...(modifiedFiles.length > 0 ? [`Files Touched: ${modifiedFiles.join(", ")}`] : []),
			``,
			`Summary:`,
			summary.trim(),
		].join("\n");

		return {
			gitClaim,
			success,
			synthesizedSummary,
			stagedModifications: modifiedFiles,
		};
	}
}

/**
 * Multi-Agent Context Distribution Engine.
 *
 * Coordinates concurrent or sequential subagent spawns with scoped,
 * compressed memory distribution across specialist roles.
 */
export class MultiAgentContextDistributor {
	readonly #coordinator: HarvestSubagentCoordinator;

	constructor(workspaceRoot: string = process.cwd()) {
		this.#coordinator = new HarvestSubagentCoordinator(workspaceRoot);
	}

	get coordinator(): HarvestSubagentCoordinator {
		return this.#coordinator;
	}

	distribute(requests: readonly SubagentDelegationRequest[]): ScopedSubagentContext[] {
		return requests.map(req => this.#coordinator.prepareScopedDelegation(req));
	}

	/**
	 * Detect potential file mutation conflicts across concurrent delegation requests
	 * before subagents start execution.
	 */
	detectPreflightConflicts(
		requests: readonly SubagentDelegationRequest[],
	): Array<{ readonly file: string; readonly roles: SpecialistRole[] }> {
		const fileToRoles = new Map<string, Set<SpecialistRole>>();

		for (const req of requests) {
			const scoped = this.#coordinator.prepareScopedDelegation(req);
			for (const f of scoped.relevantFiles) {
				if (!fileToRoles.has(f)) {
					fileToRoles.set(f, new Set());
				}
				fileToRoles.get(f)!.add(req.targetRole);
			}
		}

		const conflicts: Array<{ readonly file: string; readonly roles: SpecialistRole[] }> = [];
		for (const [file, roles] of fileToRoles.entries()) {
			if (roles.size > 1) {
				conflicts.push({ file, roles: Array.from(roles) });
			}
		}

		return conflicts;
	}

	/**
	 * Complete and synthesize multi-agent delegations, detecting file mutation conflicts,
	 * verifying git claims, and ingesting combined findings into parent local memory.
	 */
	synthesizeMultiAgentResults(
		results: readonly {
			readonly role: SpecialistRole;
			readonly summary: string;
			readonly modifiedFiles?: readonly string[];
			readonly readFiles?: readonly string[];
		}[],
		parentLocalMemory?: LocalMemoryStore,
	): MultiAgentDelegationReport {
		let totalSuccessful = 0;
		let totalFailed = 0;
		const allStaged = new Set<string>();
		const fileModCount = new Map<string, Set<SpecialistRole>>();
		const summaryLines: string[] = ["# Multi-Agent Delegation Synthesis Report", ""];

		for (const res of results) {
			const completion = this.#coordinator.completeDelegation(res.summary, {
				subagentRole: res.role,
				subagentModifiedFiles: res.modifiedFiles,
				subagentReadFiles: res.readFiles,
				parentLocalMemory,
			});

			if (completion.success) {
				totalSuccessful++;
			} else {
				totalFailed++;
			}

			summaryLines.push(completion.synthesizedSummary, "");

			if (res.modifiedFiles) {
				for (const f of res.modifiedFiles) {
					allStaged.add(f);
					if (!fileModCount.has(f)) {
						fileModCount.set(f, new Set());
					}
					fileModCount.get(f)!.add(res.role);
				}
			}
		}

		// Detect post-execution file conflicts
		const fileConflicts: Array<{ readonly filePath: string; readonly modifiedBy: readonly SpecialistRole[] }> = [];
		for (const [filePath, roles] of fileModCount.entries()) {
			if (roles.size > 1) {
				fileConflicts.push({ filePath, modifiedBy: Array.from(roles) });
			}
		}

		if (fileConflicts.length > 0) {
			summaryLines.push("## Warning: Conflicting Modifications Detected");
			for (const c of fileConflicts) {
				summaryLines.push(`- File '${c.filePath}' modified by multiple roles: ${c.modifiedBy.join(", ")}`);
			}
			summaryLines.push("");
		}

		return {
			totalDelegations: results.length,
			successful: totalSuccessful,
			failed: totalFailed,
			hasConflicts: fileConflicts.length > 0,
			fileConflicts,
			stagedModifications: Array.from(allStaged),
			synthesizedSummary: summaryLines.join("\n").trim(),
		};
	}
}
