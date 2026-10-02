/**
 * LLM-Assisted Diagnosis Engine for Harvest Laya Setup and Runtime Failures.
 *
 * Provides a second-tier recovery path when a failure falls outside the
 * deterministic 8-item self-healing table. Uses Harvest's configured
 * model/credentials to analyze the diagnostic bundle and propose a diagnosis,
 * candidate fix, and strict risk classification.
 *
 * Invariants:
 * 1. NEVER replaces or touches the deterministic calibration system or known-failure table.
 * 2. The LLM only PROPOSES; it never acts unilaterally.
 * 3. Low-risk, clearly reversible actions (e.g. missing dependency in venv) may be auto-applied
 *    and are ALWAYS logged plainly as "auto-applied: <what, why>".
 * 4. System-touching actions (ports, processes, data deletion, external files) or ambiguous
 *    actions REQUIRE explicit user confirmation and are NEVER auto-applied.
 * 5. If the LLM connection is unavailable, degrades gracefully to diagnostic dump and stop.
 * 6. Never automatically expands the deterministic known-signature table.
 */

import { type Api, completeSimple, type Model } from "@harvest/pi-ai";
import { prompt, logger } from "@harvest/pi-utils";
import type { ModelRegistry } from "../../config/model-registry";
import { resolveModelRoleValue, getModelMatchPreferences } from "../../config/model-resolver";
import type { Settings } from "../../config/settings";
import { discoverAuthStorage } from "../../sdk";
import promptTemplate from "../../prompts/system/laya-diagnosis-system.md" with { type: "text" };
import type { DiagnosticBundle, LayaSetupLogger } from "./laya-self-healing";
export type { DiagnosticBundle };

export type RiskLevel = "informational" | "low-risk-reversible" | "touches-system-state";

export type DiagnosisActionType = "install_dependency" | "retry_download" | "none" | "custom";

export interface LayaDiagnosisProposal {
	readonly diagnosis: string;
	readonly proposedFix: string;
	readonly actionType: DiagnosisActionType;
	readonly suggestedAction?: string | null;
	readonly modelRiskLevel: RiskLevel;
	readonly riskReason: string;
}

export interface EvaluatedDiagnosisProposal extends LayaDiagnosisProposal {
	readonly effectiveRiskLevel: RiskLevel;
	readonly isAutoAppliable: boolean;
	readonly overrideReason?: string;
	autoApplied?: boolean;
	executionError?: string;
}

export interface LayaLLMDiagnosisResult {
	readonly available: boolean;
	readonly proposal?: EvaluatedDiagnosisProposal;
	readonly rawResponse?: string;
	readonly error?: string;
}

const SYSTEM_PROMPT = prompt.render(promptTemplate);

/**
 * Strict safety evaluator that inspects a proposed action and enforces risk gating.
 * The model's own risk self-assessment is NEVER the sole gate for something destructive.
 */
export function evaluateRiskClassification(proposal: {
	actionType: string;
	suggestedAction?: string | null;
	modelRiskLevel?: RiskLevel;
	riskLevel?: RiskLevel;
	riskReason?: string;
}): {
	effectiveRiskLevel: RiskLevel;
	isAutoAppliable: boolean;
	overrideReason?: string;
} {
	const rawRisk = proposal.modelRiskLevel ?? proposal.riskLevel ?? "touches-system-state";
	const actionStr = (proposal.suggestedAction ?? "").trim().toLowerCase();

	// 1. Unconditionally reject any action that touches ports, kills processes, deletes files, or escalates privileges
	const destructivePatterns: Array<{ pattern: RegExp; reason: string }> = [
		{ pattern: /\b(kill|pkill|taskkill|stop-process|killall)\b/, reason: "Action attempts to terminate processes" },
		{
			pattern: /\b(port|bind|listen|netstat|fuser|lsof)\b/,
			reason: "Action attempts to alter or probe network ports",
		},
		{ pattern: /\b(rm|rmdir|del|remove-item|erase)\b/, reason: "Action attempts filesystem deletion" },
		{ pattern: /\b(sudo|runas|chmod|chown)\b/, reason: "Action requires elevated privileges" },
		{ pattern: /(\|\s*(bash|sh|powershell|iex|cmd))/, reason: "Action pipes content into a shell interpreter" },
		{ pattern: /\b(reboot|shutdown)\b/, reason: "Action attempts system restart" },
	];

	for (const { pattern, reason } of destructivePatterns) {
		if (pattern.test(actionStr)) {
			return {
				effectiveRiskLevel: "touches-system-state",
				isAutoAppliable: false,
				overrideReason: `${reason} (overridden to touches-system-state)`,
			};
		}
	}

	// 2. Low-risk reversible: strictly scoped Python package install in virtualenv
	if (proposal.actionType === "install_dependency") {
		// Verify the command looks like a package install without dangerous global flags
		const isSafePip =
			actionStr.startsWith("pip install ") ||
			actionStr.startsWith("python -m pip install ") ||
			/^[a-z0-9_.-]+$/.test(actionStr); // bare package name

		const hasDangerousFlags =
			actionStr.includes("--root") || actionStr.includes("--target") || actionStr.includes("--prefix");

		if (isSafePip && !hasDangerousFlags) {
			return {
				effectiveRiskLevel: "low-risk-reversible",
				isAutoAppliable: true,
			};
		}

		return {
			effectiveRiskLevel: "touches-system-state",
			isAutoAppliable: false,
			overrideReason: "Dependency installation includes ambiguous or non-standard options",
		};
	}

	// 3. Low-risk reversible: retrying a network operation once
	if (proposal.actionType === "retry_download") {
		return {
			effectiveRiskLevel: "low-risk-reversible",
			isAutoAppliable: true,
		};
	}

	// 4. Informational / Read-only
	if (proposal.actionType === "none" || !proposal.suggestedAction || proposal.suggestedAction.trim().length === 0) {
		return {
			effectiveRiskLevel: "informational",
			isAutoAppliable: false,
		};
	}

	// 5. Default fallback for ambiguous, custom, or model-asserted low-risk actions
	return {
		effectiveRiskLevel: "touches-system-state",
		isAutoAppliable: false,
		overrideReason:
			rawRisk !== "touches-system-state"
				? "Custom action defaulted to touches-system-state for user safety"
				: undefined,
	};
}

/**
 * Execute a low-risk, clearly reversible fix in the isolated Python environment.
 */
export async function executeAutoAppliedFix(
	proposal: EvaluatedDiagnosisProposal,
	context: {
		pythonPath?: string;
		setupLogger?: LayaSetupLogger;
		onProgress?: (message: string) => void;
	},
): Promise<{ success: boolean; error?: string }> {
	if (!proposal.isAutoAppliable) {
		return { success: false, error: "Action is not marked auto-appliable; explicit user confirmation required." };
	}

	if (proposal.actionType === "install_dependency") {
		const pythonPath = context.pythonPath;
		if (!pythonPath) {
			return { success: false, error: "Cannot auto-apply package installation: active Python path is unknown." };
		}

		let pkgName = proposal.suggestedAction?.trim() ?? "";
		if (pkgName.startsWith("pip install ")) pkgName = pkgName.slice("pip install ".length).trim();
		if (pkgName.startsWith("python -m pip install ")) pkgName = pkgName.slice("python -m pip install ".length).trim();

		context.onProgress?.(`Auto-applying dependency install: ${pkgName}...`);
		const logMsg = `auto-applied: pip install ${pkgName} in virtualenv: ${proposal.proposedFix}`;
		await context.setupLogger?.log(`[LLM_DIAGNOSIS] ${logMsg}`);

		try {
			const proc = Bun.spawn([pythonPath, "-m", "pip", "install", "--prefer-binary", pkgName], {
				stdout: "pipe",
				stderr: "pipe",
			});
			const exitCode = await proc.exited;
			if (exitCode !== 0) {
				const errText = await new Response(proc.stderr).text();
				return { success: false, error: `pip install exited with code ${exitCode}: ${errText}` };
			}
			proposal.autoApplied = true;
			return { success: true };
		} catch (err) {
			const error = err instanceof Error ? err.message : String(err);
			return { success: false, error };
		}
	}

	if (proposal.actionType === "retry_download") {
		const logMsg = `auto-applied: retry download: ${proposal.proposedFix}`;
		await context.setupLogger?.log(`[LLM_DIAGNOSIS] ${logMsg}`);
		proposal.autoApplied = true;
		return { success: true };
	}

	return { success: false, error: `Unsupported auto-apply action type: ${proposal.actionType}` };
}

/**
 * Query Harvest's configured main LLM connection with the diagnostic bundle.
 */
export async function queryLlmDiagnosis(
	bundle: DiagnosticBundle,
	options: {
		settings?: Settings;
		modelRegistry?: ModelRegistry;
		model?: Model<Api>;
		timeoutMs?: number;
		completeSimpleFn?: typeof completeSimple;
	} = {},
): Promise<LayaDiagnosisProposal | null> {
	try {
		const settings = options.settings;
		let registry = options.modelRegistry;

		if (!registry) {
			try {
				const authStorage = await discoverAuthStorage();
				const { ModelRegistry } = await import("../../config/model-registry");
				registry = new ModelRegistry(authStorage, undefined, { settings });
			} catch {
				return null;
			}
		}

		let candidateModel = options.model;
		if (!candidateModel) {
			const availableModels = registry.getAvailable();
			if (availableModels.length === 0) return null;

			if (settings) {
				const matchPreferences = getModelMatchPreferences(settings);
				const defaultRole = settings.getModelRole("default");
				if (defaultRole) {
					const resolved = resolveModelRoleValue(defaultRole, availableModels, { settings, matchPreferences });
					candidateModel = resolved.model;
				}
				if (!candidateModel) {
					const smolRole = settings.getModelRole("smol");
					if (smolRole) {
						const resolved = resolveModelRoleValue(smolRole, availableModels, { settings, matchPreferences });
						candidateModel = resolved.model;
					}
				}
			}

			if (!candidateModel) {
				candidateModel = availableModels[0];
			}
		}

		if (!candidateModel) return null;

		const apiKey = await registry.getApiKey(candidateModel);
		if (!apiKey) return null;

		const userMessage = JSON.stringify(
			{
				diagnosticBundle: {
					timestamp: bundle.timestamp,
					error: bundle.error,
					platform: bundle.platform,
					arch: bundle.arch,
					osRelease: bundle.osRelease,
					bunVersion: bundle.bunVersion,
					pythonPath: bundle.pythonPath,
					pythonVersion: bundle.pythonVersion,
					torchVersion: bundle.torchVersion,
					torchCuda: bundle.torchCuda,
					hostHasNvidia: bundle.hostHasNvidia,
					diskSpaceMbAvailable: bundle.diskSpaceMbAvailable,
					activePort: bundle.activePort,
					recentLogTail: bundle.recentLogTail.slice(-25),
				},
			},
			null,
			2,
		);

		const completeFn = options.completeSimpleFn ?? completeSimple;
		const response = await completeFn(
			candidateModel,
			{
				systemPrompt: [SYSTEM_PROMPT],
				messages: [{ role: "user", content: userMessage, timestamp: Date.now() }],
			},
			{
				apiKey: registry.resolver(candidateModel),
				maxTokens: 1024,
			},
		);

		if (response.stopReason === "error") {
			logger.debug("laya-llm-diagnosis: completeSimple returned error", { error: response.errorMessage });
			return null;
		}

		let text = "";
		for (const part of response.content) {
			if (part.type === "text") text += part.text;
		}
		text = text.trim();
		if (!text) return null;

		// Strip markdown code block if present
		const jsonMatch = text.match(/```(?:json)?\s*([\s\S]*?)\s*```/);
		const rawJson = jsonMatch ? jsonMatch[1].trim() : text;

		const parsed = JSON.parse(rawJson);
		if (!parsed || typeof parsed !== "object" || !parsed.diagnosis || !parsed.proposedFix) {
			return null;
		}

		const validRisk: RiskLevel =
			parsed.riskLevel === "low-risk-reversible" ||
			parsed.riskLevel === "informational" ||
			parsed.riskLevel === "touches-system-state"
				? parsed.riskLevel
				: "touches-system-state";

		const validActionType: DiagnosisActionType =
			parsed.actionType === "install_dependency" ||
			parsed.actionType === "retry_download" ||
			parsed.actionType === "none"
				? parsed.actionType
				: "custom";

		return {
			diagnosis: String(parsed.diagnosis),
			proposedFix: String(parsed.proposedFix),
			actionType: validActionType,
			suggestedAction: parsed.suggestedAction ? String(parsed.suggestedAction) : null,
			modelRiskLevel: validRisk,
			riskReason: parsed.riskReason ? String(parsed.riskReason) : "No risk justification provided",
		};
	} catch (err) {
		logger.debug("laya-llm-diagnosis: LLM query failed", { error: err instanceof Error ? err.message : String(err) });
		return null;
	}
}

/**
 * Coordinate the second-tier LLM-assisted diagnosis for unrecognized setup failures.
 */
export async function runLlmAssistedDiagnosis(
	bundle: DiagnosticBundle,
	options: {
		settings?: Settings;
		setupLogger?: LayaSetupLogger;
		modelRegistry?: ModelRegistry;
		pythonPath?: string;
		onProgress?: (message: string) => void;
		onUserConfirmation?: (proposal: EvaluatedDiagnosisProposal) => Promise<boolean>;
		completeSimpleFn?: typeof completeSimple;
		autoApplyLowRisk?: boolean;
	} = {},
): Promise<LayaLLMDiagnosisResult> {
	await options.setupLogger?.log(
		`[LLM_DIAGNOSIS] Unrecognized failure triggered second-tier LLM diagnosis with diagnostic bundle`,
		{
			error: bundle.error,
			platform: bundle.platform,
			python: bundle.pythonPath,
		},
	);

	options.onProgress?.("Analyzing failure with Harvest LLM connection (second-tier diagnosis)...");

	const proposal = await queryLlmDiagnosis(bundle, {
		settings: options.settings,
		modelRegistry: options.modelRegistry,
		completeSimpleFn: options.completeSimpleFn,
	});

	if (!proposal) {
		await options.setupLogger?.log(
			`[LLM_DIAGNOSIS] LLM connection unavailable or query failed; falling back cleanly to diagnostic dump`,
		);
		return { available: false };
	}

	const riskEval = evaluateRiskClassification(proposal);
	const evaluated: EvaluatedDiagnosisProposal = {
		...proposal,
		effectiveRiskLevel: riskEval.effectiveRiskLevel,
		isAutoAppliable: riskEval.isAutoAppliable,
		overrideReason: riskEval.overrideReason,
	};

	await options.setupLogger?.log(`[LLM_DIAGNOSIS] Received proposal from model`, {
		diagnosis: evaluated.diagnosis,
		proposedFix: evaluated.proposedFix,
		modelRiskLevel: evaluated.modelRiskLevel,
		effectiveRiskLevel: evaluated.effectiveRiskLevel,
		isAutoAppliable: evaluated.isAutoAppliable,
		suggestedAction: evaluated.suggestedAction,
		overrideReason: evaluated.overrideReason,
	});

	// Low-risk, clearly reversible auto-application
	if (
		evaluated.effectiveRiskLevel === "low-risk-reversible" &&
		evaluated.isAutoAppliable &&
		options.autoApplyLowRisk !== false
	) {
		const applyRes = await executeAutoAppliedFix(evaluated, {
			pythonPath: options.pythonPath ?? bundle.pythonPath,
			setupLogger: options.setupLogger,
			onProgress: options.onProgress,
		});
		if (!applyRes.success) {
			evaluated.executionError = applyRes.error;
			await options.setupLogger?.log(`[LLM_DIAGNOSIS] Auto-application failed: ${applyRes.error}`);
		}
	} else if (evaluated.effectiveRiskLevel === "touches-system-state") {
		// Touches system state: MUST prompt for explicit confirmation or skip
		if (options.onUserConfirmation) {
			const userApproved = await options.onUserConfirmation(evaluated);
			if (userApproved) {
				await options.setupLogger?.log(
					`[LLM_DIAGNOSIS] User explicitly confirmed candidate fix: ${evaluated.suggestedAction}`,
				);
			} else {
				await options.setupLogger?.log(`[LLM_DIAGNOSIS] User declined candidate fix: ${evaluated.suggestedAction}`);
			}
		} else {
			await options.setupLogger?.log(
				`[LLM_DIAGNOSIS] Fix requires explicit confirmation (${evaluated.effectiveRiskLevel}); no confirmation handler available in this mode.`,
			);
		}
	}

	return {
		available: true,
		proposal: evaluated,
	};
}
