/**
 * Harvest Prompt Assembler & Memory Context Integration Engine.
 *
 * Assembles dynamic, tier-budgeted system prompts enriched with:
 * 1. Specialist role directives & prompt addenda
 * 2. Grounded Coding Task Contract
 * 3. Relevant Global Memory (.harvest/ patterns, anti-patterns, graph)
 *    retrieved via vectorless RAG with page indexing
 * 4. Relevant Local Memory (working memory, read/modified files, verifications)
 * 5. Bounded reusable skills formatted for prompt injection
 */

import { injectCodingContract } from "./coding-contract";
import { HarvestContextCompactor, type CompressedContextResult, safeTruncateXml } from "./compaction";
import type { HarvestKnowledgeGraph } from "./graph";
import type { LocalMemoryStore, LocalSessionMemory } from "./memory";
import { type ContextTier, type TierBudgetConfig, allocateDynamicPromptBudget } from "./model-tier";
import type { SectionPage } from "./retrieval";
import { getRoleDefinition, type SpecialistRole } from "./roles";
import { type LoadedSkill, SkillContextManager } from "./skill-context";

export interface HarvestPromptAssemblerOptions {
	readonly basePrompt: string;
	readonly taskQuery?: string;
	readonly role?: SpecialistRole;
	readonly intent?: "informational" | "mutation" | "complex";
	readonly contextWindowTokens?: number;
	readonly modelName?: string;
	readonly tier?: ContextTier;
	readonly localMemory?: LocalSessionMemory | LocalMemoryStore;
	readonly graph?: HarvestKnowledgeGraph;
	readonly workspaceRoot?: string;
	readonly skills?: readonly LoadedSkill[];
	readonly skillsPrompt?: string;
	readonly knowledgePrompt?: string;
	readonly compactor?: HarvestContextCompactor;
}

export interface AssembledHarvestPromptResult {
	readonly prompt: string;
	readonly tierBudget: TierBudgetConfig;
	readonly role: SpecialistRole;
	readonly contractInjected: boolean;
	readonly compressedContext?: CompressedContextResult;
	readonly retrievedPages: readonly SectionPage[];
}

export class HarvestPromptAssembler {
	readonly #workspaceRoot: string;
	readonly #skillContextManager: SkillContextManager;

	constructor(workspaceRoot: string = process.cwd()) {
		this.#workspaceRoot = workspaceRoot;
		this.#skillContextManager = new SkillContextManager(this.#workspaceRoot);
	}

	/**
	 * Assemble a complete, tier-budgeted system prompt with compressed local & global memory.
	 */
	assemble(options: HarvestPromptAssemblerOptions): AssembledHarvestPromptResult {
		const role = options.role ?? "backend";
		const roleDef = getRoleDefinition(role);
		const budget = allocateDynamicPromptBudget({
			contextWindowTokens: options.contextWindowTokens,
			modelName: options.modelName,
		});

		let assembled = options.basePrompt;

		// 1. Role Directives & Addendum
		if (roleDef.promptAddendum) {
			assembled += `\n\n# Role Directive: ${roleDef.name}\n${roleDef.promptAddendum}\n`;
			if (roleDef.directives.length > 0) {
				assembled += `\nDirectives:\n${roleDef.directives.map(d => `- ${d}`).join("\n")}\n`;
			}
		}

		// 2. Local & Global Memory via Vectorless RAG & Compaction
		let compressedContext: CompressedContextResult | undefined;
		let retrievedPages: SectionPage[] = [];

		if (options.localMemory || options.graph || options.taskQuery) {
			const compactor = options.compactor ?? new HarvestContextCompactor(this.#workspaceRoot, options.graph);

			const emptyMem: LocalSessionMemory = {
				sessionId: "ad-hoc",
				readFiles: [],
				modifiedFiles: [],
				verificationRecords: [],
				workingMemoryItems: [],
			};

			const memInput = options.localMemory ?? emptyMem;

			compressedContext = compactor.compressContext(memInput, {
				taskQuery: options.taskQuery,
				targetBudget: budget,
				role,
				maxLocalChars: Math.floor(budget.maxKnowledgeChars * 0.5),
				maxGlobalChars: Math.floor(budget.maxKnowledgeChars * 0.5),
			});

			retrievedPages = [...compressedContext.localMemoryPages, ...compressedContext.globalMemoryPages];

			if (compressedContext.formattedContext.length > 0) {
				assembled += `\n\n${compressedContext.formattedContext}\n`;
			}
		} else if (options.knowledgePrompt) {
			// Fallback: raw knowledge prompt if supplied
			const bounded = this.#safelyBoundBlock(
				options.knowledgePrompt.trim(),
				budget.maxKnowledgeChars,
				"harvest-knowledge",
			);
			if (bounded.length > 0) {
				assembled += `\n\n${bounded}\n`;
			}
		}

		// 3. Skills Context (relevant skills bounded by tier)
		if (options.skillsPrompt) {
			const bounded = this.#safelyBoundBlock(
				options.skillsPrompt.trim(),
				budget.maxInlineSkillsChars,
				"harvest-skills",
			);
			if (bounded.length > 0) {
				assembled += `\n\n${bounded}\n`;
			}
		} else if (options.taskQuery) {
			// Query-guided relevant skill formatting
			const formattedSkills = this.#skillContextManager.formatSkillsPrompt({
				query: options.taskQuery,
				maxSkills: budget.maxInlineSkills,
				maxTotalChars: budget.maxInlineSkillsChars,
			});
			if (formattedSkills.length > 0) {
				assembled += `\n\n${formattedSkills}\n`;
			}
		}

		// 4. Grounded Coding Task Contract for mutations / complex tasks
		const beforeContract = assembled;
		assembled = injectCodingContract(assembled, options.intent ?? "mutation");
		const contractInjected = assembled !== beforeContract;

		// 5. Enforce system prompt length limit for tier with safe XML tag balancing
		if (assembled.length > budget.maxSystemPromptChars) {
			assembled = safeTruncateXml(assembled, budget.maxSystemPromptChars);
		}

		return {
			prompt: assembled,
			tierBudget: budget,
			role,
			contractInjected,
			compressedContext,
			retrievedPages,
		};
	}

	#safelyBoundBlock(raw: string, maxChars: number, tagName: string): string {
		if (!raw || raw.length === 0) return "";
		if (raw.length <= maxChars) return raw;
		return safeTruncateXml(raw, maxChars, tagName);
	}
}

/**
 * Functional entrypoint to assemble Harvest prompt with compressed memory context.
 */
export function assembleHarvestPrompt(options: HarvestPromptAssemblerOptions): AssembledHarvestPromptResult {
	const assembler = new HarvestPromptAssembler(options.workspaceRoot);
	return assembler.assemble(options);
}
