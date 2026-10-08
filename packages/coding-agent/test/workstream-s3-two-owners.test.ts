import { afterEach, describe, expect, it } from "bun:test";
import * as path from "node:path";
import { Agent } from "@harvest/pi-agent-core";
import { TempDir } from "@harvest/pi-utils";
import { VirtualTerminal } from "../../tui/test/virtual-terminal";
import { ModelRegistry } from "../src/config/model-registry";
import { resetSettingsForTest, Settings } from "../src/config/settings";
import { Composer } from "../src/modes/composer";
import { InteractiveMode } from "../src/modes/interactive-mode";
import { initTheme } from "../src/modes/theme/theme";
import { AgentSession } from "../src/session/agent-session";
import { AuthStorage } from "../src/session/auth-storage";
import { SessionManager } from "../src/session/session-manager";
import { AgentRegistry } from "../src/registry/agent-registry";
import { getToolSessionForAgentSession } from "../src/sdk";
import type { ToolSession } from "../src/tools/index";
import { clearSessionToolDepsForTests, SessionsTool } from "../src/tools/sessions";

/**
 * T04/S3: two real interactive owners with different projects isolate
 * creation. Each owner's factory serves its own cwd/registry/tabs through
 * the shared router; neither borrows the other's authority or callbacks,
 * initial tasks dispatch once without focus steal, descendants inherit the
 * correct scope, and unknown projects/callers fail truthfully.
 */
describe("two owners isolate model-created sessions", () => {
	const cleanups: Array<() => Promise<void>> = [];

	interface Owner {
		mode: InteractiveMode;
		terminal: VirtualTerminal;
		directory: TempDir;
		auth: AuthStorage;
		session: AgentSession;
		cwd: string;
	}

	async function bootOwner(tag: string): Promise<Owner> {
		const directory = TempDir.createSync(`@harvest-two-owners-${tag}-`);
		await Settings.init({ inMemory: true, cwd: directory.path() });
		Settings.instance.set("tui.fullscreen", true);
		await initTheme(false);
		const auth = await AuthStorage.create(path.join(directory.path(), "auth.db"));
		const registry = new ModelRegistry(auth);
		const model = registry.find("anthropic", "claude-sonnet-4-5");
		if (!model) throw new Error("Missing test model");
		const manager = SessionManager.create(directory.path(), directory.path());
		await manager.setSessionName(`Owner ${tag}`, "user");
		manager.appendMessage({ role: "user", content: `Owner ${tag} transcript`, timestamp: Date.now() });
		await manager.ensureOnDisk();
		const session = new AgentSession({
			agent: new Agent({ initialState: { model, systemPrompt: ["Test"], tools: [], messages: [] } }),
			sessionManager: manager,
			settings: Settings.isolated(),
			modelRegistry: registry,
		});
		const terminal = new VirtualTerminal(100, 30);
		const mode = new InteractiveMode(
			session,
			"test",
			undefined,
			() => {},
			undefined,
			undefined,
			undefined,
			new Composer({ terminal }),
		);
		await mode.init({ suppressWelcomeIntro: true });
		void mode.getUserInput();
		await terminal.waitForRender();
		const owner: Owner = { mode, terminal, directory, auth, session, cwd: directory.path() };
		cleanups.push(async () => {
			mode.stop();
			await mode.liveSessions?.dispose();
			await session.dispose();
			auth.close();
			await directory.remove();
		});
		return owner;
	}

	function callerTools(owner: Owner, callerId: string): { tool: SessionsTool; session: ToolSession } {
		const session = {
			cwd: owner.cwd,
			settings: Settings.isolated({ "autolearn.enabled": true }),
			getSessionId: () => callerId,
			getSessionSpawns: () => "*",
			taskDepth: 0,
			agentRegistry: AgentRegistry.global(),
		} as unknown as ToolSession;
		return { tool: new SessionsTool(session), session };
	}

	afterEach(async () => {
		for (const cleanup of cleanups.splice(0)) await cleanup().catch(() => {});
		clearSessionToolDepsForTests();
		resetSettingsForTest();
	});

	it("routes each creation to its own owner with its own authority", async () => {
		const ownerA = await bootOwner("a");
		const ownerB = await bootOwner("b");
		const uuidA = ownerA.session.sessionManager.getSessionId();
		const uuidB = ownerB.session.sessionManager.getSessionId();
		const fileA = ownerA.session.sessionManager.getSessionFile();
		const fileB = ownerB.session.sessionManager.getSessionFile();

		// Create a child through owner A's binding (global router → owner A).
		const { tool: toolA } = callerTools(ownerA, uuidA);
		const createdA = await toolA.execute("1", { action: "create" });
		const idA = (createdA.details as { id: string }).id;
		expect(typeof idA).toBe("string");
		expect((createdA.details as { registryId: string }).registryId).toBe(`tab:${idA}`);
		// Landed in A's registry/tabs only, with A's project.
		expect(ownerA.mode.liveSessions?.snapshots.some(s => s.id === idA)).toBe(true);
		expect(ownerB.mode.liveSessions?.snapshots.some(s => s.id === idA)).toBe(false);
		const childA = ownerA.mode.liveSessions?.sessions.find(s => s.sessionManager.getSessionId() === idA);
		if (!childA) throw new Error("Child A is not live in owner A");
		expect(childA.sessionManager.getCwd()).toBe(ownerA.cwd);
		// No focus steal on either owner.
		expect(ownerA.session.sessionManager.getSessionFile()).toBe(fileA);
		expect(ownerB.session.sessionManager.getSessionFile()).toBe(fileB);

		// Same through owner B: disjoint authority, callbacks, and registries.
		const { tool: toolB } = callerTools(ownerB, uuidB);
		const createdB = await toolB.execute("2", { action: "create" });
		const idB = (createdB.details as { id: string }).id;
		expect(ownerB.mode.liveSessions?.snapshots.some(s => s.id === idB)).toBe(true);
		expect(ownerA.mode.liveSessions?.snapshots.some(s => s.id === idB)).toBe(false);
		const childB = ownerB.mode.liveSessions?.sessions.find(s => s.sessionManager.getSessionId() === idB);
		if (!childB) throw new Error("Child B is not live in owner B");
		expect(childB.sessionManager.getCwd()).toBe(ownerB.cwd);

		// The returned UUIDs work immediately under the creator's lineage.
		const renamed = await toolA.execute("3", { action: "rename", sessionId: idA, title: "Worker A" });
		expect(JSON.stringify(renamed.content)).toContain("Worker A");
		await expect(toolB.execute("4", { action: "rename", sessionId: idA, title: "Hijack" })).rejects.toThrow();

		// Descendants inherit the correct scope through the session binding:
		// the child's own ToolSession creates a grandchild in owner A at depth 2.
		const childTools = getToolSessionForAgentSession(childA);
		if (!childTools) throw new Error("Child ToolSession is not registered");
		const grandchildTool = new SessionsTool(childTools);
		const grand = await grandchildTool.execute("5", { action: "create" });
		const idG = (grand.details as { id: string }).id;
		expect(ownerA.mode.liveSessions?.snapshots.some(s => s.id === idG)).toBe(true);
		expect(ownerB.mode.liveSessions?.snapshots.some(s => s.id === idG)).toBe(false);

		// Unknown projects fail truthfully through the router — never with
		// another owner's authority. Unknown callers are rejected, never
		// silently attributed to the foreground session.
		const nowhere = TempDir.createSync("@harvest-two-owners-nowhere-");
		cleanups.push(async () => {
			await nowhere.remove().catch(() => {});
		});
		const nowhereSession = {
			cwd: nowhere.path(),
			settings: Settings.isolated({ "autolearn.enabled": true }),
			getSessionId: () => uuidA,
			getSessionSpawns: () => "*",
			taskDepth: 0,
			agentRegistry: AgentRegistry.global(),
		} as unknown as ToolSession;
		await expect(new SessionsTool(nowhereSession).execute("6", { action: "create" })).rejects.toThrow(
			/unwired|limited/,
		);
		const { tool: unknownCaller } = callerTools(ownerA, "00000000-0000-0000-0000-000000000000");
		await expect(unknownCaller.execute("7", { action: "create" })).rejects.toThrow(/not live|unwired|limited/);
	}, 120000);
});
