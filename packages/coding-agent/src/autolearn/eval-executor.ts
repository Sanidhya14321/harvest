/**
 * Production evaluation executors for managed skill and preset revisions.
 *
 * Both executors run the candidate through real restricted task execution:
 * an isolated child session (restrictToolNames, minimal tools, no UI, no
 * auto-learn) executes the supplied task with the candidate pinned, the
 * observed output is graded against the explicit expected outcome, and the
 * verification basis plus run/session IDs are returned for the revision
 * record. Failed or unverifiable outcomes never promote.
 */
import { completeSimple, resolveApiKeyOnce, retryTransientCompletion, type Model } from "@harvest/pi-ai";
import { logger, prompt } from "@harvest/pi-utils";
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

function resolveChildModel(parent: ToolSession, requested?: string[]): Model | undefined {
	const fallback = parent.getActiveModel?.();
	if (!requested || requested.length === 0) return fallback;
	const first = requested[0]!;
	const slash = first.indexOf("/");
	if (slash > 0) {
		const found = parent.modelRegistry?.find(first.slice(0, slash), first.slice(slash + 1));
		if (found) return found;
	}
	return fallback;
}

async function runManagedEval(input: ManagedEvalInput): Promise<SkillEvalOutcome & PresetEvalOutcome> {
	const parent = input.parent;
	const task = input.task.trim();
	const expectedOutcome = input.expectedOutcome.trim();
	if (!task) throw new Error("Evaluation needs an explicit task.");
	if (!expectedOutcome) throw new Error("Evaluation needs an explicit expected outcome.");
	const { createAgentSession } = await import("../sdk");
	const settings = await parent.settings.cloneForCwd(parent.cwd);
	// Runtime-only override: neither success nor failure may persist into
	// user/project configuration (contrast Settings.set, which saves).
	settings.override("autolearn.enabled", false);
	if (input.signal?.aborted) throw new Error("Evaluation aborted before execution.");

	let systemPrompt: string[];
	let toolNames: string[];
	let childModel: Model | undefined;
	if (input.kind === "skill") {
		systemPrompt = [prompt.render(evalCandidateSkillPrompt, { name: input.name, content: input.content })];
		toolNames = ["read", "glob", "grep"];
		childModel = parent.getActiveModel?.();
	} else {
		const spec = parseAgent(`managed:${input.name}`, input.content, "user", "warn");
		systemPrompt = [spec.systemPrompt];
		toolNames = spec.tools ?? ["read", "glob", "grep"];
		childModel = resolveChildModel(parent, spec.model);
	}
	if (!childModel) throw new Error("No active model is available to run the evaluation.");
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
	});
	const runId = session.sessionManager.getSessionId();
	try {
		await session.prompt(task);
		if (input.signal?.aborted) throw new Error("Evaluation aborted during execution.");
		const output = lastAssistantText(session.state.messages);
		if (!output) {
			return {
				passed: false,
				summary: `No observable output from evaluation run ${runId}; nothing to verify against the expected outcome.`,
				runId,
				sessionId: runId,
			};
		}
		const rawKey = await parent.getApiKey?.(childModel);
		const apiKey = await resolveApiKeyOnce(rawKey, input.signal);
		if (!apiKey) {
			return {
				passed: false,
				summary: `Run ${runId} produced output but no API key is available to grade it; outcome unverifiable.`,
				runId,
				sessionId: runId,
			};
		}
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
					{ apiKey, sessionId: runId, maxTokens: 256, disableReasoning: true, signal: input.signal },
				),
			{ signal: input.signal },
		);
		if (verdict.stopReason === "error") {
			return {
				passed: false,
				summary: `Run ${runId} produced output but grading failed (${verdict.errorMessage ?? "unknown error"}); outcome unverifiable.`,
				runId,
				sessionId: runId,
			};
		}
		const lines = verdict.content
			.filter((part): part is { type: "text"; text: string } => part.type === "text")
			.map(part => part.text)
			.join("\n")
			.trim()
			.split("\n");
		const passed = lines[0]?.trim().toUpperCase() === "PASS";
		const basis = lines.slice(1).join(" ").trim() || "No verification basis returned by the grader.";
		return {
			passed,
			summary: `Run ${runId}: ${basis} Basis: ${output.slice(0, OUTPUT_BASIS_CHARS)}`,
			runId,
			sessionId: runId,
		};
	} finally {
		await session.dispose().catch((error: unknown) => logger.warn("Failed to dispose evaluation run", { error }));
	}
}

let installed = false;

/**
 * Install the production evaluation executors (real restricted task
 * execution). Idempotent; tests may still override via the module setters.
 * Called once from SDK session creation.
 */
export function installManagedEvalExecutors(): void {
	if (installed) return;
	installed = true;
	setSkillEvalRunner(async input => {
		if (!input.parent) throw new Error("Skill evaluation needs a parent session context.");
		return runManagedEval({ ...input, kind: "skill", parent: input.parent });
	});
	setPresetEvalRunner(async input => {
		if (!input.parent) throw new Error("Preset evaluation needs a parent session context.");
		return runManagedEval({ ...input, kind: "preset", parent: input.parent });
	});
}
