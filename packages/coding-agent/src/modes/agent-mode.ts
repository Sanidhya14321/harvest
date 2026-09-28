import type { ApprovalMode } from "../tools/approval";

/**
 * Unified agent operating mode cycled with Shift+Tab.
 *
 * This is the Opencode / Claude Code style mode switch:
 * - `plan` — read-only planning (plan mode enabled)
 * - `build` — ask before writes and exec (`always-ask`)
 * - `accept-edits` — auto-accept file edits, ask for exec (`write`)
 * - `auto` — auto-accept everything (`yolo`)
 *
 * The order is an escalating permissiveness ladder so repeated Shift+Tab
 * walks plan → build → accept-edits → auto → plan.
 */
export type AgentMode = "plan" | "build" | "accept-edits" | "auto";

export interface AgentModeDef {
	id: AgentMode;
	/** Short label used in the status line and status messages. */
	label: string;
	/** One-line description shown on cycle. */
	description: string;
	/** Approval mode applied when this agent mode is active. Undefined for plan. */
	approvalMode?: ApprovalMode;
}

export const AGENT_MODE_ORDER: readonly AgentMode[] = ["plan", "build", "accept-edits", "auto"];

export const AGENT_MODE_DEFS: Record<AgentMode, AgentModeDef> = {
	plan: {
		id: "plan",
		label: "Plan",
		description: "Read-only planning — no writes or exec without approval",
	},
	build: {
		id: "build",
		label: "Build",
		description: "Ask before file edits and commands",
		approvalMode: "always-ask",
	},
	"accept-edits": {
		id: "accept-edits",
		label: "Accept Edits",
		description: "Auto-accept file edits, ask before commands",
		approvalMode: "write",
	},
	auto: {
		id: "auto",
		label: "Auto",
		description: "Auto-accept edits and commands",
		approvalMode: "yolo",
	},
};

function normalizeApprovalMode(value: unknown): ApprovalMode {
	return value === "always-ask" || value === "write" || value === "yolo" ? value : "yolo";
}

/**
 * Resolve the current agent mode from plan state + approval setting.
 * Plan wins over any approval value; paused plan still reads as plan so a
 * mid-toggle cycle lands on a deterministic next mode.
 */
export function getAgentMode(planActive: boolean, approvalMode: unknown): AgentMode {
	if (planActive) return "plan";
	switch (normalizeApprovalMode(approvalMode)) {
		case "always-ask":
			return "build";
		case "write":
			return "accept-edits";
		case "yolo":
			return "auto";
	}
}

/** Next mode in the Shift+Tab ladder, wrapping around. */
export function nextAgentMode(current: AgentMode): AgentMode {
	const index = AGENT_MODE_ORDER.indexOf(current);
	return AGENT_MODE_ORDER[(index + 1) % AGENT_MODE_ORDER.length] ?? "plan";
}

export function agentModeDef(mode: AgentMode): AgentModeDef {
	return AGENT_MODE_DEFS[mode];
}
