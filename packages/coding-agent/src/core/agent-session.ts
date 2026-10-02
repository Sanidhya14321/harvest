/**
 * Agent Session Harvest Integration & Session Facade.
 *
 * Integrates Entropy-Gated Role Routing, Pre-Read Enforcement,
 * Execution Grounding with Verify-Before-Done Nudging, and
 * File Freshness Tracking into the agent session lifecycle.
 */

import type { ToolTier } from "@harvest/pi-agent-core";
import type { Settings } from "../config/settings";
import { ExecutionGroundingEngine } from "./harvest/grounding";
import { PreReadEnforcement, type ToolCallPayload } from "./harvest/loop-policy";
import { routeRole, type RoleRoutingResult, type SpecialistRole } from "./harvest/roles";
import { FileSession } from "./tools/file-session";
import { FreshnessTracker } from "./tools/freshness";

import { checkToolCallGating, type ToolGatingDecision } from "./harvest/laya-gating";

export * from "../session/agent-session";
export * from "./harvest/laya-client";
export * from "./harvest/laya-gating";
export * from "./harvest/laya-routing";
export * from "./harvest/laya-completion";
export * from "./harvest/laya-pruning";

export interface HarvestSessionHooks {
	readonly preReadEnforcement: PreReadEnforcement;
	readonly freshnessTracker: FreshnessTracker;
	readonly fileSession: FileSession;
	readonly groundingEngine: ExecutionGroundingEngine;
	readonly activeRole: SpecialistRole;
	readonly routingResult: RoleRoutingResult;
}

/**
 * Create unified Harvest session controller with all mutation safeguards
 * and execution grounding active.
 */
export function createHarvestSession(workspaceRoot: string = process.cwd(), promptText?: string): HarvestSessionHooks {
	const routingResult = promptText ? routeRole(promptText) : routeRole("");
	const preReadEnforcement = new PreReadEnforcement(workspaceRoot);
	const freshnessTracker = new FreshnessTracker(workspaceRoot);
	const fileSession = new FileSession(workspaceRoot);
	const groundingEngine = new ExecutionGroundingEngine(workspaceRoot);

	return {
		preReadEnforcement,
		freshnessTracker,
		fileSession,
		groundingEngine,
		activeRole: routingResult.role,
		routingResult,
	};
}

/**
 * Verify-Before-Done Nudge helper for session loops.
 */
export function installGroundingNudge(
	groundingEngine: ExecutionGroundingEngine,
	options: { stopReason?: string; role?: string; isStandaloneDeliverable?: boolean } = {},
): { shouldNudge: boolean; steerPrompt?: string } {
	return groundingEngine.checkVerifyBeforeDoneNudge(options);
}

/**
 * Intercept tool calls in session loop for pre-read enforcement and freshness recording.
 */
export function interceptSessionToolCall(
	hooks: HarvestSessionHooks,
	toolCall: ToolCallPayload,
): { allowed: boolean; error?: string } {
	const check = hooks.preReadEnforcement.interceptToolCall(toolCall);
	if (!check.allowed) {
		return check;
	}

	const name = toolCall.name.toLowerCase();
	const args = toolCall.args ?? {};
	const rawPath =
		(typeof args.path === "string" && args.path) ||
		(typeof args.filePath === "string" && args.filePath) ||
		(typeof args.targetFile === "string" && args.targetFile) ||
		(typeof args.TargetFile === "string" && args.TargetFile) ||
		(typeof args.target_file === "string" && args.target_file) ||
		(typeof args.file === "string" && args.file) ||
		(typeof args.file_path === "string" && args.file_path) ||
		(typeof args.AbsolutePath === "string" && args.AbsolutePath) ||
		undefined;

	if ((name === "read" || name === "view_file" || name === "read_file") && rawPath) {
		hooks.freshnessTracker.recordRead(rawPath);
	} else if (
		name === "write" ||
		name === "edit" ||
		name === "replace_file_content" ||
		name === "write_to_file" ||
		name === "ast-edit" ||
		name === "ast_edit"
	) {
		if (rawPath) {
			hooks.groundingEngine.recordFileMutation(rawPath);
		}
	}

	return { allowed: true };
}

/**
 * Laya Tool-Call Gating Interceptor for session loops.
 * Evaluates high-risk tool calls with a noul question.
 */
export async function interceptSessionToolCallLaya(
	toolCall: ToolCallPayload,
	sessionId?: string,
	signal?: AbortSignal,
	toolTier?: ToolTier,
	owningSettings?: Settings,
): Promise<ToolGatingDecision> {
	return await checkToolCallGating(toolCall.name, toolCall.args ?? {}, {
		sessionId,
		signal,
		toolTier,
		settings: owningSettings,
	});
}
