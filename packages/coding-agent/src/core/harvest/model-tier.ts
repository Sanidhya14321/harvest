/**
 * Context-Tiered Dynamic Prompt Budgeting & Escalation.
 *
 * Dynamically sizes prompt components based on model context window:
 * tiny (<16K), small (16K-64K), standard (64K-200K), large (>200K).
 * Smaller models get lean prompts and HIGHER turn budgets (up to 80 turns)
 * to compensate with tool-driven self-repair.
 */

export type ContextTier = "tiny" | "small" | "standard" | "large";

export interface TierBudgetConfig {
	readonly tier: ContextTier;
	readonly minTokens: number;
	readonly maxTokens: number;
	readonly maxSystemPromptChars: number;
	readonly maxInlineSkills: number;
	readonly maxInlineSkillsChars: number;
	readonly maxKnowledgeHits: number;
	readonly maxKnowledgeChars: number;
	readonly maxCodePages: number;
	readonly maxCodeChars: number;
	readonly turnBudget: number;
}

export const TIER_CONFIGS: Record<ContextTier, TierBudgetConfig> = {
	tiny: {
		tier: "tiny",
		minTokens: 0,
		maxTokens: 16_000,
		maxSystemPromptChars: 10_000,
		maxInlineSkills: 2,
		maxInlineSkillsChars: 800,
		maxKnowledgeHits: 2,
		maxKnowledgeChars: 600,
		maxCodePages: 2,
		maxCodeChars: 1_200,
		turnBudget: 80,
	},
	small: {
		tier: "small",
		minTokens: 16_000,
		maxTokens: 64_000,
		maxSystemPromptChars: 16_000,
		maxInlineSkills: 2,
		maxInlineSkillsChars: 1_200,
		maxKnowledgeHits: 3,
		maxKnowledgeChars: 800,
		maxCodePages: 3,
		maxCodeChars: 1_600,
		turnBudget: 64,
	},
	standard: {
		tier: "standard",
		minTokens: 64_000,
		maxTokens: 200_000,
		maxSystemPromptChars: 40_000,
		maxInlineSkills: 3,
		maxInlineSkillsChars: 2_000,
		maxKnowledgeHits: 4,
		maxKnowledgeChars: 1_200,
		maxCodePages: 4,
		maxCodeChars: 2_400,
		turnBudget: 48,
	},
	large: {
		tier: "large",
		minTokens: 200_000,
		maxTokens: Infinity,
		maxSystemPromptChars: 80_000,
		maxInlineSkills: 4,
		maxInlineSkillsChars: 3_000,
		maxKnowledgeHits: 5,
		maxKnowledgeChars: 1_800,
		maxCodePages: 6,
		maxCodeChars: 3_200,
		turnBudget: 40,
	},
};

/**
 * Detect context tier from context window size in tokens, or infer from model name.
 */
export function detectContextTier(contextWindowTokens?: number, modelName?: string): ContextTier {
	if (contextWindowTokens && contextWindowTokens > 0) {
		if (contextWindowTokens < 16_000) return "tiny";
		if (contextWindowTokens < 64_000) return "small";
		if (contextWindowTokens <= 200_000) return "standard";
		return "large";
	}

	if (modelName) {
		const lower = modelName.toLowerCase();
		if (lower.includes("mini") || lower.includes("flash") || lower.includes("8b") || lower.includes("7b")) {
			return "small";
		}
		if (lower.includes("claude-3-5-sonnet") || lower.includes("gpt-4o") || lower.includes("gemini")) {
			return "standard";
		}
	}

	return "standard";
}

export function getTierBudget(tier: ContextTier): TierBudgetConfig {
	return TIER_CONFIGS[tier] ?? TIER_CONFIGS.standard;
}

/**
 * Allocate dynamic prompt budget based on detected tier.
 */
export function allocateDynamicPromptBudget(options: {
	readonly contextWindowTokens?: number;
	readonly modelName?: string;
}): TierBudgetConfig {
	const tier = detectContextTier(options.contextWindowTokens, options.modelName);
	return getTierBudget(tier);
}
