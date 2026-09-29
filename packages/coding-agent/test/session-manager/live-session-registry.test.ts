import { describe, expect, it, mock } from "bun:test";
import type { AgentSession } from "../../src/session/agent-session";
import type { AgentSessionEvent } from "../../src/session/agent-session-events";
import { LiveSessionRegistry } from "../../src/session/live-session-registry";

function fakeSession(id: string, path: string, cwd = "C:/project") {
	const listeners = new Set<(event: AgentSessionEvent) => void>();
	const titleListeners = new Set<() => void>();
	let name = "Untitled";
	const dispose = mock(async () => {});
	const abort = mock(async () => {});
	const session = {
		isStreaming: false,
		isBashRunning: false,
		isEvalRunning: false,
		hasPendingAsyncWork: () => false,
		sessionManager: {
			getSessionId: () => id,
			getSessionFile: () => path,
			getSessionName: () => name,
			getCwd: () => cwd,
			onSessionNameChanged: (listener: () => void) => {
				titleListeners.add(listener);
				return () => titleListeners.delete(listener);
			},
		},
		subscribe: (listener: (event: AgentSessionEvent) => void) => {
			listeners.add(listener);
			return () => listeners.delete(listener);
		},
		dispose,
		abort,
	} as unknown as AgentSession;
	return {
		session,
		dispose,
		abort,
		emit: (event: AgentSessionEvent) => {
			for (const listener of listeners) listener(event);
		},
		rename: (title: string) => {
			name = title;
			for (const listener of titleListeners) listener();
		},
	};
}

describe("LiveSessionRegistry", () => {
	it("switches warm sessions without opening files or disposing the previous run", async () => {
		const first = fakeSession("one", "C:/project/one.jsonl");
		const second = fakeSession("two", "C:/project/two.jsonl");
		const open = mock(async () => second.session);
		const registry = new LiveSessionRegistry(first.session, open);
		first.emit({ type: "agent_start" });
		await registry.select("C:/project/two.jsonl");
		await registry.select("C:/project/one.jsonl");
		await registry.select("C:/project/two.jsonl");
		expect(open).toHaveBeenCalledTimes(1);
		expect(first.dispose).not.toHaveBeenCalled();
		expect(registry.snapshots.find(item => item.id === "one")?.status).toBe("running");
		await registry.dispose();
	});

	it("deduplicates concurrent cold opens and preserves the selected runtime on failure", async () => {
		const first = fakeSession("one", "C:/project/one.jsonl");
		const second = fakeSession("two", "C:/project/two.jsonl");
		let release!: (session: AgentSession) => void;
		const open = mock(
			() =>
				new Promise<AgentSession>(resolve => {
					release = resolve;
				}),
		);
		const registry = new LiveSessionRegistry(first.session, open);
		const a = registry.select("C:/project/two.jsonl");
		const b = registry.select("C:/project/two.jsonl");
		expect(open).toHaveBeenCalledTimes(1);
		release(second.session);
		await Promise.all([a, b]);
		expect(registry.snapshots.filter(item => item.selected).map(item => item.id)).toEqual(["two"]);
		await registry.dispose();
	});

	it("marks a background completion unread and clears it when selected", async () => {
		const first = fakeSession("one", "C:/project/one.jsonl");
		const second = fakeSession("two", "C:/project/two.jsonl");
		const registry = new LiveSessionRegistry(first.session, async () => second.session);
		await registry.select("C:/project/two.jsonl");
		first.emit({ type: "agent_start" });
		first.emit({ type: "agent_end", messages: [] });
		expect(registry.snapshots.find(item => item.id === "one")).toMatchObject({ status: "completed", unread: true });
		await registry.select("C:/project/one.jsonl");
		expect(registry.snapshots.find(item => item.id === "one")).toMatchObject({ status: "idle", unread: false });
		await registry.dispose();
	});

	it("keeps a hidden run alive until an explicit stop targets its session", async () => {
		const first = fakeSession("one", "C:/project/one.jsonl");
		const second = fakeSession("two", "C:/project/two.jsonl");
		const registry = new LiveSessionRegistry(first.session, async () => second.session);
		first.emit({ type: "agent_start" });
		await registry.select("C:/project/two.jsonl");
		expect(registry.busySessions.map(snapshot => snapshot.id)).toEqual(["one"]);
		expect(first.abort).not.toHaveBeenCalled();
		await registry.stop("one");
		expect(first.abort).toHaveBeenCalledTimes(1);
		expect(second.abort).not.toHaveBeenCalled();
		await registry.dispose();
	});

	it("publishes a renamed background session without selecting it", async () => {
		const first = fakeSession("one", "C:/project/one.jsonl");
		const second = fakeSession("two", "C:/project/two.jsonl");
		const registry = new LiveSessionRegistry(first.session, async () => second.session);
		await registry.select("C:/project/two.jsonl");
		let changes = 0;
		registry.onChange(() => changes++);
		first.rename("Investigate parser");
		expect(registry.snapshots.find(item => item.id === "one")?.title).toBe("Investigate parser");
		expect(changes).toBe(1);
		await registry.dispose();
	});

	it("rejects cross-project live sessions without replacing the selected one", async () => {
		const first = fakeSession("one", "C:/project/one.jsonl");
		const other = fakeSession("two", "C:/other/two.jsonl", "C:/other");
		const registry = new LiveSessionRegistry(first.session, async () => other.session);
		await expect(registry.select("C:/other/two.jsonl")).rejects.toThrow("current project");
		expect(registry.selected).toBe(first.session);
		await registry.dispose();
	});

	it("does not let a slow earlier open steal focus from a later tab click", async () => {
		const first = fakeSession("one", "C:/project/one.jsonl");
		const slow = fakeSession("slow", "C:/project/slow.jsonl");
		let release!: (session: AgentSession) => void;
		const registry = new LiveSessionRegistry(
			first.session,
			async () =>
				new Promise(resolve => {
					release = resolve;
				}),
		);
		const pending = registry.select("C:/project/slow.jsonl");
		await registry.select("C:/project/one.jsonl");
		release(slow.session);
		expect(await pending).toMatchObject({ session: slow.session, selected: false });
		expect(registry.selected).toBe(first.session);
		await registry.dispose();
	});

	it("closes a cold runtime that finishes opening after shutdown", async () => {
		const first = fakeSession("one", "C:/project/one.jsonl");
		const second = fakeSession("two", "C:/project/two.jsonl");
		let release!: (session: AgentSession) => void;
		const registry = new LiveSessionRegistry(
			first.session,
			() =>
				new Promise(resolve => {
					release = resolve;
				}),
		);
		const pending = registry.select("C:/project/two.jsonl");
		const closing = registry.dispose();
		release(second.session);
		await expect(pending).rejects.toThrow("closed");
		await closing;
		expect(first.dispose).toHaveBeenCalledTimes(1);
		expect(second.dispose).toHaveBeenCalledTimes(1);
	});

	it("releases the oldest idle runtime while keeping a running hidden session", async () => {
		const sessions = Array.from({ length: 8 }, (_, index) =>
			fakeSession(`session-${index}`, `C:/project/${index}.jsonl`),
		);
		const first = sessions[0]!;
		const registry = new LiveSessionRegistry(first.session, async path => {
			const match = sessions.find(item => item.session.sessionManager.getSessionFile() === path);
			if (!match) throw new Error("missing fixture");
			return match.session;
		});
		first.emit({ type: "agent_start" });
		for (const item of sessions.slice(1)) await registry.select(item.session.sessionManager.getSessionFile()!);
		sessions[1]!.emit({ type: "agent_start" });
		sessions[1]!.emit({ type: "agent_end", messages: [] });
		await registry.releaseIdleRuntimes();
		expect(first.dispose).not.toHaveBeenCalled();
		expect(sessions[1]!.dispose).toHaveBeenCalledTimes(1);
		expect(registry.snapshots.map(item => item.id)).toContain("session-0");
		expect(registry.snapshots.find(item => item.id === "session-1")).toMatchObject({
			status: "completed",
			unread: true,
		});
		expect(registry.sessions).not.toContain(sessions[1]!.session);
		await registry.dispose();
	});
});
