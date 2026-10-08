import { afterEach, describe, expect, it, vi } from "bun:test";
import { type SettingPath, Settings } from "@harvest/pi-coding-agent/config/settings";
import { AgentRegistry } from "@harvest/pi-coding-agent/registry/agent-registry";
import type { AgentSession } from "@harvest/pi-coding-agent/session/agent-session";
import {
	openLiveAgentSessionFromSnapshot,
	snapshotTrustedCaller,
	type OpenLiveSessionOptions,
} from "@harvest/pi-coding-agent/session/live-session-factory";
import type { ToolSession } from "@harvest/pi-coding-agent/tools";
import {
	clearSessionToolDepsForTests,
	dispatchInitialTaskOnce,
	setSessionToolDeps,
	SessionsTool,
	type ManagedLiveSessionLike,
	type OpenManagedSessionInput,
} from "@harvest/pi-coding-agent/tools/sessions";

function fakeLive(uuid: string): ManagedLiveSessionLike {
	const calls: string[] = [];
	const live: ManagedLiveSessionLike = {
		getSessionId: () => uuid,
		abort: async () => {
			calls.push("abort");
		},
		setSessionName: async () => {
			calls.push("rename");
		},
		prompt: async () => {
			calls.push("prompt");
			return undefined;
		},
	};
	(live as unknown as { calls: string[] }).calls = calls;
	(live as unknown as { sessionManager: { getSessionId: () => string } }).sessionManager = {
		getSessionId: () => uuid,
	};
	return live;
}

function liveCalls(live: ManagedLiveSessionLike): string[] {
	return (live as unknown as { calls: string[] }).calls;
}

function makeToolSession(
	uuid: string,
	agentId: string | null,
	registry: AgentRegistry,
	extra?: Partial<ToolSession>,
	settingsOverrides?: Partial<Record<SettingPath, unknown>>,
): ToolSession {
	return {
		cwd: "/proj",
		hasUI: false,
		skipPythonPreflight: true,
		getSessionFile: () => null,
		getSessionSpawns: () => "*",
		taskDepth: 0,
		getSessionId: () => uuid,
		getAgentId: () => agentId,
		settings: Settings.isolated({ "autolearn.enabled": true, ...settingsOverrides }),
		agentRegistry: registry,
		...extra,
	} as ToolSession;
}

/** Register a live ref carrying both identities: registry ID + session UUID. */
function registerLive(
	registry: AgentRegistry,
	ref: { id: string; uuid: string; parentId?: string; status?: "running" | "idle" },
): ManagedLiveSessionLike {
	const live = fakeLive(ref.uuid);
	registry.register({
		id: ref.id,
		displayName: ref.id,
		kind: "sub",
		parentId: ref.parentId,
		session: live as unknown as AgentSession,
		status: ref.status ?? "idle",
	});
	return live;
}

describe("sessions tool caller ownership (A6/A7)", () => {
	afterEach(() => {
		clearSessionToolDepsForTests();
		vi.restoreAllMocks();
	});

	function mainAndChild(status: "running" | "idle" = "idle"): {
		registry: AgentRegistry;
		tool: SessionsTool;
		child: ManagedLiveSessionLike;
	} {
		const registry = new AgentRegistry();
		registerLive(registry, { id: "Main", uuid: "uuid-main" });
		const child = registerLive(registry, {
			id: "tab:uuid-child",
			uuid: "uuid-child",
			parentId: "Main",
			status,
		});
		const tool = new SessionsTool(makeToolSession("uuid-main", "Main", registry));
		return { registry, tool, child };
	}

	it("manages owned children by public UUID: inspect, rename, send, stop", async () => {
		const { tool, child } = mainAndChild();
		const inspected = await tool.execute("1", { action: "inspect", sessionId: "uuid-child" });
		expect((inspected.details as { id: string }).id).toBe("tab:uuid-child");
		expect((inspected.details as { uuid: string }).uuid).toBe("uuid-child");
		await tool.execute("2", { action: "rename", sessionId: "uuid-child", title: "Worker" });
		await tool.execute("3", { action: "send", sessionId: "uuid-child", message: "status?" });
		await tool.execute("4", { action: "stop", sessionId: "uuid-child" });
		expect(liveCalls(child)).toEqual(["rename", "prompt", "abort"]);
		// Registry-ID spelling works too.
		await tool.execute("5", { action: "inspect", sessionId: "tab:uuid-child" });
	});

	it("a background caller manages its own child lineage", async () => {
		const registry = new AgentRegistry();
		registerLive(registry, { id: "Main", uuid: "uuid-main" });
		registerLive(registry, { id: "tab:uuid-bg", uuid: "uuid-bg", parentId: "Main" });
		const grandchild = registerLive(registry, {
			id: "tab:uuid-grand",
			uuid: "uuid-grand",
			parentId: "tab:uuid-bg",
		});
		const sibling = registerLive(registry, {
			id: "tab:uuid-sibling",
			uuid: "uuid-sibling",
			parentId: "Main",
		});
		const backgroundCaller = new SessionsTool(makeToolSession("uuid-bg", "tab:uuid-bg", registry));
		await backgroundCaller.execute("1", { action: "rename", sessionId: "uuid-grand", title: "G" });
		expect(liveCalls(grandchild)).toContain("rename");
		// Main's other child is outside the background caller's lineage.
		await expect(
			backgroundCaller.execute("2", { action: "send", sessionId: "uuid-sibling", message: "hi" }),
		).rejects.toThrow(/another lineage/);
		expect(liveCalls(sibling)).not.toContain("prompt");
	});

	it("two owners stay isolated: foreign lineages are denied without a grant", async () => {
		const registry = new AgentRegistry();
		registerLive(registry, { id: "Main", uuid: "uuid-main" });
		const other = registerLive(registry, {
			id: "tab:uuid-other",
			uuid: "uuid-other",
			parentId: "tab:uuid-bg",
		});
		const tool = new SessionsTool(makeToolSession("uuid-main", "Main", registry));
		await expect(tool.execute("1", { action: "stop", sessionId: "uuid-other" })).rejects.toThrow(
			/host-provided grant or the existing approval path/,
		);
		expect(liveCalls(other)).not.toContain("abort");
	});

	it("refuses self-stop however the caller is spelled", async () => {
		const { tool } = mainAndChild("running");
		await expect(tool.execute("1", { action: "stop", sessionId: "uuid-main" })).rejects.toThrow(/itself/);
		await expect(tool.execute("2", { action: "stop", sessionId: "Main" })).rejects.toThrow(/itself/);
	});

	it("capacity is shared across owners", async () => {
		const registry = new AgentRegistry();
		registerLive(registry, { id: "Main", uuid: "uuid-main" });
		registerLive(registry, { id: "tab:uuid-bg", uuid: "uuid-bg", parentId: "Main", status: "running" });
		setSessionToolDeps({
			openSession: async () => ({
				id: "never",
				registryId: "tab:never",
				taskAccepted: false,
				session: fakeLive("never"),
			}),
		});
		const ownerA = new SessionsTool(
			makeToolSession("uuid-main", "Main", registry, undefined, { "task.maxConcurrency": 1 }),
		);
		const ownerB = new SessionsTool(
			makeToolSession("uuid-bg", "tab:uuid-bg", registry, undefined, { "task.maxConcurrency": 1 }),
		);
		await expect(ownerA.execute("1", { action: "create" })).rejects.toThrow(/maxConcurrency/);
		await expect(ownerB.execute("1", { action: "create" })).rejects.toThrow(/maxConcurrency/);
	});

	it("creator manages its own child by returned UUID", async () => {
		const registry = new AgentRegistry();
		registerLive(registry, { id: "Main", uuid: "uuid-main" });
		let seen: OpenManagedSessionInput | undefined;
		setSessionToolDeps({
			openSession: async input => {
				seen = input;
				// Owner records lineage in registry space, adopts the runtime.
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
		const tool = new SessionsTool(makeToolSession("uuid-main", "Main", registry));
		const created = await tool.execute("1", { action: "create", task: "do work" });
		expect((created.details as { id: string }).id).toBe("uuid-fresh");
		// Parent link is the trusted registry identity, plus trusted depth + policy.
		expect(seen?.callerId).toBe("Main");
		expect(seen?.callerTaskDepth).toBe(0);
		expect(seen?.taskDepth).toBe(1);
		expect(seen?.callerPolicy?.spawns).toBe("*");
		const child = registry.list().find(ref => ref.id === "tab:uuid-fresh");
		expect(child?.parentId).toBe("Main");
		await tool.execute("2", { action: "inspect", sessionId: "uuid-fresh" });
		await tool.execute("3", { action: "rename", sessionId: "uuid-fresh", title: "Fresh" });
		await tool.execute("4", { action: "send", sessionId: "uuid-fresh", message: "go" });
		await tool.execute("5", { action: "stop", sessionId: "uuid-fresh" });
	});

	it("prefers the session-scoped owner seam over the legacy global", async () => {
		const registry = new AgentRegistry();
		registerLive(registry, { id: "Main", uuid: "uuid-main" });
		setSessionToolDeps({
			openSession: async () => ({
				id: "uuid-global",
				registryId: "tab:uuid-global",
				taskAccepted: false,
				session: fakeLive("uuid-global"),
			}),
		});
		const scoped = new SessionsTool(
			makeToolSession("uuid-main", "Main", registry, {
				openManagedSession: async () => ({
					id: "uuid-scoped",
					registryId: "tab:uuid-scoped",
					taskAccepted: true,
					session: fakeLive("uuid-scoped"),
				}),
			}),
		);
		const created = await scoped.execute("1", { action: "create" });
		expect((created.details as { id: string }).id).toBe("uuid-scoped");
	});

	it("dispatchInitialTaskOnce dispatches exactly once and reports truthfully", async () => {
		const calls: string[] = [];
		const target = {
			prompt: async (message: string) => {
				calls.push(message);
				return undefined;
			},
		};
		await expect(dispatchInitialTaskOnce(target, "  do work  ")).resolves.toBe(true);
		await Bun.sleep(50);
		expect(calls).toEqual(["do work"]);
		await expect(dispatchInitialTaskOnce(target, "   ")).resolves.toBe(false);
		await expect(dispatchInitialTaskOnce(target, undefined)).resolves.toBe(false);
		expect(calls).toEqual(["do work"]);

		const failures: unknown[] = [];
		const failing = {
			prompt: async () => {
				throw new Error("boom");
			},
		};
		// Fire-and-forget acceptance is truthful about the attempt; the later
		// failure is routed to onError, never redelivered.
		await expect(dispatchInitialTaskOnce(failing, "go", error => failures.push(error))).resolves.toBe(true);
		await Bun.sleep(50);
		expect(failures).toHaveLength(1);
	});
});

describe("trusted caller snapshot propagation (A5)", () => {
	function bareSource() {
		const settings = Settings.isolated({});
		return {
			sessionManager: {
				getCwd: () => "/proj",
				getSessionDir: () => "/proj/sessions",
			},
			session: {
				settings,
				modelRegistry: { authStorage: {} },
				model: { id: "m" },
				effectiveExtensionRoots: {},
				getAvailableModels: () => [{ id: "m" }],
			},
			mcpManager: { id: "mcp" },
		} as unknown as Parameters<typeof snapshotTrustedCaller>[0];
	}

	it("a restricted background caller begets a restricted child", async () => {
		const snapshot = snapshotTrustedCaller(bareSource(), {
			callerSessionId: "uuid-bg",
			callerAgentId: "tab:uuid-bg",
			taskDepth: 2,
			policyFallback: { restrictToolNames: true, spawns: "", autoApprove: true, enableMCP: false },
		});
		expect(snapshot.taskDepth).toBe(3);
		expect(snapshot.callerAgentId).toBe("tab:uuid-bg");
		expect(snapshot.policy.restrictToolNames).toBe(true);
		expect(snapshot.policy.spawns).toBe("");
		// Auto-approve from the fallback is carried, never widened beyond it.
		expect(snapshot.policy.autoApprove).toBe(true);
		expect(snapshot.policy.enableMCP).toBe(false);

		const seen: Record<string, unknown>[] = [];
		type CreateSeam = NonNullable<OpenLiveSessionOptions["createSession"]>;
		await openLiveAgentSessionFromSnapshot(snapshot, {
			agentId: "tab:uuid-fresh",
			background: true,
			createSession: (async (options: Record<string, unknown>) => {
				seen.push(options);
				return { session: {} };
			}) as unknown as CreateSeam,
		});
		const options = seen[0]!;
		expect(options["restrictToolNames"]).toBe(true);
		expect(options["spawns"]).toBe("");
		expect(options["hasUI"]).toBe(false);
		expect(options["taskDepth"]).toBe(3);
		expect(options["parentAgentId"]).toBe("tab:uuid-bg");
		// MCP forwarding prohibited: the inherited manager is dropped.
		expect(options["mcpManager"]).toBeUndefined();
	});

	it("live-session facts win over the ToolSession fallback", () => {
		const source = bareSource();
		(source.session as unknown as Record<string, unknown>)["getSessionSpawns"] = () => "task";
		const snapshot = snapshotTrustedCaller(source, {
			policyFallback: { spawns: "", restrictToolNames: true },
		});
		expect(snapshot.policy.spawns).toBe("task");
		expect(snapshot.policy.restrictToolNames).toBe(true);
	});
});
