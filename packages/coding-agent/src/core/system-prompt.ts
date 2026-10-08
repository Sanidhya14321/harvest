/**
 * System Prompt Construction & Harvest Dynamic Budgeting Facade.
 *
 * Integrates Grounded Coding Task Contract injection, context-tiered
 * dynamic prompt budgeting, and specialist role addenda.
 */

import { injectCodingContract } from "./harvest/coding-contract";
import type { HarvestKnowledgeGraph } from "./harvest/graph";
import type { LocalMemoryStore, LocalSessionMemory } from "./harvest/memory";
import { allocateDynamicPromptBudget, type TierBudgetConfig } from "./harvest/model-tier";
import { assembleHarvestPrompt } from "./harvest/prompt-assembler";
import { getRoleDefinition, type SpecialistRole } from "./harvest/roles";

export * from "../system-prompt";
export * from "./harvest/prompt-assembler";

export interface HarvestPromptOptions {
	readonly basePrompt: string;
	readonly role?: SpecialistRole;
	readonly intent?: "informational" | "mutation" | "complex";
	readonly contextWindowTokens?: number;
	readonly modelName?: string;
	readonly skillsPrompt?: string;
	readonly knowledgePrompt?: string;
	readonly taskQuery?: string;
	readonly localMemory?: LocalSessionMemory | LocalMemoryStore;
	readonly graph?: HarvestKnowledgeGraph;
	readonly workspaceRoot?: string;
}

export interface AssembledHarvestPrompt {
	readonly prompt: string;
	readonly tierBudget: TierBudgetConfig;
	readonly role: SpecialistRole;
	readonly contractInjected: boolean;
}

/**
 * Assemble a tier-budgeted system prompt with specialist role addenda
 * and the Grounded Coding Task Contract.
 */
export function assembleHarvestSystemPrompt(options: HarvestPromptOptions): AssembledHarvestPrompt {
	if (
		options.localMemory ||
		options.graph ||
		(options.taskQuery && !options.skillsPrompt && !options.knowledgePrompt)
	) {
		const res = assembleHarvestPrompt(options);
		return {
			prompt: res.prompt,
			tierBudget: res.tierBudget,
			role: res.role,
			contractInjected: res.contractInjected,
		};
	}

	const role = options.role ?? "backend";
	const roleDef = getRoleDefinition(role);
	const budget = allocateDynamicPromptBudget({
		contextWindowTokens: options.contextWindowTokens,
		modelName: options.modelName,
	});

	let assembled = options.basePrompt;

	// Append role addendum
	if (roleDef.promptAddendum) {
		assembled += `\n\n# Role Directive: ${roleDef.name}\n${roleDef.promptAddendum}\n`;
		if (roleDef.directives.length > 0) {
			assembled += `\nDirectives:\n${roleDef.directives.map(d => `- ${d}`).join("\n")}\n`;
		}
	}

	// Append bounded skills
	if (options.skillsPrompt) {
		const rawSkills = options.skillsPrompt.trim();
		if (rawSkills.length > 0) {
			let boundedSkills = rawSkills.slice(0, budget.maxInlineSkillsChars).trim();
			if (rawSkills.includes("<harvest-skills>")) {
				if (!boundedSkills.includes("</harvest-skills>")) {
					const openTagIdx = boundedSkills.indexOf("<harvest-skills>");
					if (openTagIdx !== -1) {
						const lastNewline = boundedSkills.lastIndexOf("\n");
						if (lastNewline > openTagIdx + "<harvest-skills>".length) {
							boundedSkills = boundedSkills.slice(0, lastNewline).trim() + "\n</harvest-skills>";
							assembled += `\n\n${boundedSkills}\n`;
						}
					}
				} else {
					assembled += `\n\n${boundedSkills}\n`;
				}
			} else if (boundedSkills.length > 0) {
				assembled += `\n\n${boundedSkills}\n`;
			}
		}
	}

	// Append bounded knowledge
	if (options.knowledgePrompt) {
		const rawKnowledge = options.knowledgePrompt.trim();
		if (rawKnowledge.length > 0) {
			let boundedKnowledge = rawKnowledge.slice(0, budget.maxKnowledgeChars).trim();
			if (rawKnowledge.includes("<harvest-knowledge>")) {
				if (!boundedKnowledge.includes("</harvest-knowledge>")) {
					const openTagIdx = boundedKnowledge.indexOf("<harvest-knowledge>");
					if (openTagIdx !== -1) {
						const lastNewline = boundedKnowledge.lastIndexOf("\n");
						if (lastNewline > openTagIdx + "<harvest-knowledge>".length) {
							boundedKnowledge = boundedKnowledge.slice(0, lastNewline).trim() + "\n</harvest-knowledge>";
							assembled += `\n\n${boundedKnowledge}\n`;
						}
					}
				} else {
					assembled += `\n\n${boundedKnowledge}\n`;
				}
			} else if (boundedKnowledge.length > 0) {
				assembled += `\n\n${boundedKnowledge}\n`;
			}
		}
	}

	// Inject Grounded Coding Task Contract for mutations
	const beforeContract = assembled;
	assembled = injectCodingContract(assembled, options.intent ?? "mutation");
	const contractInjected = assembled !== beforeContract;

	// Enforce system prompt length limit for tier
	if (assembled.length > budget.maxSystemPromptChars) {
		assembled = assembled.slice(0, budget.maxSystemPromptChars) + "\n...[context budgeted]";
	}

	return {
		prompt: assembled,
		tierBudget: budget,
		role,
		contractInjected,
	};
}
