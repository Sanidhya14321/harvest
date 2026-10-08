import { afterEach, describe, expect, it } from "bun:test";
import { Settings } from "@harvest/pi-coding-agent/config/settings";
import { AgentRegistry } from "@harvest/pi-coding-agent/registry/agent-registry";
import { intersectCallerPolicy } from "@harvest/pi-coding-agent/session/live-session-factory";
import type { ToolSession } from "@harvest/pi-coding-agent/tools";
import type { AgentSession } from "@harvest/pi-coding-agent/session/agent-session";
import {
	clearSessionToolDepsForTests,
	setSessionToolDeps,
	SessionsTool,
	type ManagedLiveSessionLike,
	type OpenManagedSessionInput,
} from "@harvest/pi-coding-agent/tools/sessions";

function fakeLive(uuid: string): ManagedLiveSessionLike {
	return {
		getSessionId: () => uuid,
		abort: async () => {},
		setSessionName: async () => {},
		prompt: async () => undefined,
	};
}

function makeToolSession(uuid: string, agentId: string | null, registry: AgentRegistry, extra?: object): ToolSession {
	return {
		cwd: "/proj",
		hasUI: false,
		getSessionFile: () => null,
		getSessionSpawns: () => "*",
		taskDepth: 0,
		getSessionId: () => uuid,
		getAgentId: () => agentId,
		settings: Settings.isolated({ "autolearn.enabled": true }),
		agentRegistry: registry,
		...extra,
	} as ToolSession;
}

describe("effective caller policy intersection (S2)", () => {
	it("intersects tool names caller-ordered; empty stays explicitly empty", () => {
		const effective = intersectCallerPolicy(
			{ toolNames: ["read", "write", "bash"], restrictToolNames: true },
			{ toolNames: ["bash", "read", "exec"] },
		);
		expect(effective.toolNames).toEqual(["read", "bash"]);
		expect(effective.restrictToolNames).toBe(true);
		const empty = intersectCallerPolicy({ toolNames: ["read"], restrictToolNames: true }, { toolNames: ["exec"] });
		expect(empty.toolNames).toEqual([]);
		// An empty intersection is an explicit deny — never widened back.
		expect(empty.restrictToolNames).toBe(true);
	});

	it("one-sided tool names pass through verbatim; omission never widens", () => {
		expect(intersectCallerPolicy({ toolNames: ["read"], restrictToolNames: true }, undefined).toolNames).toEqual([
			"read",
		]);
		expect(intersectCallerPolicy({ restrictToolNames: false }, { toolNames: ["exec"] }).toolNames).toEqual(["exec"]);
		expect(intersectCallerPolicy({}, undefined).toolNames).toBeUndefined();
	});

	it("restrictions are never relaxed, approvals never escalated, MCP false wins", () => {
		expect(intersectCallerPolicy({ restrictToolNames: true }, { restrictToolNames: false }).restrictToolNames).toBe(
			true,
		);
		expect(intersectCallerPolicy({}, { restrictToolNames: true }).restrictToolNames).toBe(true);
		expect(intersectCallerPolicy({ autoApprove: true }, { autoApprove: false }).autoApprove).toBe(false);
		expect(intersectCallerPolicy({ autoApprove: false }, { autoApprove: true }).autoApprove).toBe(false);
		expect(intersectCallerPolicy({ autoApprove: true }, undefined).autoApprove).toBe(true);
		expect(intersectCallerPolicy({}, { autoApprove: true }).autoApprove).toBe(true);
		expect(intersectCallerPolicy({}, undefined).autoApprove).toBe(false);
		expect(intersectCallerPolicy({ enableMCP: true }, { enableMCP: false }).enableMCP).toBe(false);
		expect(intersectCallerPolicy({ enableMCP: false }, undefined).enableMCP).toBe(false);
		expect(intersectCallerPolicy({}, undefined).enableMCP).toBeUndefined();
	});

	it("a stated spawn disable is never widened by the other side", () => {
		expect(intersectCallerPolicy({ spawns: "" }, { spawns: "*" }).spawns).toBe("");
		expect(intersectCallerPolicy({ spawns: null }, undefined).spawns).toBeNull();
		expect(intersectCallerPolicy({ spawns: "task" }, undefined).spawns).toBe("task");
		expect(intersectCallerPolicy({}, undefined).spawns).toBe("*");
	});
});

describe("sessions tool forwards the effective caller policy verbatim (S2/S3)", () => {
	afterEach(() => {
		clearSessionToolDepsForTests();
	});

	it("forwards toolNames alongside restrictToolNames plus approval, depth, and the background contract", async () => {
		const registry = new AgentRegistry();
		registry.register({
			id: "Main",
			displayName: "Main",
			kind: "main",
			session: fakeLive("uuid-main") as unknown as AgentSession,
			status: "idle",
		});
		let seen: OpenManagedSessionInput | undefined;
		setSessionToolDeps({
			openSession: async input => {
				seen = input;
				const live = fakeLive("uuid-fresh");
				registry.register({
					id: "tab:uuid-fresh",
					displayName: "tab:uuid-fresh",
					kind: "sub",
					parentId: input.callerId ?? undefined,
					session: live as unknown as AgentSession,
					status: "idle",
				});
				return { id: "uuid-fresh", registryId: "tab:uuid-fresh", taskAccepted: true, session: live };
			},
		});
		const tool = new SessionsTool(
			makeToolSession("uuid-main", "Main", registry, { toolNames: ["read", "bash"], autoApprove: true }),
		);
		await tool.execute("1", { action: "create", task: "do work" });
		expect(seen?.background).toBe(true);
		expect(seen?.callerPolicy?.toolNames).toEqual(["read", "bash"]);
		expect(seen?.callerPolicy?.restrictToolNames).toBeUndefined();
		expect(seen?.callerPolicy?.autoApprove).toBe(true);
		expect(seen?.callerPolicy?.spawns).toBe("*");
		expect(seen?.callerTaskDepth).toBe(0);
		expect(seen?.taskDepth).toBe(1);
	});

	it("absent policy fields stay absent (omission never widens downstream)", async () => {
		const registry = new AgentRegistry();
		registry.register({
			id: "Main",
			displayName: "Main",
			kind: "main",
			session: fakeLive("uuid-main") as unknown as AgentSession,
			status: "idle",
		});
		let seen: OpenManagedSessionInput | undefined;
		setSessionToolDeps({
			openSession: async input => {
				seen = input;
				return {
					id: "uuid-x",
					registryId: "tab:uuid-x",
					taskAccepted: false,
					session: fakeLive("uuid-x"),
				};
			},
		});
		const tool = new SessionsTool(makeToolSession("uuid-main", "Main", registry));
		await tool.execute("1", { action: "create" });
		expect(seen?.callerPolicy?.toolNames).toBeUndefined();
		expect(seen?.callerPolicy?.autoApprove).toBeUndefined();
	});
});
