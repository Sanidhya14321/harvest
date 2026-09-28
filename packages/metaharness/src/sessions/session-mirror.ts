/**
 * Read-only mirror: trial dirs and normalized benchmark snapshots become
 * harness sessions. No writes, no SessionManager instantiation — safe to run
 * on every sync tick.
 */
import type { BenchmarkSnapshot, BenchmarkTrace } from "../benchmarks";
import type { HarnessSession, HarnessSessionStatus } from "./types";

function toStatus(status: BenchmarkTrace["status"]): HarnessSessionStatus {
	if (status === "pass" || status === "fail" || status === "error" || status === "running") return status;
	return "error";
}

export function mirrorTraceToSession(run: string, trace: BenchmarkTrace): HarnessSession {
	return {
		id: `${run}:${trace.name}`,
		run,
		trial: trace.name,
		task: trace.task,
		status: toStatus(trace.status),
		tracePath: trace.tracePath,
		sessionFile: null,
		costUsd: trace.costUsd,
		durationMs: trace.durationMs,
		detail: trace.detail,
		updatedAt: Date.now(),
		live: false,
	};
}

export function mirrorSnapshotToSessions(run: string, snapshot: BenchmarkSnapshot): HarnessSession[] {
	return snapshot.traces.map(trace => mirrorTraceToSession(run, trace));
}
