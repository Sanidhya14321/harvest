import { describe, expect, it } from "bun:test";
import {
	resolveMcpApproval,
	resolveMcpApprovalPolicy,
	type McpToolIdentity,
} from "@harvest/pi-coding-agent/mcp/mcp-approval-policy";
import { denyError, type ApprovalMode } from "@harvest/pi-coding-agent/tools/approval";
import type { AgentTool } from "@harvest/pi-agent-core";

type ApprovalSubject = Pick<AgentTool, "name" | "approval" | "formatApprovalDetails">;

const MODE: ApprovalMode = "always-ask";
// Mirrors the bridge: every MCP tool mints with a uniform write tier.
const bridgeTool: ApprovalSubject = { name: "mcp__billing_publish", approval: "write" };
const identity: McpToolIdentity = {
	serverName: "billing",
	toolName: "publish",
	mintedName: "mcp__billing_publish",
};

describe("mcp approval policy lookup", () => {
	it("prefers nested tools.approval.<server>.<tool> over tools.approval.<tool>", () => {
		const match = resolveMcpApprovalPolicy(identity, {
			publish: "allow",
			billing: { publish: "deny" },
		});
		expect(match).toEqual({ policy: "deny", key: "billing.publish" });
		const resolved = resolveMcpApproval(bridgeTool, identity, {}, MODE, {
			publish: "allow",
			billing: { publish: "deny" },
		});
		expect(resolved).toMatchObject({ policy: "deny", source: "user", policyKey: "billing.publish" });
	});

	it("honors the flat dotted tools.approval.<server>.<tool> key", () => {
		const userConfig = { "billing.publish": "prompt" };
		expect(resolveMcpApprovalPolicy(identity, userConfig)).toEqual({
			policy: "prompt",
			key: "billing.publish",
		});
		expect(resolveMcpApproval(bridgeTool, identity, {}, "yolo", userConfig).policy).toBe("prompt");
	});

	it("falls back to tools.approval.<tool> when no scoped policy exists", () => {
		expect(resolveMcpApprovalPolicy(identity, { publish: "deny" })).toEqual({
			policy: "deny",
			key: "publish",
		});
		const resolved = resolveMcpApproval(bridgeTool, identity, {}, MODE, { publish: "deny" });
		expect(resolved).toMatchObject({ policy: "deny", source: "user", policyKey: "publish" });
	});

	it("names the scoped key in user-deny refusals", () => {
		const resolved = resolveMcpApproval(bridgeTool, identity, {}, MODE, {
			billing: { publish: "deny" },
		});
		expect(() => {
			throw denyError(resolved, bridgeTool.name);
		}).toThrow('remove "tools.approval.billing.publish: deny"');
	});

	it("ignores invalid scoped values and falls through fail-closed", () => {
		// Invalid scoped values must not allow: with a write-tier tool in
		// always-ask mode the mode decision (prompt) applies.
		const userConfig = { billing: { publish: "yes" }, "billing.publish": 1 };
		expect(resolveMcpApprovalPolicy(identity, userConfig)).toEqual({});
		expect(resolveMcpApproval(bridgeTool, identity, {}, MODE, userConfig)).toMatchObject({
			policy: "prompt",
			source: "mode",
		});
	});

	it("keeps tool-owned deny above a scoped allow", () => {
		const blocked: ApprovalSubject = {
			name: "mcp__billing_publish",
			approval: { tier: "write", override: true, policy: "deny", reason: "Blocked by tool" },
		};
		const resolved = resolveMcpApproval(blocked, identity, {}, "yolo", {
			billing: { publish: "allow" },
		});
		expect(resolved).toMatchObject({ policy: "deny", source: "tool" });
	});

	it("honors scoped deny in yolo mode where the tier alone would allow", () => {
		const resolved = resolveMcpApproval(bridgeTool, identity, {}, "yolo", {
			billing: { publish: "deny" },
		});
		expect(resolved).toMatchObject({ policy: "deny", source: "user", policyKey: "billing.publish" });
	});
});
