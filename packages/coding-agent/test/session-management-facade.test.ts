import { afterEach, describe, expect, it, vi } from "bun:test";
import * as fs from "node:fs/promises";
import * as os from "node:os";
import * as path from "node:path";
import type { AgentSession } from "../src/session/agent-session";
import type { AgentSessionEvent } from "../src/session/agent-session-events";
import { LiveSessionRegistry } from "../src/session/live-session-registry";
import { SessionManagementFacade } from "../src/session/session-management-facade";
import { SessionTabs } from "../src/session/session-tabs";
import type { SessionStorage } from "../src/session/session-storage";
import { SessionViewStateStore } from "../src/session/session-view-state";

interface Stub {
	id: string;
	file: string;
	session: AgentSession;
	abort: ReturnType<typeof vi.fn>;
	dispose: ReturnType<typeof vi.fn>;
	setSessionName: ReturnType<typeof vi.fn>;
	seal: ReturnType<typeof vi.fn>;
	setStreaming: (value: boolean) => void;
}

function makeStub(id: string, file: string, cwd: string): Stub {
	const abort = vi.fn(async () => {});
	const dispose = vi.fn(async () => {});
	const setSessionName = vi.fn(async () => true);
	const seal = vi.fn(() => {});
	let streaming = false;
	const listeners = new Set<(event: AgentSessionEvent) => void>();
	const session = {
		get isStreaming() {
			return streaming;
		},
		isBashRunning: false,
		isEvalRunning: false,
		isCompacting: false,
		hasPendingAsyncWork: () => false,
		sessionManager: {
			getSessionId: () => id,
			getSessionFile: () => file,
			getSessionName: () => `Task ${id}`,
			getCwd: () => cwd,
			setSessionName,
			onSessionNameChanged: () => () => {},
			seal,
		},
		subscribe: (listener: (event: AgentSessionEvent) => void) => {
			listeners.add(listener);
			return () => listeners.delete(listener);
		},
		setSessionName,
		dispose,
		abort,
	} as unknown as AgentSession;
	return { id, file, session, abort, dispose, setSessionName, seal, setStreaming: v => (streaming = v) };
}

describe("SessionManagementFacade", () => {
	const dirs: string[] = [];
	afterEach(async () => {
		vi.restoreAllMocks();
		await Promise.all(dirs.splice(0).map(dir => fs.rm(dir, { recursive: true, force: true })));
	});

	async function makeFacade(ids: string[]) {
		const root = await fs.mkdtemp(path.join(os.tmpdir(), "harvest-facade-"));
		dirs.push(root);
		const sessionDir = path.join(root, "sessions");
		await fs.mkdir(sessionDir, { recursive: true });
		const stubs = ids.map(id => makeStub(id, path.join(sessionDir, `${id}.jsonl`), root));
		const registry = new LiveSessionRegistry(stubs[0]!.session, async () => {
			throw new Error("no cold opens in facade tests");
		});
		for (const stub of stubs.slice(1)) registry.adopt(stub.session);
		const tabs = new SessionTabs();
		for (const stub of stubs) {
			tabs.open(stub.file, `Task ${stub.id}`);
			tabs.noteId(stub.file, stub.id);
		}
		const deleted: string[] = [];
		const storage = {
			deleteSessionWithArtifacts: vi.fn(async (file: string) => {
				deleted.push(file);
			}),
			readText: vi.fn(async () => {
				throw Object.assign(new Error("missing"), { code: "ENOENT" });
			}),
			writeTextAtomic: vi.fn(async () => {}),
			unlink: vi.fn(async () => {}),
			drain: vi.fn(async () => {}),
		} as unknown as SessionStorage;
		const facade = new SessionManagementFacade({
			live: registry,
			tabs,
			viewState: new SessionViewStateStore(),
			storage,
			createSession: async () => {
				throw new Error("no creation in facade tests");
			},
			openSession: async () => {
				throw new Error("no cold opens in facade tests");
			},
			directories: { cwd: root, sessionDir },
			legacyAgentDir: path.join(root, "agent"),
		});
		return { facade, stubs, tabs, registry, deleted, storage };
	}

	it("close hides the view and preserves the runtime", async () => {
		const { facade, stubs, tabs, registry } = await makeFacade(["a", "b"]);
		await facade.close("b");
		expect(tabs.paths).not.toContain(stubs[1]!.file);
		expect(registry.snapshots.some(snapshot => snapshot.id === "b")).toBe(true);
		expect(stubs[1]!.abort).not.toHaveBeenCalled();
		expect(stubs[1]!.dispose).not.toHaveBeenCalled();
	});

	it("close reports the last tab so the owner can display Home", async () => {
		const { facade } = await makeFacade(["a"]);
		const result = await facade.close("a");
		expect(result).toEqual({ lastTab: true });
	});

	it("stop aborts only the targeted run", async () => {
		const { facade, stubs } = await makeFacade(["a", "b"]);
		await facade.stop("b");
		expect(stubs[1]!.abort).toHaveBeenCalledTimes(1);
		expect(stubs[0]!.abort).not.toHaveBeenCalled();
	});

	it("delete rejects busy targets without stop-and-delete", async () => {
		const { facade, stubs, deleted } = await makeFacade(["a", "b"]);
		stubs[1]!.setStreaming(true);
		await expect(facade.delete("b")).rejects.toThrow(/busy/);
		expect(deleted).toEqual([]);
		expect(facade.isDeleted("b")).toBe(false);
	});

	it("stop-and-delete seals the writer, deletes, tombstones, and releases the runtime", async () => {
		const { facade, stubs, tabs, registry, deleted } = await makeFacade(["a", "b"]);
		stubs[1]!.setStreaming(true);
		stubs[1]!.abort.mockImplementationOnce(async () => stubs[1]!.setStreaming(false));
		await facade.delete("b", { stopFirst: true });
		expect(stubs[1]!.abort).toHaveBeenCalledTimes(1);
		expect(stubs[1]!.seal).toHaveBeenCalledTimes(1);
		expect(deleted).toEqual([stubs[1]!.file]);
		expect(facade.isDeleted("b")).toBe(true);
		expect(tabs.paths).not.toContain(stubs[1]!.file);
		expect(stubs[1]!.dispose).toHaveBeenCalledTimes(1);
		expect(registry.snapshots.some(snapshot => snapshot.id === "b")).toBe(false);
		expect(stubs[0]!.abort).not.toHaveBeenCalled();
		expect(stubs[0]!.dispose).not.toHaveBeenCalled();
	});

	it("stop-and-delete rejects when the run never settles", async () => {
		const { facade, stubs, deleted } = await makeFacade(["a", "b"]);
		stubs[1]!.setStreaming(true);
		await expect(facade.delete("b", { stopFirst: true })).rejects.toThrow(/did not settle/);
		expect(deleted).toEqual([]);
		expect(stubs[1]!.seal).not.toHaveBeenCalled();
		expect(facade.isDeleted("b")).toBe(false);
	});

	it("archive requires idle, persists metadata via storage, closes tabs, restores identity", async () => {
		const { facade, stubs, tabs, storage } = await makeFacade(["a", "b"]);
		stubs[1]!.setStreaming(true);
		await expect(facade.archive("b")).rejects.toThrow(/busy/);
		stubs[1]!.setStreaming(false);
		await facade.archive("b");
		expect(tabs.paths).not.toContain(stubs[1]!.file);
		expect(facade.inspect("b")?.archived).toBe(true);
		const writeTextAtomic = storage.writeTextAtomic as unknown as ReturnType<typeof vi.fn>;
		expect(writeTextAtomic).toHaveBeenCalled();
		const written = writeTextAtomic.mock.calls[0]![1] as string;
		expect(JSON.parse(written)).toEqual({ version: 1, ids: ["b"] });
		await facade.restore("b");
		expect(facade.inspect("b")?.archived).toBe(false);
	});

	it("rename and inspect use stable session IDs", async () => {
		const { facade, stubs } = await makeFacade(["a"]);
		await facade.rename("a", "My Title");
		expect(stubs[0]!.setSessionName).toHaveBeenCalledWith("My Title", "user");
		expect(facade.inspect("a")?.id).toBe("a");
		expect(facade.inspect("missing")).toBeUndefined();
	});

	it("select resolves warm runtimes without disk reads", async () => {
		const { facade } = await makeFacade(["a", "b"]);
		const result = await facade.select("b");
		expect(result).toEqual({ switched: true });
	});

	it("reopen restores the exact requested session", async () => {
		const { facade, stubs } = await makeFacade(["a", "b"]);
		await facade.close("b");
		const result = await facade.reopen("b");
		expect(result).toEqual({ switched: true });
		expect(stubs[1]!.file).toContain("b.jsonl");
	});

	it("serializeTabs carries stable IDs for restart restore", async () => {
		const { facade } = await makeFacade(["a", "b"]);
		const snapshot = (await facade.serializeTabs("proj", { sessionId: "a" })) as {
			version: number;
			tabs: { path: string; sessionId?: string }[];
		};
		expect(snapshot.version).toBe(2);
		expect(snapshot.tabs.map(tab => tab.sessionId).sort()).toEqual(["a", "b"]);
	});
});
