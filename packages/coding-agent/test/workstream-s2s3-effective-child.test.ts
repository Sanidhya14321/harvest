import { afterEach, beforeEach, describe, expect, it } from "bun:test";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { Snowflake } from "@harvest/pi-utils";
import { getBundledModel } from "@harvest/pi-catalog/models";
import { ModelRegistry } from "../src/config/model-registry";
import { Settings } from "../src/config/settings";
import { createAgentSession, discoverAuthStorage, getToolSessionForAgentSession } from "../src/sdk";
import { SessionManager } from "../src/session/session-manager";
import {
	openLiveAgentSessionFromSnapshot,
	snapshotTrustedCaller,
} from "../src/session/live-session-factory";

/**
 * T03/S2: a real restricted SDK parent ([read, sessions, task], restricted
 * options, bounded spawns) creates a usable child through the production
 * factory path. The child exposes exactly the intended tools — forbidden
 * execution is impossible because the tool is absent — and inherits the
 * spawn/permission/depth limits. Asserts executed behavior (real tool
 * registry lookups on real sessions), not factory arguments.
 */
describe("restricted caller begets restricted child (real SDK)", () => {
	const tempDirs: string[] = [];
	let modelRegistry!: ModelRegistry;

	function makeTempDir(): string {
		const created = path.join(os.tmpdir(), `pi-child-policy-${Snowflake.next()}`);
		fs.mkdirSync(created, { recursive: true });
		const tempDir = fs.realpathSync.native(created);
		tempDirs.push(tempDir);
		return tempDir;
	}

	beforeEach(async () => {
		const authDir = path.join(os.tmpdir(), `pi-child-policy-auth-${Snowflake.next()}`);
		fs.mkdirSync(authDir, { recursive: true });
		tempDirs.push(authDir);
		modelRegistry = new ModelRegistry(await discoverAuthStorage(authDir));
	});

	afterEach(async () => {
		for (const dir of tempDirs.splice(0)) {
			try {
				fs.rmSync(dir, { recursive: true, force: true });
			} catch {
				// Best-effort cleanup.
			}
		}
	});

	it("a [read, sessions, task] restricted parent yields a child without bash/edit/write and with scout-only spawns", async () => {
		const projectDir = makeTempDir();
		// autolearn.enabled gates the sessions tool itself (model-callable
		// management surface); the restriction under test is the tool allowlist.
		const settings = Settings.isolated({ "autolearn.enabled": true });
		const { session: parent } = await createAgentSession({
			cwd: projectDir,
			agentDir: projectDir,
			modelRegistry,
			sessionManager: SessionManager.inMemory(),
			settings,
			model: getBundledModel("openai", "gpt-4o-mini"),
			disableExtensionDiscovery: true,
			skills: [],
			contextFiles: [],
			promptTemplates: [],
			slashCommands: [],
			enableMCP: false,
			enableLsp: false,
			rules: [],
			workspaceTree: { rootPath: projectDir, rendered: "", truncated: false, totalLines: 0, agentsMdFiles: [] },
			toolNames: ["read", "sessions", "task"],
			restrictToolNames: true,
			spawns: "scout",
			taskDepth: 0,
		});
		try {
			// Parent surface is actually restricted (executed behavior).
			expect(parent.getToolByName("read")).toBeDefined();
			expect(parent.getToolByName("bash")).toBeUndefined();
			expect(parent.getToolByName("edit")).toBeUndefined();
			expect(parent.getToolByName("write")).toBeUndefined();
			// The SDK-registered ToolSession carries the effective facts.
			const parentTools = getToolSessionForAgentSession(parent);
			if (!parentTools) throw new Error("Parent ToolSession is not registered");
			expect(parentTools.taskDepth).toBe(0);

			const snapshot = snapshotTrustedCaller(
				{
					sessionManager: parent.sessionManager,
					session: parent,
					mcpManager: undefined,
					getSessionId: () => parent.sessionManager.getSessionId(),
					getAgentId: () => "Main",
				},
				{
					callerSessionId: parent.sessionManager.getSessionId(),
					callerAgentId: "Main",
					taskDepth: parentTools.taskDepth ?? 0,
					policyFallback: {
						toolNames: parentTools.toolNames,
						restrictToolNames: parentTools.restrictToolNames,
						spawns: parentTools.getSessionSpawns(),
						autoApprove: parentTools.autoApprove,
						enableMCP: parentTools.enableMCP,
					},
				},
			);
			expect(snapshot.policy.toolNames).toEqual(["read", "sessions", "task"]);
			expect(snapshot.policy.restrictToolNames).toBe(true);
			expect(snapshot.taskDepth).toBe(1);

			const child = await openLiveAgentSessionFromSnapshot(snapshot, { background: true });
			try {
				// Intended tools work; forbidden tools are absent (not merely denied).
				expect(child.getToolByName("read")).toBeDefined();
				expect(child.getToolByName("sessions")).toBeDefined();
				expect(child.getToolByName("task")).toBeDefined();
				expect(child.getToolByName("bash")).toBeUndefined();
				expect(child.getToolByName("edit")).toBeUndefined();
				expect(child.getToolByName("write")).toBeUndefined();
				const childTools = getToolSessionForAgentSession(child);
				if (!childTools) throw new Error("Child ToolSession is not registered");
				expect(childTools.getSessionSpawns()).toBe("scout");
				expect(childTools.taskDepth).toBe(1);
			} finally {
				await child.dispose();
			}
		} finally {
			await parent.dispose();
		}
	});
});
