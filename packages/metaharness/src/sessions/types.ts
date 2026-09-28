/**
 * Harness session types: OpenCode-style session view over benchmark trials.
 *
 * A harness session is a read mirror of one trial's conversation artifacts,
 * optionally backed by a live coding-agent `SessionManager` sidecar journal
 * when `META_SESSIONS=live`. The filesystem stays the source of truth; this
 * layer never writes into trial dirs.
 */

export type HarnessSessionStatus = "pass" | "fail" | "error" | "running";

export interface HarnessSession {
	id: string;
	run: string;
	trial: string;
	task: string;
	status: HarnessSessionStatus;
	/** Trial transcript locator (e.g. `<trial>/agent/omp.txt` or `record:N`). Null when unknown. */
	tracePath: string | null;
	/** Sidecar JSONL journal for live sessions. Null in mirror mode. */
	sessionFile: string | null;
	costUsd: number;
	durationMs: number;
	detail: string;
	updatedAt: number;
	live: boolean;
}

export interface HarnessSessionListFilter {
	run?: string;
	status?: HarnessSessionStatus;
}
