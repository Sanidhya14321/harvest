import { afterEach, describe, expect, it, vi } from "bun:test";
import * as fs from "node:fs/promises";
import * as os from "node:os";
import * as path from "node:path";
import { SelectorController } from "../../../src/modes/controllers/selector-controller";
import type { InteractiveModeContext } from "../../../src/modes/types";
import type { AgentSession, AgentSessionEvent } from "../../../src/session/agent-session";
import { LiveSessionRegistry } from "../../../src/session/live-session-registry";

interface TabStub {
	session: AgentSession;
	id: string;
	file: string;
	name: string;
	abort: ReturnType<typeof vi.fn>;
	dispose: ReturnType<typeof vi.fn>;
	emit: (event: AgentSessionEvent) => void;
}

function sessionFileLine(id: string, cwd: string): string {
	return `${JSON.stringify({ type: "session", id, timestamp: new Date().toISOString(), cwd })}\n`;
}

interface Harness {
	ctx: InteractiveModeContext;
	controller: SelectorController;
	registry: LiveSessionRegistry;
	stubs: Map<string, TabStub>;
	bySession: Map<AgentSession, TabStub>;
	active: () => TabStub;
	errors: string[];
	openCalls: string[];
}

/**
 * Live-tab navigation contracts: switching tabs never stops the other run.
 * Warm tabs reattach without file reads, cold tabs open through the
 * registry, failures and cross-project targets preserve the current
 * session, and closing hides (never aborts).
 */
describe("live tab navigation", () => {
	const roots: string[] = [];
	let root = "";
	let active: TabStub | undefined;
	const allStubs = new Map<string, TabStub>();

	afterEach(async () => {
		vi.restoreAllMocks();
		active = undefined;
		allStubs.clear();
		await Promise.all(roots.splice(0).map(dir => fs.rm(dir, { recursive: true, force: true })));
	});

	function makeTabStub(id: string, file: string, cwd: string, name?: string): TabStub {
		const listeners = new Set<(event: AgentSessionEvent) => void>();
		const abort = vi.fn(async () => {});
		const dispose = vi.fn(async () => {});
		const stub: TabStub = {
			session: undefined as unknown as AgentSession,
			id,
			file,
			name: name ?? `Task ${id}`,
			abort,
			dispose,
			emit: event => {
				for (const listener of listeners) listener(event);
			},
		};
		stub.session = {
			isStreaming: false,
			isBashRunning: false,
			isEvalRunning: false,
			hasPendingAsyncWork: () => false,
			sessionManager: {
				getSessionId: () => stub.id,
				getSessionFile: () => stub.file,
				getSessionName: () => stub.name,
				getCwd: () => cwd,
				getSessionDir: () => root,
				onSessionNameChanged: () => () => {},
			},
			subscribe: (listener: (event: AgentSessionEvent) => void) => {
				listeners.add(listener);
				return () => listeners.delete(listener);
			},
			dispose,
			abort,
			switchSession: vi.fn(async (target: string) => {
				const next = allStubs.get(target);
				if (next) active = next;
				return true;
			}),
		} as unknown as AgentSession;
		allStubs.set(file, stub);
		return stub;
	}

	function makeHarness(initial: TabStub, openStub?: (path: string) => Promise<AgentSession>): Harness {
		const stubs = new Map<string, TabStub>();
		const bySession = new Map<AgentSession, TabStub>();
		const errors: string[] = [];
		const openCalls: string[] = [];
		active = initial;
		const sessionManager = {
			getSessionFile: () => active?.file,
			getSessionName: () => active?.name,
			getSessionId: () => active?.id,
			getCwd: () => root,
			getSessionDir: () => root,
		};
		const openSession = async (target: string) => {
			openCalls.push(target);
			if (openStub) return openStub(target);
			const stub = stubs.get(target);
			if (!stub) throw new Error(`no such session: ${target}`);
			return stub.session;
		};
		const ctx = {
			sessionManager,
			get session() {
				return active!.session;
			},
			settings: { flush: async () => {} },
			editor: { getText: () => "", setText: vi.fn() },
			ui: { requestRender: vi.fn() },
			showError: (message: string) => {
				errors.push(message);
			},
			showHookConfirm: vi.fn(async () => true),
			showStatus: vi.fn(),
			clearTransientSessionUi: vi.fn(),
			updateEditorBorderColor: vi.fn(),
			renderInitialMessages: vi.fn(async () => {}),
			reloadTodos: vi.fn(async () => {}),
			applyCwdChange: vi.fn(async () => true),
			selectMainSession: vi.fn(async (session: AgentSession) => {
				const stub = bySession.get(session);
				if (!stub) throw new Error("unknown session");
				active = stub;
			}),
			handleResumeSession: (target: string) => controller.handleResumeSession(target, { settingsFlushed: true }),
			clearHomeDetached: () => {},
			enterHomeDetached: () => {},
			isHomeDetached: () => false,
		} as unknown as InteractiveModeContext;
		const controller: SelectorController = new SelectorController(ctx);
		const registry = new LiveSessionRegistry(initial.session, openSession);
		(ctx as unknown as { liveSessions: LiveSessionRegistry }).liveSessions = registry;
		stubs.set(initial.file, initial);
		bySession.set(initial.session, initial);
		return {
			ctx,
			controller,
			registry,
			stubs,
			bySession,
			active: () => active!,
			errors,
			openCalls,
		};
	}

	async function writeFile(id: string, cwd: string): Promise<string> {
		const file = path.join(root, `${id}.jsonl`);
		await Bun.write(file, sessionFileLine(id, cwd));
		return file;
	}

	function register(harness: Harness, stub: TabStub): void {
		harness.stubs.set(stub.file, stub);
		harness.bySession.set(stub.session, stub);
	}

	it("switches to a warm tab without stopping the previous run", async () => {
		root = await fs.mkdtemp(path.join(os.tmpdir(), "harvest-live-nav-"));
		roots.push(root);
		const fileA = await writeFile("a", root);
		const fileB = await writeFile("b", root);
		const stubA = makeTabStub("a", fileA, root);
		const stubB = makeTabStub("b", fileB, root);
		const harness = makeHarness(stubA);
		register(harness, stubB);
		harness.registry.adopt(stubB.session);
		stubA.emit({ type: "agent_start" });

		expect(await harness.controller.handleResumeSession(fileB)).toBe(true);
		expect(stubA.abort).not.toHaveBeenCalled();
		expect(stubA.dispose).not.toHaveBeenCalled();
		expect(harness.active()).toBe(stubB);
		expect(harness.controller.sessionTabs.paths).toContain(fileA);
		expect(harness.controller.sessionTabs.paths).toContain(fileB);
		const persisted = (await Bun.file(path.join(root, "tabs.json")).json()) as { tabs: { path: string }[] };
		expect(persisted.tabs.map(tab => tab.path).sort()).toEqual([fileA, fileB].sort());
	});

	it("cold-opens through the registry without stopping the current run", async () => {
		root = await fs.mkdtemp(path.join(os.tmpdir(), "harvest-live-nav-"));
		roots.push(root);
		const fileA = await writeFile("a", root);
		const fileC = await writeFile("c", root);
		const stubA = makeTabStub("a", fileA, root);
		const stubC = makeTabStub("c", fileC, root);
		const opened: string[] = [];
		const harness = makeHarness(stubA, async target => {
			opened.push(target);
			const stub = harness.stubs.get(target);
			if (!stub) throw new Error(`no such session: ${target}`);
			return stub.session;
		});
		register(harness, stubC);
		stubA.emit({ type: "agent_start" });

		expect(await harness.controller.handleResumeSession(fileC)).toBe(true);
		expect(opened).toEqual([fileC]);
		expect(stubA.abort).not.toHaveBeenCalled();
		expect(harness.active()).toBe(stubC);
	});

	it("reattaches a warm session whose backing file disappeared without cold-opening it", async () => {
		root = await fs.mkdtemp(path.join(os.tmpdir(), "harvest-live-nav-"));
		roots.push(root);
		const fileA = await writeFile("a", root);
		const fileB = await writeFile("b", root);
		const stubA = makeTabStub("a", fileA, root);
		const stubB = makeTabStub("b", fileB, root);
		const harness = makeHarness(stubA);
		register(harness, stubB);
		harness.registry.adopt(stubB.session);
		await fs.unlink(fileB);
		expect(await harness.controller.handleResumeSession(fileB)).toBe(true);
		expect(harness.active()).toBe(stubB);
		expect(harness.openCalls).toEqual([]);
		expect(stubA.abort).not.toHaveBeenCalled();
	});

	it("rejects a corrupt cold target before the fallback can stop the current run", async () => {
		root = await fs.mkdtemp(path.join(os.tmpdir(), "harvest-live-nav-"));
		roots.push(root);
		const fileA = await writeFile("a", root);
		const broken = path.join(root, "broken.jsonl");
		await Bun.write(broken, "not a session\n");
		const stubA = makeTabStub("a", fileA, root);
		const harness = makeHarness(stubA);
		stubA.emit({ type: "agent_start" });
		expect(await harness.controller.handleResumeSession(broken)).toBe(false);
		expect(harness.active()).toBe(stubA);
		expect(stubA.session.switchSession).not.toHaveBeenCalled();
		expect(stubA.abort).not.toHaveBeenCalled();
		expect(harness.errors.join(" ")).toContain("header");
	});

	it("preserves the current session when a cold open fails", async () => {
		root = await fs.mkdtemp(path.join(os.tmpdir(), "harvest-live-nav-"));
		roots.push(root);
		const fileA = await writeFile("a", root);
		const fileC = await writeFile("c", root);
		const stubA = makeTabStub("a", fileA, root);
		const harness = makeHarness(stubA, async () => {
			throw new Error("extension blew up");
		});

		expect(await harness.controller.handleResumeSession(fileC)).toBe(false);
		expect(harness.active()).toBe(stubA);
		expect(stubA.abort).not.toHaveBeenCalled();
		expect(harness.errors.join("\n")).toContain("extension blew up");
	});

	it("falls back to the legacy path for cross-project targets", async () => {
		root = await fs.mkdtemp(path.join(os.tmpdir(), "harvest-live-nav-"));
		roots.push(root);
		const fileA = await writeFile("a", root);
		const fileD = await writeFile("d", "/other");
		const stubA = makeTabStub("a", fileA, root);
		const stubD = makeTabStub("d", fileD, "/other", "Other");
		const harness = makeHarness(stubA);
		register(harness, stubD);
		// The peek reads the recorded cwd (/other) and routes around the registry.
		expect(await harness.controller.handleResumeSession(fileD)).toBe(true);
		expect(stubA.session.switchSession).toHaveBeenCalled();
		expect(harness.active()).toBe(stubD);
	});

	it("closes the active tab without aborting its run and reopens it warm", async () => {
		root = await fs.mkdtemp(path.join(os.tmpdir(), "harvest-live-nav-"));
		roots.push(root);
		const fileA = await writeFile("a", root);
		const fileB = await writeFile("b", root);
		const stubA = makeTabStub("a", fileA, root);
		const stubB = makeTabStub("b", fileB, root);
		const harness = makeHarness(stubA);
		register(harness, stubB);
		harness.registry.adopt(stubB.session);
		harness.controller.sessionTabs.open(fileB, stubB.name);
		stubA.emit({ type: "agent_start" });

		expect(await harness.controller.handleSessionTabsCommand("close")).toContain("Closed session tab");
		expect(stubA.abort).not.toHaveBeenCalled();
		expect(harness.active()).toBe(stubB);
		expect(harness.controller.sessionTabs.paths).not.toContain(fileA);
		expect(await harness.controller.handleSessionTabsCommand("reopen")).toContain("Reopened session tab");
		expect(harness.active()).toBe(stubA);
		expect(stubA.abort).not.toHaveBeenCalled();
	});

	it("evicts the oldest idle runtime past six tabs while keeping the running one", async () => {
		root = await fs.mkdtemp(path.join(os.tmpdir(), "harvest-live-nav-"));
		roots.push(root);
		const files: string[] = [];
		for (let index = 0; index < 9; index++) files.push(await writeFile(`s${index}`, root));
		const stubs = files.map((file, index) => makeTabStub(`s${index}`, file, root));
		const harness = makeHarness(stubs[0]!);
		for (const stub of stubs.slice(1)) register(harness, stub);
		for (const stub of stubs.slice(1, 8)) harness.registry.adopt(stub.session);
		stubs[1]!.emit({ type: "agent_start" });

		expect(await harness.controller.handleResumeSession(files[8]!)).toBe(true);
		expect(harness.active().id).toBe("s8");
		expect(stubs[1]!.dispose).not.toHaveBeenCalled();
		expect(stubs[0]!.dispose).toHaveBeenCalledTimes(1);
		expect(harness.registry.sessions.map(session => session.sessionManager.getSessionId())).not.toContain("s0");
		expect(harness.registry.snapshotForPath(files[0]!)).toMatchObject({ id: "s0" });
	});

	it("refuses to delete a session with a live run", async () => {
		root = await fs.mkdtemp(path.join(os.tmpdir(), "harvest-live-nav-"));
		roots.push(root);
		const fileA = await writeFile("a", root);
		const stubA = makeTabStub("a", fileA, root);
		const harness = makeHarness(stubA);
		(stubA.session as unknown as { isStreaming: boolean }).isStreaming = true;

		await harness.controller.handleSessionDeleteCommand();
		expect(harness.errors.join("\n")).toContain("Stop the run first");
		const confirm = harness.ctx.showHookConfirm as unknown as ReturnType<typeof vi.fn>;
		expect(confirm).not.toHaveBeenCalled();
		expect(await Bun.file(fileA).exists()).toBe(true);
	});
});
