/**
 * Harness agent types: OpenCode-style agent view over benchmark workers.
 *
 * Statuses mirror `AgentRegistry` (`running|idle|parked|aborted`) so a future
 * real `AgentSession` can attach without a schema change. Harness agents are
 * logical workers (one per running trial plus one per managed run); they carry
 * `session: null` until a live session is explicitly attached.
 */

export type HarnessAgentStatus = "running" | "idle" | "parked" | "aborted";

export type HarnessAgentKind = "trial-worker" | "benchmark-runner";

export interface HarnessAgent {
	id: string;
	displayName: string;
	kind: HarnessAgentKind;
	run: string;
	trial: string | null;
	status: HarnessAgentStatus;
	pid: number | null;
	sessionId: string | null;
	sessionFile: string | null;
	createdAt: number;
	lastActivity: number;
}

export interface HarnessAgentListFilter {
	run?: string;
	status?: HarnessAgentStatus;
}
