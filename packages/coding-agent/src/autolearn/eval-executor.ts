/**
 * Production evaluation executors for managed skill and preset revisions.
 *
 * Both executors run the candidate through real restricted task execution:
 * an isolated child session (restrictToolNames, minimal tools, no UI, no
 * auto-learn) executes the supplied task with the candidate pinned, the
 * observed output is graded against the explicit expected outcome, and the
 * verification basis plus run/session IDs are returned for the revision
 * record. Failed or unverifiable outcomes never promote.
 *
 * Credentials come only from the parent session: its direct resolver first,
 * then its model-registry resolver (stored/OAuth + env cascade). The child
 * borrows the parent registry without taking ownership, so child disposal
 * never closes parent auth state. Key material is never logged.
 */
import { completeSimple, resolveApiKeyOnce, retryTransientCompletion, type ApiKey, type Model } from "@harvest/pi-ai";
import { modelsAreEqual } from "@harvest/pi-catalog/models";
import { logger, prompt } from "@harvest/pi-utils";
import { isAuthenticated, kNoAuth } from "../config/model-provider-discovery";
import { resolveModelOverride, resolveModelOverrideWithAuthFallback } from "../config/model-resolver";
import { MODEL_ROLE_ALIAS_PREFIX } from "../config/model-roles";
import { parseAgent } from "../task/agents";
import { setPresetEvalRunner, type PresetEvalOutcome } from "../task/agents";
import type { ToolSession } from "../tools/index";
import evalCandidateSkillPrompt from "../prompts/autolearn/eval-candidate-skill.md" with { type: "text" };
import evalGradePrompt from "../prompts/autolearn/eval-grade.md" with { type: "text" };
import { setSkillEvalRunner, type SkillEvalOutcome } from "./managed-skills";

export interface ManagedEvalInput {
	kind: "skill" | "preset";
	name: string;
	revId: string;
	content: string;
	task: string;
	expectedOutcome: string;
	parent: ToolSession;
	agentDir?: string;
	signal?: AbortSignal;
}

/** How the evaluation child model was resolved (recorded on the revision for audit). */
export type EvalModelBasis = "exact" | "role" | "pattern" | "auth-fallback" | "parent-active";

/** Default model pattern for skill evaluations (skills carry no model field of their own). */
export const EVAL_DEFAULT_MODEL_PATTERN = "@task";

/** Normalize a spec model field (string | string[]) to candidate patterns. */
function normalizeEvalModelPatterns(requested: string | string[] | undefined): string[] {
	const list =
		requested === undefined ? [EVAL_DEFAULT_MODEL_PATTERN] : Array.isArray(requested) ? requested : [requested];
	const patterns = list.map(entry => entry.trim()).filter(entry => entry.length > 0);
	return patterns.length > 0 ? patterns : [EVAL_DEFAULT_MODEL_PATTERN];
}

function classifyEvalModelBasis(
	winningPattern: string | undefined,
	model: Model,
	authFallbackUsed: boolean,
): EvalModelBasis {
	if (authFallbackUsed) return "auth-fallback";
	const pattern = winningPattern?.trim() ?? "";
	if (pattern === `${model.provider}/${model.id}`) return "exact";
	if (pattern.startsWith(MODEL_ROLE_ALIAS_PREFIX)) return "role";
	return "pattern";
}

/**
 * Resolve the evaluation child model through the shared task/preset
 * resolver (exact provider/id, settings roles, patterns, auth-aware fallback
 * to the parent active model) and record how it resolved. No
 * provider/model-identity conditionals: policy stays in catalog/KDL and
 * settings roles. Throws when nothing resolves — the revision then stays
 * unevaluated instead of running on an arbitrary model.
 */
export async function resolveEvalChildModel(
	parent: ToolSession,
	requested: string | string[] | undefined,
): Promise<{ model: Model; basis: EvalModelBasis }> {
	const patterns = normalizeEvalModelPatterns(requested);
	const registry = parent.modelRegistry;
	if (!registry) {
		const active = parent.getActiveModel?.();
		if (!active) throw new Error("No evaluation model resolves and no active model is available to run it.");
		return { model: active, basis: "parent-active" };
	}
	const sessionId = parent.getSessionId?.() ?? undefined;
	const resolved = await resolveModelOverrideWithAuthFallback(
		patterns,
		parent.getActiveModelString?.(),
		registry,
		parent.settings,
		sessionId,
	);
	if (!resolved.model) {
		throw new Error(
			`No evaluation model resolves from ${patterns.join(", ")}; the revision stays unevaluated — unevaluated content is never treated as passed.`,
		);
	}
	if (resolved.authFallbackUsed) return { model: resolved.model, basis: "auth-fallback" };
	let winning: string | undefined;
	for (const pattern of patterns) {
		const candidate = resolveModelOverride([pattern], registry, parent.settings).model;
		if (candidate && modelsAreEqual(candidate, resolved.model)) {
			winning = pattern;
			break;
		}
	}
	return { model: resolved.model, basis: classifyEvalModelBasis(winning, resolved.model, false) };
}

export interface EvalCredential {
	/**
	 * Resolved bearer for the grading call; undefined when keyless or
	 * unavailable. Always a string: the streaming driver only honors string
	 * bearers, so resolvers are resolved (not forwarded) here.
	 */
	key: string | undefined;
	/** True when no credential exists: the run is unverifiable without calling the provider. */
	unavailable: boolean;
}

/**
 * Resolve the grading credential through the centralized contract, asking
 * only the parent session: its direct resolver first (override precedence),
 * then the parent model-registry resolver (stored/OAuth + env cascade).
 * Keyless-by-design providers resolve as keyless (proceed without a bearer);
 * anything else without a bearer is unavailable. Never logs key material.
 */
export async function resolveEvalCredential(
	parent: ToolSession,
	model: Model,
	sessionId: string | undefined,
	signal: AbortSignal | undefined,
): Promise<EvalCredential> {
	let direct: ApiKey | undefined;
	try {
		direct = await parent.getApiKey?.(model);
	} catch {
		direct = undefined;
	}
	const initial = await resolveApiKeyOnce(direct, signal);
	if (isAuthenticated(initial)) return { key: initial, unavailable: false };
	let registryKey: string | undefined;
	try {
		registryKey = await parent.modelRegistry?.getApiKey(model, sessionId, { signal });
	} catch {
		registryKey = undefined;
	}
	const resolved = await resolveApiKeyOnce(registryKey, signal);
	if (isAuthenticated(resolved)) return { key: resolved, unavailable: false };
	if (direct === kNoAuth || registryKey === kNoAuth) return { key: undefined, unavailable: false };
	return { key: undefined, unavailable: true };
}

const OUTPUT_BASIS_CHARS = 1200;

function lastAssistantText(messages: readonly unknown[]): string {
	for (let i = messages.length - 1; i >= 0; i--) {
		const message = messages[i] as { role?: unknown; content?: unknown };
		if (message?.role !== "assistant" || !Array.isArray(message.content)) continue;
		const text = message.content
			.map(block =>
				block && typeof block === "object" && "type" in block && block.type === "text" && "text" in block
					? String(block.text)
					: "",
			)
			.join("\n")
			.trim();
		if (text) return text;
	}
	return "";
}

async function runManagedEval(input: ManagedEvalInput): Promise<SkillEvalOutcome & PresetEvalOutcome> {
	const parent = input.parent;
	const signal = input.signal;
	const task = input.task.trim();
	const expectedOutcome = input.expectedOutcome.trim();
	if (!task) throw new Error("Evaluation needs an explicit task.");
	if (!expectedOutcome) throw new Error("Evaluation needs an explicit expected outcome.");
	if (signal?.aborted) throw new Error("Evaluation aborted before execution.");

	// One path for both kinds (no model-identity conditionals): the prompt
	// assembly differs, but model selection always flows through the shared
	// resolver below.
	let systemPrompt: string[];
	let toolNames: string[];
	let modelPatterns: string | string[] | undefined;
	if (input.kind === "skill") {
		systemPrompt = [prompt.render(evalCandidateSkillPrompt, { name: input.name, content: input.content })];
		toolNames = ["read", "glob", "grep"];
		modelPatterns = undefined;
	} else {
		const spec = parseAgent(`managed:${input.name}`, input.content, "user", "warn");
		systemPrompt = [spec.systemPrompt];
		toolNames = spec.tools ?? ["read", "glob", "grep"];
		modelPatterns = spec.model;
	}
	const { model: childModel, basis: modelBasis } = await resolveEvalChildModel(parent, modelPatterns);
	const modelSelector = `${childModel.provider}/${childModel.id}`;
	if (signal?.aborted) throw new Error("Evaluation aborted before execution.");

	const { createAgentSession } = await import("../sdk");
	const settings = await parent.settings.cloneForCwd(parent.cwd);
	// Runtime-only override: neither success nor failure may persist into
	// user/project configuration (contrast Settings.set, which saves).
	settings.override("autolearn.enabled", false);
	if (signal?.aborted) throw new Error("Evaluation aborted before execution.");

	const { session } = await createAgentSession({
		cwd: parent.cwd,
		settings,
		model: childModel,
		systemPrompt,
		hasUI: false,
		enableLsp: false,
		enableMCP: false,
		disableExtensionDiscovery: true,
		toolNames,
		restrictToolNames: true,
		customTools: [],
		skills: [],
		contextFiles: [],
		promptTemplates: [],
		slashCommands: [],
		taskDepth: (parent.taskDepth ?? 0) + 1,
		// Borrow the parent registry (and direct resolver) without ownership:
		// child disposal never closes parent auth state (SDK ownsAuthStorage).
		modelRegistry: parent.modelRegistry,
		getApiKey: parent.getApiKey,
	});
	const runId = session.sessionManager.getSessionId();
	const finish = (outcome: SkillEvalOutcome & PresetEvalOutcome): SkillEvalOutcome & PresetEvalOutcome => ({
		...outcome,
		model: modelSelector,
		modelBasis,
	});
	let onAbort: (() => void) | undefined;
	try {
		// session.prompt takes no signal: execution-phase cancellation arrives
		// by disposing the child below. The listener registers BEFORE the run
		// starts; the race rejects promptly on abort while the child is torn
		// down, and the listener is always removed before returning.
		const promptWork = session.prompt(task);
		if (signal && !signal.aborted) {
			const abortWait = new Promise<never>((_, reject) => {
				onAbort = () => reject(new Error("Evaluation aborted during execution."));
				signal.addEventListener("abort", onAbort, { once: true });
			});
			try {
				await Promise.race([promptWork, abortWait]);
			} finally {
				if (onAbort) signal.removeEventListener("abort", onAbort);
				onAbort = undefined;
			}
		} else {
			if (signal?.aborted) throw new Error("Evaluation aborted during execution.");
			await promptWork;
		}
		if (signal?.aborted) throw new Error("Evaluation aborted during execution.");
		const output = lastAssistantText(session.state.messages);
		if (!output) {
			return finish({
				passed: false,
				summary: `No observable output from evaluation run ${runId}; nothing to verify against the expected outcome.`,
				runId,
				sessionId: runId,
			});
		}
		const credential = await resolveEvalCredential(parent, childModel, runId, signal);
		if (credential.unavailable) {
			return finish({
				passed: false,
				summary: `Run ${runId} produced output but no API key is available to grade it; outcome unverifiable.`,
				runId,
				sessionId: runId,
			});
		}
		if (signal?.aborted) throw new Error("Evaluation aborted before grading.");
		const verdict = await retryTransientCompletion(
			() =>
				completeSimple(
					childModel,
					{
						systemPrompt: ["You grade evaluation runs."],
						messages: [
							{
								role: "user",
								content: prompt.render(evalGradePrompt, {
									task,
									expectedOutcome,
									output: output.slice(0, OUTPUT_BASIS_CHARS),
								}),
								timestamp: Date.now(),
							},
						],
					},
					{ apiKey: credential.key, sessionId: runId, maxTokens: 256, disableReasoning: true, signal },
				),
			{ signal },
		);
		if (signal?.aborted) throw new Error("Evaluation aborted during grading.");
		if (verdict.stopReason === "error") {
			return finish({
				passed: false,
				summary: `Run ${runId} produced output but grading failed (${verdict.errorMessage ?? "unknown error"}); outcome unverifiable.`,
				runId,
				sessionId: runId,
			});
		}
		const lines = verdict.content
			.filter((part): part is { type: "text"; text: string } => part.type === "text")
			.map(part => part.text)
			.join("\n")
			.trim()
			.split("\n");
		const passed = lines[0]?.trim().toUpperCase() === "PASS";
		const basis = lines.slice(1).join(" ").trim() || "No verification basis returned by the grader.";
		return finish({
			passed,
			summary: `Run ${runId}: ${basis} Basis: ${output.slice(0, OUTPUT_BASIS_CHARS)}`,
			runId,
			sessionId: runId,
		});
	} finally {
		if (onAbort && signal) signal.removeEventListener("abort", onAbort);
		await session.dispose().catch((error: unknown) => logger.warn("Failed to dispose evaluation run", { error }));
	}
}

/**
 * Install the production evaluation executors (real restricted task
 * execution). Safe to call repeatedly: re-installing simply re-points the
 * module runners at the production executors (so a test or host that cleared
 * the slots gets the real path back). Tests may still override via the
 * module setters. Called from SDK session creation.
 */
export function installManagedEvalExecutors(): void {
	setSkillEvalRunner(async input => {
		if (!input.parent) throw new Error("Skill evaluation needs a parent session context.");
		return runManagedEval({ ...input, kind: "skill", parent: input.parent });
	});
	setPresetEvalRunner(async input => {
		if (!input.parent) throw new Error("Preset evaluation needs a parent session context.");
		return runManagedEval({ ...input, kind: "preset", parent: input.parent });
	});
}
