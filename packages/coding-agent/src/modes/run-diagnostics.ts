import type { AssistantMessage } from "@harvest/pi-ai";
import type { AgentSessionEvent } from "../session/agent-session-events";

/** Where the visible run currently stands. */
export type RunStage =
	/** No turn in flight. */
	| "idle"
	/** A turn streams model output. */
	| "streaming"
	/** A tool implementation runs. */
	| "tool"
	/** A retry delay or attempt is outstanding. */
	| "retry"
	/** Context compaction runs. */
	| "compaction"
	/** The loop settled mid-saga and waits on background delivery to re-wake it. */
	| "awaitingDelivery";

/** Point-in-time diagnostic for one session's run. */
export interface RunDiagnosticSnapshot {
	stage: RunStage;
	/** Tool running while `stage === "tool"`. */
	activeTool: string | undefined;
	/** Retry attempt, compaction reason, or tool intent — whichever is current. */
	detail: string | undefined;
	/** Wall time the current saga started; absent while idle. */
	runStartedAt: number | undefined;
	/** Milliseconds since `runStartedAt`; 0 while idle. */
	elapsedMs: number;
	/** Prompt dispatch to first streamed token this saga; absent when no token streamed yet. */
	firstTokenMs: number | undefined;
	/** Completed assistant turns this saga; helps separate turn-limit stalls from hangs. */
	turnCount: number;
	/** Most recent finished tool and its wall duration. */
	lastTool: { name: string; durationMs: number } | undefined;
	/** Terminal failure of the last settled saga; survives until the next run starts. */
	failure: string | undefined;
	failureAt: number | undefined;
	/** Most recent meaningful event (message deltas excluded as streaming noise). */
	lastEvent: string | undefined;
	lastEventAt: number | undefined;
}

/** Terminal failure text kept verbatim, capped so one bad turn cannot bloat the record. */
const MAX_FAILURE_CHARS = 300;

/** Precise durations for diagnostics; sub-second values keep millisecond resolution. */
function formatPreciseDuration(ms: number): string {
	const value = Math.max(0, Math.round(ms));
	if (value < 1000) return `${value}ms`;
	return `${Math.round(value / 1000)}s`;
}

/**
 * Render a snapshot as plain operator-facing lines. Only populated fields
 * print, so an idle session reads as one line and an active or failed run
 * shows exactly what distinguishes model latency, tool execution, and stalls.
 */
export function formatRunDiagnostic(snapshot: RunDiagnosticSnapshot): string {
	const lines = [
		`Stage: ${snapshot.stage}${snapshot.elapsedMs > 0 ? ` (elapsed ${formatPreciseDuration(snapshot.elapsedMs)})` : ""}`,
	];
	if (snapshot.activeTool) lines.push(`Active tool: ${snapshot.activeTool}`);
	if (snapshot.detail) lines.push(`Detail: ${snapshot.detail}`);
	if (snapshot.firstTokenMs !== undefined) {
		lines.push(`First token: ${formatPreciseDuration(snapshot.firstTokenMs)} after dispatch`);
	}
	if (snapshot.turnCount > 0) lines.push(`Turns: ${snapshot.turnCount}`);
	if (snapshot.lastTool) {
		lines.push(`Last tool: ${snapshot.lastTool.name} (${formatPreciseDuration(snapshot.lastTool.durationMs)})`);
	}
	if (snapshot.lastEvent) lines.push(`Last event: ${snapshot.lastEvent}`);
	if (snapshot.failure) lines.push(`Failure: ${snapshot.failure}`);
	return lines.join("\n");
}

/**
 * Passive per-session observer of the agent event stream. Answers "why did
 * work stop or pause": current stage, active tool, last meaningful event,
 * run elapsed time, prompt-to-first-token latency, per-turn count, finished
 * tool durations, and the preserved terminal failure reason.
 *
 * Mirrors the two guards `EventController.#handleAgentEnd` applies before
 * tearing a turn down: an `agent_end` arriving while the runtime still
 * reports active is a superseded turn (ignored for stage purposes), and an
 * `agent_end` tagged `isTerminal: false` is a scheduling pause, not a
 * settle. Callers pass the owning runtime's liveness oracle — the same
 * `isStreaming` read the event controller uses — so background sessions can
 * each carry their own without sharing mutable UI state.
 *
 * Failure sources are narrow and explicit: error notices, failed retry
 * sagas, failed compactions, errored tool results, and terminal assistant
 * messages the system already classified with `stopReason === "error"`.
 * Message text is never sniffed beyond that classified signal.
 */
export class RunDiagnosticsTracker {
	readonly #runs = new Map<string, MutableRunDiagnostic>();
	readonly #retryPending = new Set<string>();

	/**
	 * Observe one event for `sessionId`. `isActive` must report whether that
	 * session's runtime is still streaming at dispatch time.
	 */
	handleEvent(sessionId: string, event: AgentSessionEvent, isActive: () => boolean, now = Date.now()): void {
		const run = this.#run(sessionId);
		switch (event.type) {
			case "agent_start": {
				if (!run.runActive) {
					run.runActive = true;
					run.stage = this.#retryPending.has(sessionId) ? "retry" : "streaming";
					run.runStartedAt = now;
					run.activeTool = undefined;
					run.detail = run.stage === "retry" ? run.detail : undefined;
					run.failure = undefined;
					run.failureAt = undefined;
					run.firstTokenMs = undefined;
					run.firstTokenSince = undefined;
					run.turnCount = 0;
					run.lastTool = undefined;
				} else if (run.stage === "awaitingDelivery") {
					run.stage = this.#retryPending.has(sessionId) ? "retry" : "streaming";
				}
				this.#touch(run, event.type, now);
				return;
			}
			case "agent_end": {
				if (isActive()) {
					this.#touch(run, event.type, now);
					return;
				}
				if (this.#retryPending.has(sessionId)) {
					this.#touch(run, event.type, now);
					return;
				}
				if (event.isTerminal === false) {
					run.stage = "awaitingDelivery";
					this.#touch(run, event.type, now);
					return;
				}
				run.runActive = false;
				run.stage = "idle";
				run.activeTool = undefined;
				run.detail = undefined;
				run.runStartedAt = undefined;
				const failure = run.pendingFailure ?? errorTextFromMessages(event.messages);
				run.pendingFailure = undefined;
				if (failure) {
					run.failure = failure;
					run.failureAt = now;
				}
				this.#touch(run, event.type, now);
				return;
			}
			case "tool_execution_start": {
				run.stage = "tool";
				run.activeTool = event.toolName;
				run.detail = event.intent;
				run.toolStartedAt = now;
				this.#touch(run, event.type, now);
				return;
			}
			case "tool_execution_end": {
				if (run.stage === "tool") run.stage = run.runActive ? "streaming" : "idle";
				run.activeTool = undefined;
				run.detail = undefined;
				if (run.toolStartedAt !== undefined) {
					run.lastTool = { name: event.toolName, durationMs: Math.max(0, now - run.toolStartedAt) };
					run.toolStartedAt = undefined;
				}
				if (event.isError) {
					run.failure = toolErrorText(event.toolName, event.result);
					run.failureAt = now;
				}
				this.#touch(run, event.type, now);
				return;
			}
			case "auto_retry_start": {
				this.#retryPending.add(sessionId);
				run.stage = "retry";
				run.detail = `Attempt ${event.attempt}/${event.maxAttempts}: ${event.errorMessage}`;
				this.#touch(run, event.type, now);
				return;
			}
			case "auto_retry_end": {
				this.#retryPending.delete(sessionId);
				if (event.success) {
					run.pendingFailure = undefined;
					run.stage = run.runActive ? "streaming" : "idle";
					run.detail = undefined;
				} else {
					run.pendingFailure = truncateFailure(event.finalError ?? "Retry attempts exhausted");
					run.detail = `Attempt ${event.attempt} failed`;
				}
				this.#touch(run, event.type, now);
				return;
			}
			case "auto_compaction_start": {
				run.stage = "compaction";
				run.detail = `${event.reason} → ${event.action}`;
				this.#touch(run, event.type, now);
				return;
			}
			case "auto_compaction_end": {
				if (run.stage === "compaction") run.stage = run.runActive ? "streaming" : "idle";
				if (event.errorMessage && !event.skipped) {
					run.failure = truncateFailure(event.errorMessage);
					run.failureAt = now;
				}
				run.detail = event.aborted ? "Compaction aborted" : undefined;
				this.#touch(run, event.type, now);
				return;
			}
			case "notice": {
				if (event.level === "error") {
					run.failure = truncateFailure(event.message);
					run.failureAt = now;
				}
				this.#touch(run, event.type, now);
				return;
			}
			case "turn_start":
			case "retry_fallback_applied":
			case "retry_fallback_succeeded":
			case "model_changed": {
				this.#touch(run, event.type, now);
				return;
			}
			case "turn_end": {
				run.turnCount++;
				this.#touch(run, event.type, now);
				return;
			}
			case "message_start": {
				if (run.runActive && run.firstTokenMs === undefined) run.firstTokenSince = now;
				return;
			}
			case "message_update": {
				if (run.runActive && run.firstTokenMs === undefined && run.firstTokenSince !== undefined) {
					run.firstTokenMs = Math.max(0, now - run.firstTokenSince);
					run.firstTokenSince = undefined;
				}
				return;
			}
			case "message_end": {
				run.firstTokenSince = undefined;
				return;
			}
			default: {
				return;
			}
		}
	}

	/** Diagnostic snapshot for `sessionId`; unknown sessions read as idle. */
	snapshot(sessionId: string, now = Date.now()): RunDiagnosticSnapshot {
		const run = this.#runs.get(sessionId);
		if (!run) {
			return {
				stage: "idle",
				activeTool: undefined,
				detail: undefined,
				runStartedAt: undefined,
				elapsedMs: 0,
				firstTokenMs: undefined,
				turnCount: 0,
				lastTool: undefined,
				failure: undefined,
				failureAt: undefined,
				lastEvent: undefined,
				lastEventAt: undefined,
			};
		}
		return {
			stage: run.stage,
			activeTool: run.activeTool,
			detail: run.detail,
			runStartedAt: run.runStartedAt,
			elapsedMs: run.runStartedAt === undefined ? 0 : Math.max(0, now - run.runStartedAt),
			firstTokenMs: run.firstTokenMs,
			turnCount: run.turnCount,
			lastTool: run.lastTool ? { ...run.lastTool } : undefined,
			failure: run.failure,
			failureAt: run.failureAt,
			lastEvent: run.lastEvent,
			lastEventAt: run.lastEventAt,
		};
	}

	/** Drop `sessionId`'s record (session deleted). */
	clear(sessionId: string): void {
		this.#runs.delete(sessionId);
		this.#retryPending.delete(sessionId);
	}

	#run(sessionId: string): MutableRunDiagnostic {
		let run = this.#runs.get(sessionId);
		if (!run) {
			run = {
				runActive: false,
				stage: "idle",
				activeTool: undefined,
				detail: undefined,
				runStartedAt: undefined,
				firstTokenMs: undefined,
				firstTokenSince: undefined,
				turnCount: 0,
				toolStartedAt: undefined,
				lastTool: undefined,
				pendingFailure: undefined,
				failure: undefined,
				failureAt: undefined,
				lastEvent: undefined,
				lastEventAt: undefined,
			};
			this.#runs.set(sessionId, run);
		}
		return run;
	}

	#touch(run: MutableRunDiagnostic, type: string, now: number): void {
		run.lastEvent = type;
		run.lastEventAt = now;
	}
}

interface MutableRunDiagnostic {
	runActive: boolean;
	stage: RunStage;
	activeTool: string | undefined;
	detail: string | undefined;
	runStartedAt: number | undefined;
	firstTokenMs: number | undefined;
	firstTokenSince: number | undefined;
	turnCount: number;
	toolStartedAt: number | undefined;
	lastTool: { name: string; durationMs: number } | undefined;
	pendingFailure: string | undefined;
	failure: string | undefined;
	failureAt: number | undefined;
	lastEvent: string | undefined;
	lastEventAt: number | undefined;
}

function truncateFailure(text: string): string {
	const trimmed = text.trim();
	if (trimmed.length <= MAX_FAILURE_CHARS) return trimmed;
	return `${trimmed.slice(0, MAX_FAILURE_CHARS)}…`;
}

/**
 * Failure text from a terminal turn's own messages — the same read
 * `sendErrorNotification` performs: only an assistant message the system
 * classified with `stopReason === "error"` counts, and only its text blocks.
 */
function errorTextFromMessages(messages: readonly { role: string }[]): string | undefined {
	const last = messages.findLast((message): message is AssistantMessage => message.role === "assistant");
	if (last?.stopReason !== "error") return undefined;
	if (last.errorMessage?.trim()) return truncateFailure(last.errorMessage);
	const text = last.content
		.filter((block): block is Extract<AssistantMessage["content"][number], { type: "text" }> => block.type === "text")
		.map(block => block.text)
		.join("")
		.trim();
	if (!text) return "The run stopped with an error.";
	return truncateFailure(text);
}

function toolErrorText(toolName: string, result: unknown): string {
	if (typeof result === "string" && result.trim()) return truncateFailure(result);
	return `${toolName} reported an error.`;
}
