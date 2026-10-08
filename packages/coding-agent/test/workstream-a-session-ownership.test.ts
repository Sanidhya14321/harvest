import { afterEach, describe, expect, it } from "bun:test";
import { Settings } from "@harvest/pi-coding-agent/config/settings";
import { AgentRegistry } from "@harvest/pi-coding-agent/registry/agent-registry";
import type { AgentSession } from "@harvest/pi-coding-agent/session/agent-session";
import type { ToolSession } from "@harvest/pi-coding-agent/tools";
import {
	clearSessionToolDepsForTests,
	setSessionToolDeps,
	SessionsTool,
	type ManagedLiveSessionLike,
	type OpenManagedSessionInput,
} from "@harvest/pi-coding-agent/tools/sessions";

function fakeLive(uuid: string, opts?: { prompt?: boolean }): ManagedLiveSessionLike {
	const live: ManagedLiveSessionLike = {
		getSessionId: () => uuid,
		abort: async () => {},
		setSessionName: async () => {},
	};
	if (opts?.prompt !== false) {
		live.prompt = async () => undefined;
	}
	(live as unknown as { sessionManager: { getSessionId: () => string } }).sessionManager = {
		getSessionId: () => uuid,
	};
	return live;
}

function makeToolSession(
	cwd: string,
	uuid: string,
	agentId: string | null,
	registry: AgentRegistry,
	extra?: Record<string, unknown>,
): ToolSession {
	return {
		cwd,
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

function registerLive(
	registry: AgentRegistry,
	ref: { id: string; uuid: string; parentId?: string; status?: "running" | "idle" },
	prompt = true,
): ManagedLiveSessionLike {
	const live = fakeLive(ref.uuid, { prompt });
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

describe("sessions tool two-owner isolation (T04)", () => {
	afterEach(() => {
		clearSessionToolDepsForTests();
	});

	it("two owners in different projects never share authority: scoped seams win, projects stay pinned", async () => {
		const registryA = new AgentRegistry();
		registerLive(registryA, { id: "Main", uuid: "uuid-a-main" });
		const registryB = new AgentRegistry();
		registerLive(registryB, { id: "Main", uuid: "uuid-b-main" });

		const globalCalls: OpenManagedSessionInput[] = [];
		setSessionToolDeps({
			openSession: async input => {
				globalCalls.push(input);
				return {
					id: "uuid-global-child",
					registryId: "tab:uuid-global-child",
					taskAccepted: false,
					session: fakeLive("uuid-global-child"),
				};
			},
		});
		const scopedCallsB: OpenManagedSessionInput[] = [];
		const ownerB = new SessionsTool(
			makeToolSession("/proj-b", "uuid-b-main", "Main", registryB, {
				openManagedSession: async (input: OpenManagedSessionInput) => {
					scopedCallsB.push(input);
					const live = fakeLive("uuid-b-child");
					registryB.register({
						id: "tab:uuid-b-child",
						displayName: "tab:uuid-b-child",
						kind: "sub",
						parentId: input.callerId ?? undefined,
						session: live as unknown as AgentSession,
						status: "idle",
					});
					return { id: "uuid-b-child", registryId: "tab:uuid-b-child", taskAccepted: true, session: live };
				},
			}),
		);
		// Owner B binds to its own owner/session: scoped seam wins, cwd pinned.
		const created = await ownerB.execute("1", { action: "create", task: "work" });
		expect((created.details as { id: string }).id).toBe("uuid-b-child");
		expect(globalCalls).toHaveLength(0);
		expect(scopedCallsB).toHaveLength(1);
		expect(scopedCallsB[0]?.cwd).toBe("/proj-b");
		expect(scopedCallsB[0]?.background).toBe(true);
		expect(scopedCallsB[0]?.taskDepth).toBe(1);

		// Owner A has no scoped seam: the explicitly-supported global adapter serves it.
		const ownerA = new SessionsTool(makeToolSession("/proj-a", "uuid-a-main", "Main", registryA));
		const createdA = await ownerA.execute("1", { action: "create" });
		expect((createdA.details as { id: string }).id).toBe("uuid-global-child");
		expect(globalCalls).toHaveLength(1);

		// No cross-authority: B cannot touch A's registry, and A's child is not in B's lineage.
		await expect(
			ownerB.execute("2", { action: "rename", sessionId: "uuid-global-child", title: "X" }),
		).rejects.toThrow(/not live/);
		// B manages its own descendant exactly once per call.
		await ownerB.execute("3", { action: "inspect", sessionId: "uuid-b-child" });
		await ownerB.execute("4", { action: "rename", sessionId: "uuid-b-child", title: "B child" });
	});

	it("creation is serialized: concurrent creates dispatch once each and never oversubscribe", async () => {
		const registry = new AgentRegistry();
		registerLive(registry, { id: "Main", uuid: "uuid-main" });
		let calls = 0;
		setSessionToolDeps({
			openSession: async () => {
				calls++;
				const id = `uuid-child-${calls}`;
				return { id, registryId: `tab:${id}`, taskAccepted: true, session: fakeLive(id) };
			},
		});
		const tool = new SessionsTool(makeToolSession("/proj", "uuid-main", "Main", registry));
		const [first, second] = await Promise.all([
			tool.execute("1", { action: "create", task: "one" }),
			tool.execute("2", { action: "create", task: "two" }),
		]);
		expect(calls).toBe(2);
		expect((first.details as { taskAccepted: boolean }).taskAccepted).toBe(true);
		expect((second.details as { taskAccepted: boolean }).taskAccepted).toBe(true);
		expect((first.details as { id: string }).id).not.toBe((second.details as { id: string }).id);
	});

	it("a background caller begets background descendants it can manage", async () => {
		const registry = new AgentRegistry();
		registerLive(registry, { id: "Main", uuid: "uuid-main" });
		registerLive(registry, { id: "tab:uuid-bg", uuid: "uuid-bg", parentId: "Main" });
		setSessionToolDeps({
			openSession: async input => {
				const id = "uuid-grand";
				const live = fakeLive(id);
				registry.register({
					id: "tab:uuid-grand",
					displayName: "tab:uuid-grand",
					kind: "sub",
					parentId: input.callerId ?? undefined,
					session: live as unknown as AgentSession,
					status: "idle",
				});
				return { id, registryId: "tab:uuid-grand", taskAccepted: true, session: live };
			},
		});
		const backgroundCaller = new SessionsTool(makeToolSession("/proj", "uuid-bg", "tab:uuid-bg", registry));
		const created = await backgroundCaller.execute("1", { action: "create", task: "descend" });
		expect((created.details as { id: string }).id).toBe("uuid-grand");
		const grandchild = registry.list().find(ref => ref.id === "tab:uuid-grand");
		expect(grandchild?.parentId).toBe("tab:uuid-bg");
		await backgroundCaller.execute("2", { action: "rename", sessionId: "uuid-grand", title: "G" });
		await backgroundCaller.execute("3", { action: "send", sessionId: "uuid-grand", message: "go" });
	});

	it("headless-unavailable paths report truthfully instead of inventing runtimes", async () => {
		const registry = new AgentRegistry();
		registerLive(registry, { id: "Main", uuid: "uuid-main" });
		const tool = new SessionsTool(makeToolSession("/proj", "uuid-main", "Main", registry));
		await expect(tool.execute("1", { action: "create" })).rejects.toThrow(/not wired/);

		// Delivery without a seam or prompt entry is unwired, not silent.
		const muteRegistry = new AgentRegistry();
		muteRegistry.register({
			id: "Main",
			displayName: "Main",
			kind: "main",
			session: fakeLive("uuid-main") as unknown as AgentSession,
			status: "idle",
		});
		registerLive(muteRegistry, { id: "tab:uuid-mute", uuid: "uuid-mute", parentId: "Main" }, false);
		const muteTool = new SessionsTool(makeToolSession("/proj", "uuid-main", "Main", muteRegistry));
		await expect(muteTool.execute("2", { action: "send", sessionId: "uuid-mute", message: "hi" })).rejects.toThrow(
			/cannot receive messages/,
		);
	});

	it("session-scoped delivery wins over the global adapter", async () => {
		const registry = new AgentRegistry();
		const child = registerLive(registry, { id: "tab:uuid-child", uuid: "uuid-child", parentId: "Main" });
		registerLive(registry, { id: "Main", uuid: "uuid-main" });
		const scopedReceipts: string[] = [];
		const globalReceipts: string[] = [];
		setSessionToolDeps({
			deliverMessage: async () => {
				globalReceipts.push("global");
				return "global";
			},
		});
		const tool = new SessionsTool(
			makeToolSession("/proj", "uuid-main", "Main", registry, {
				deliverManagedMessage: async (target: ManagedLiveSessionLike, message: string) => {
					scopedReceipts.push(message);
					expect(target).toBe(child);
					return "scoped";
				},
			}),
		);
		const result = await tool.execute("1", { action: "send", sessionId: "uuid-child", message: "hello" });
		expect((result.content[0] as { text: string }).text ?? "").toContain("scoped");
		expect(scopedReceipts).toEqual(["hello"]);
		expect(globalReceipts).toHaveLength(0);
	});
});
