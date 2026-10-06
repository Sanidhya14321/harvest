import { afterEach, describe, expect, it } from "bun:test";
import { type SettingPath, Settings } from "@harvest/pi-coding-agent/config/settings";
import { AgentRegistry } from "@harvest/pi-coding-agent/registry/agent-registry";
import type { AgentSession } from "@harvest/pi-coding-agent/session/agent-session";
import type { ToolSession } from "@harvest/pi-coding-agent/tools";
import {
	clearSessionToolDepsForTests,
	setSessionToolDeps,
	SessionsTool,
	type ManagedLiveSessionLike,
} from "@harvest/pi-coding-agent/tools/sessions";

function fakeLive(overrides: Partial<ManagedLiveSessionLike> & { id: string }): ManagedLiveSessionLike {
	const calls: string[] = [];
	const live: ManagedLiveSessionLike = {
		getSessionId: () => overrides.id,
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
		...overrides,
	};
	(live as unknown as { calls: string[] }).calls = calls;
	return live;
}

function liveCalls(live: ManagedLiveSessionLike): string[] {
	return (live as unknown as { calls: string[] }).calls;
}

function makeSession(
	settingsOverrides: Partial<Record<SettingPath, unknown>>,
	extra: Partial<ToolSession> & { registry: AgentRegistry },
): ToolSession {
	return {
		cwd: "/proj",
		hasUI: false,
		skipPythonPreflight: true,
		getSessionFile: () => null,
		getSessionSpawns: () => "*",
		taskDepth: 0,
		getSessionId: () => "caller",
		settings: Settings.isolated({ "autolearn.enabled": true, ...settingsOverrides }),
		agentRegistry: extra.registry,
	};
}

describe("managed sessions tool", () => {
	afterEach(() => {
		clearSessionToolDepsForTests();
	});

	function setup(
		settingsOverrides: Partial<Record<SettingPath, unknown>> = {},
		refs: Array<{ id: string; parentId?: string; status?: "running" | "idle"; live?: boolean }> = [],
	): { tool: SessionsTool; registry: AgentRegistry; lives: Map<string, ManagedLiveSessionLike> } {
		const registry = new AgentRegistry();
		const lives = new Map<string, ManagedLiveSessionLike>();
		for (const ref of refs) {
			const live = ref.live === false ? null : fakeLive({ id: ref.id });
			if (live) lives.set(ref.id, live);
			registry.register({
				id: ref.id,
				displayName: ref.id,
				kind: "sub",
				parentId: ref.parentId,
				session: live as unknown as AgentSession | null,
				status: ref.status ?? "idle",
			});
		}
		const tool = new SessionsTool(makeSession(settingsOverrides, { registry }));
		return { tool, registry, lives };
	}

	it("classifies list/inspect as read and mutations as exec", () => {
		const { tool } = setup();
		expect(tool.approval({ action: "list" })).toBe("read");
		expect(tool.approval({ action: "inspect" })).toBe("read");
		expect(tool.approval({ action: "create" })).toBe("exec");
		expect(tool.approval({ action: "send" })).toBe("exec");
		expect(tool.approval({ action: "stop" })).toBe("exec");
		expect(tool.approval({ action: "rename" })).toBe("exec");
	});

	it("lists and inspects live sessions", async () => {
		const { tool } = setup({}, [
			{ id: "caller", status: "idle" },
			{ id: "child-1", parentId: "caller", status: "running" },
		]);
		const listed = await tool.execute("1", { action: "list" });
		expect(JSON.stringify(listed.content)).toContain("child-1");
		const inspected = await tool.execute("2", { action: "inspect", sessionId: "child-1" });
		expect((inspected.details as { status: string }).status).toBe("running");
		await expect(tool.execute("3", { action: "inspect", sessionId: "ghost" })).rejects.toThrow(/not live/);
	});

	it("enforces task.maxConcurrency on create", async () => {
		const { tool } = setup({ "task.maxConcurrency": 1 }, [{ id: "busy", status: "running" }]);
		setSessionToolDeps({
			openSession: async () => ({
				id: "never",
				registryId: "tab:never",
				taskAccepted: false,
				session: fakeLive({ id: "never" }),
			}),
		});
		await expect(tool.execute("1", { action: "create" })).rejects.toThrow(/maxConcurrency/);
	});

	it("enforces the spawn policy and recursion depth on create", async () => {
		const registry = new AgentRegistry();
		const denied = new SessionsTool({
			...makeSession({}, { registry }),
			getSessionSpawns: () => "",
		});
		setSessionToolDeps({
			openSession: async () => ({
				id: "never",
				registryId: "tab:never",
				taskAccepted: false,
				session: fakeLive({ id: "never" }),
			}),
		});
		await expect(denied.execute("1", { action: "create" })).rejects.toThrow(/disabled/);

		const deep = new SessionsTool({
			...makeSession({ "task.maxRecursionDepth": 2 }, { registry }),
			taskDepth: 3,
		});
		await expect(deep.execute("2", { action: "create" })).rejects.toThrow(/maxRecursionDepth/);
	});

	it("creates background sessions in the caller project without stealing focus", async () => {
		const { tool } = setup({}, [{ id: "caller", status: "idle" }]);
		setSessionToolDeps({
			openSession: async input => {
				expect(input.cwd).toBe("/proj");
				expect(input.callerId).toBe("caller");
				return {
					id: "uuid-fresh",
					registryId: "tab:uuid-fresh",
					taskAccepted: true,
					session: fakeLive({ id: "uuid-fresh" }),
				};
			},
		});
		const result = await tool.execute("1", { action: "create", task: "do work" });
		expect((result.details as { background: boolean }).background).toBe(true);
		expect((result.details as { cwd: string }).cwd).toBe("/proj");
		expect((result.details as { id: string }).id).toBe("uuid-fresh");
		expect((result.details as { registryId: string }).registryId).toBe("tab:uuid-fresh");
		expect((result.details as { taskAccepted: boolean }).taskAccepted).toBe(true);
	});

	it("refuses to create when no live-session factory is wired", async () => {
		const { tool } = setup();
		await expect(tool.execute("1", { action: "create" })).rejects.toThrow(/not wired/);
	});

	it("refuses self-stop so the call cannot deadlock", async () => {
		const { tool, lives } = setup({}, [{ id: "caller", status: "running" }]);
		await expect(tool.execute("1", { action: "stop", sessionId: "caller" })).rejects.toThrow(/itself/);
		expect(liveCalls(lives.get("caller")!)).not.toContain("abort");
	});

	it("requires explicit confirmation to stop a running session", async () => {
		const { tool, lives } = setup({}, [{ id: "child-1", parentId: "caller", status: "running" }]);
		await expect(tool.execute("1", { action: "stop", sessionId: "child-1" })).rejects.toThrow(/confirm:true/);
		expect(liveCalls(lives.get("child-1")!)).not.toContain("abort");
		await tool.execute("2", { action: "stop", sessionId: "child-1", confirm: true });
		expect(liveCalls(lives.get("child-1")!)).toContain("abort");
	});

	it("rejects foreign lineages without a host grant; forged model flags never authorize", async () => {
		const { tool, lives } = setup({}, [{ id: "foreign", parentId: "someone-else", status: "idle" }]);
		await expect(tool.execute("1", { action: "stop", sessionId: "foreign" })).rejects.toThrow(
			/host-provided grant or the existing approval path/,
		);
		// A model-supplied scopeGrant field is not part of the schema and is ignored.
		await expect(
			tool.execute("2", {
				action: "stop",
				sessionId: "foreign",
				scopeGrant: true,
			} as unknown as { action: "stop"; sessionId: string }),
		).rejects.toThrow();
		expect(liveCalls(lives.get("foreign")!)).not.toContain("abort");
	});

	it("allows foreign targets with a live host-provided grant", async () => {
		const { registry, lives } = setup({}, [{ id: "foreign", parentId: "someone-else", status: "idle" }]);
		const granted = new SessionsTool({
			...makeSession({}, { registry }),
			managedSessionGrant: { scope: "sessions:manage", expiresAt: Date.now() + 60_000 },
		});
		await granted.execute("1", { action: "stop", sessionId: "foreign" });
		expect(liveCalls(lives.get("foreign")!)).toContain("abort");
	});

	it("resolves public UUIDs to registry refs without invented aliases", async () => {
		const registry = new AgentRegistry();
		const live = fakeLive({ id: "uuid-1" });
		(live as unknown as { sessionManager: { getSessionId: () => string } }).sessionManager = {
			getSessionId: () => "uuid-1",
		};
		registry.register({
			id: "tab:uuid-1",
			displayName: "tab:uuid-1",
			kind: "sub",
			parentId: "caller",
			session: live as unknown as AgentSession,
			status: "idle",
		});
		const tool = new SessionsTool(makeSession({}, { registry }));
		const inspected = await tool.execute("1", { action: "inspect", sessionId: "uuid-1" });
		expect((inspected.details as { id: string }).id).toBe("tab:uuid-1");
		await tool.execute("2", { action: "rename", sessionId: "uuid-1", title: "Worker" });
		expect(liveCalls(live)).toContain("rename");
	});

	it("renames and messages owned children", async () => {
		const { tool, lives } = setup({}, [{ id: "child-1", parentId: "caller", status: "idle" }]);
		await tool.execute("1", { action: "rename", sessionId: "child-1", title: "Worker" });
		expect(liveCalls(lives.get("child-1")!)).toContain("rename");
		await tool.execute("2", { action: "send", sessionId: "child-1", message: "status?" });
		expect(liveCalls(lives.get("child-1")!)).toContain("prompt");
	});

	it("is gated behind autolearn.enabled", () => {
		const registry = new AgentRegistry();
		const off = SessionsTool.createIf(makeSession({ "autolearn.enabled": false }, { registry }));
		expect(off).toBeNull();
		const on = SessionsTool.createIf(makeSession({ "autolearn.enabled": true }, { registry }));
		expect(on).not.toBeNull();
	});
});
