/**
 * Per-server MCP approval overrides (Stream-F F2 slice).
 *
 * The MCP bridge mints every server tool with a uniform `approval = "write"`
 * tier (`tool-bridge.ts`, read-only), so `tools.approval.<tool>` alone cannot
 * distinguish two servers exposing the same tool name. This helper adds a
 * scoped lookup — `tools.approval.<server>.<tool>` — that falls back to the
 * existing `tools.approval.<tool>` contract. No UX redesign, no behavior flip
 * for callers that keep using `resolveApproval` directly.
 *
 * Lookup order (first valid policy wins):
 *  1. Flat dotted key `"<server>.<tool>"` in `tools.approval`.
 *  2. Nested object `tools.approval[<server>][<tool>]`.
 *  3. Minted registry name (e.g. `mcp__server_tool`) in `tools.approval`.
 *  4. Existing fallback `tools.approval[<tool>]` (original tool name).
 *
 * Fail-closed: only exact `allow`/`deny`/`prompt` (case-insensitive,
 * trimmed) are honored. Missing keys, wrong shapes, and invalid values are
 * ignored so resolution falls through to the next layer — never to `allow`.
 * Tool-owned `deny` still wins over any user policy via `resolveApproval`.
 */

import type { AgentTool } from "@harvest/pi-agent-core";
import { resolveApproval, type ApprovalMode, type ResolvedApproval } from "../tools/approval";

export type McpApprovalPolicy = "allow" | "deny" | "prompt";

export interface McpToolIdentity {
	readonly serverName: string;
	readonly toolName: string;
	readonly mintedName?: string;
}

export interface McpApprovalMatch {
	readonly policy: McpApprovalPolicy;
	readonly key: string;
}

export interface McpScopedApproval {
	readonly policy?: McpApprovalPolicy;
	readonly key?: string;
}

type ApprovalSubject = Pick<AgentTool, "name" | "approval" | "formatApprovalDetails">;

const POLICY_VALUES: ReadonlySet<McpApprovalPolicy> = new Set(["allow", "deny", "prompt"]);

/** Best-effort conversion of an arbitrary user-supplied value to a policy. */
export function normalizeMcpApprovalPolicy(value: unknown): McpApprovalPolicy | undefined {
	if (typeof value !== "string") return undefined;
	const lowered = value.trim().toLowerCase();
	return POLICY_VALUES.has(lowered as McpApprovalPolicy) ? (lowered as McpApprovalPolicy) : undefined;
}

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null && !Array.isArray(value);
}

function cleanSegment(value: unknown): string | undefined {
	if (typeof value !== "string") return undefined;
	const trimmed = value.trim();
	return trimmed.length > 0 ? trimmed : undefined;
}

/**
 * Find the scoped user policy for one MCP tool. Returns an empty object when
 * no layer carries a valid policy (fail-closed: caller falls back to mode
 * tiering, never to allow).
 */
export function resolveMcpApprovalPolicy(
	identity: McpToolIdentity,
	userConfig: Record<string, unknown> = {},
): McpScopedApproval {
	const server = cleanSegment(identity.serverName);
	const tool = cleanSegment(identity.toolName);
	const minted = cleanSegment(identity.mintedName);

	if (server !== undefined && tool !== undefined) {
		const dottedKey = `${server}.${tool}`;
		if (Object.hasOwn(userConfig, dottedKey)) {
			const policy = normalizeMcpApprovalPolicy(userConfig[dottedKey]);
			if (policy !== undefined) return { policy, key: dottedKey };
		}
		const nested = userConfig[server];
		if (isRecord(nested) && Object.hasOwn(nested, tool)) {
			const policy = normalizeMcpApprovalPolicy(nested[tool]);
			if (policy !== undefined) return { policy, key: `${server}.${tool}` };
		}
	}

	if (minted !== undefined && Object.hasOwn(userConfig, minted)) {
		const policy = normalizeMcpApprovalPolicy(userConfig[minted]);
		if (policy !== undefined) return { policy, key: minted };
	}

	if (tool !== undefined && Object.hasOwn(userConfig, tool)) {
		const policy = normalizeMcpApprovalPolicy(userConfig[tool]);
		if (policy !== undefined) return { policy, key: tool };
	}

	return {};
}

/**
 * Resolve approval for an MCP tool with per-server overrides applied.
 * A scoped match is injected as the tool's own user policy so tier, mode,
 * and tool-owned deny semantics stay identical to `resolveApproval`; the
 * reported `policyKey` names the scoped key (`<server>.<tool>`) so deny
 * messages point at the config the user must edit.
 */
export function resolveMcpApproval(
	tool: ApprovalSubject,
	identity: McpToolIdentity,
	args: unknown,
	mode: ApprovalMode,
	userConfig: Record<string, unknown> = {},
): ResolvedApproval {
	const scoped: McpScopedApproval = resolveMcpApprovalPolicy(identity, userConfig);
	if (scoped.policy === undefined) {
		return resolveApproval(tool, args, mode, userConfig);
	}
	const augmented: Record<string, unknown> = { ...userConfig, [tool.name]: scoped.policy };
	const resolved = resolveApproval(tool, args, mode, augmented);
	if (resolved.source === "user") {
		return { ...resolved, policyKey: scoped.key };
	}
	// Tool-owned deny (or tool override prompt) outranks user policy; keep it verbatim.
	return resolved;
}
