/**
 * Decision-layer circuit breaker and per-call-site statistics.
 *
 * After repeated consecutive sidecar failures (timeouts, overload, connection
 * loss) for an OPTIONAL decision point, further calls fail fast without
 * spending another HTTP round trip, for a cooldown. Required decisions
 * (tool gating) and explicit infrastructure calls (smoke tests, calibration,
 * identity probes) always attempt: gating still fails closed to approval,
 * so the breaker can never turn an unavailable sidecar into permission.
 *
 * Statistics (calls, fallbacks, timeouts, p50/p95 over a bounded latency
 * window) reuse this module instead of a second telemetry stack; per-decision
 * detail already flows into the existing decision logs and run diagnostics.
 */

export interface DecisionPointStats {
	readonly calls: number;
	readonly fallbacks: number;
	readonly timeouts: number;
	readonly consecutiveFailures: number;
	readonly breakerOpen: boolean;
	readonly p50Ms: number | undefined;
	readonly p95Ms: number | undefined;
}

/** Optional decision points the breaker may skip while open. */
const BREAKER_ELIGIBLE_CALL_SITES: ReadonlySet<string> = new Set([
	"context_pruning",
	"brain_retrieval",
	"subagent_selection",
	"model_routing",
	"completion_check",
]);

/** Consecutive eligible failures that open the breaker. */
export const MAX_CONSECUTIVE_DECISION_FAILURES = 5;
/** How long an open breaker fails fast before letting one call through. */
export const DECISION_BREAKER_COOLDOWN_MS = 30_000;
/** Bounded latency window per call site for p50/p95. */
const LATENCY_WINDOW = 100;
/** Fallback reasons that say nothing about sidecar health. */
const HEALTH_NEUTRAL_FALLBACKS: ReadonlySet<string> = new Set([
	"non_english_input",
	"non_english_state",
	"operation_cancelled",
	"breaker_open_sidecar_overloaded",
]);

interface BreakerState {
	consecutiveFailures: number;
	openedAt: number | undefined;
	calls: number;
	fallbacks: number;
	timeouts: number;
	latencies: number[];
}

const states = new Map<string, BreakerState>();

function stateFor(callSite: string): BreakerState {
	let state = states.get(callSite);
	if (!state) {
		state = { consecutiveFailures: 0, openedAt: undefined, calls: 0, fallbacks: 0, timeouts: 0, latencies: [] };
		states.set(callSite, state);
	}
	return state;
}

function percentile(sorted: readonly number[], fraction: number): number | undefined {
	if (sorted.length === 0) return undefined;
	return sorted[Math.min(sorted.length - 1, Math.floor(fraction * sorted.length))];
}

/** True when an optional decision should fail fast instead of attempting HTTP. */
export function isBreakerOpen(callSite: string): boolean {
	if (!BREAKER_ELIGIBLE_CALL_SITES.has(callSite)) return false;
	const state = states.get(callSite);
	if (!state || state.openedAt === undefined) return false;
	if (Date.now() - state.openedAt >= DECISION_BREAKER_COOLDOWN_MS) {
		state.openedAt = undefined;
		state.consecutiveFailures = 0;
		return false;
	}
	return true;
}

export interface DecisionOutcome {
	readonly success: boolean;
	readonly fallbackReason?: string;
	readonly latencyMs: number;
}

/** Record one settled decision: resets on success, opens after repeated eligible failures. */
export function recordDecisionOutcome(callSite: string, outcome: DecisionOutcome): void {
	const state = stateFor(callSite);
	state.calls++;
	if (Number.isFinite(outcome.latencyMs) && outcome.latencyMs >= 0) {
		state.latencies.push(outcome.latencyMs);
		if (state.latencies.length > LATENCY_WINDOW) state.latencies.splice(0, state.latencies.length - LATENCY_WINDOW);
	}
	if (outcome.success) {
		state.consecutiveFailures = 0;
		state.openedAt = undefined;
		return;
	}
	state.fallbacks++;
	if (outcome.fallbackReason?.startsWith("timeout_exceeded")) state.timeouts++;
	if (outcome.fallbackReason !== undefined && HEALTH_NEUTRAL_FALLBACKS.has(outcome.fallbackReason)) return;
	if (!BREAKER_ELIGIBLE_CALL_SITES.has(callSite)) return;
	state.consecutiveFailures++;
	if (state.consecutiveFailures >= MAX_CONSECUTIVE_DECISION_FAILURES) {
		state.openedAt = Date.now();
	}
}

/** Snapshot statistics for one call site. */
export function getDecisionStats(callSite: string): DecisionPointStats {
	const state = states.get(callSite);
	const latencies = [...(state?.latencies ?? [])].sort((a, b) => a - b);
	return {
		calls: state?.calls ?? 0,
		fallbacks: state?.fallbacks ?? 0,
		timeouts: state?.timeouts ?? 0,
		consecutiveFailures: state?.consecutiveFailures ?? 0,
		breakerOpen: isBreakerOpen(callSite),
		p50Ms: percentile(latencies, 0.5),
		p95Ms: percentile(latencies, 0.95),
	};
}

/** Clear breaker state (tests, or after a deliberate sidecar restart). */
export function resetDecisionBreaker(callSite?: string): void {
	if (callSite === undefined) states.clear();
	else states.delete(callSite);
}
