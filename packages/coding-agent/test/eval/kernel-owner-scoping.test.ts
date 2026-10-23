import { afterEach, describe, expect, it } from "bun:test";
import { TempDir } from "@harvest/pi-utils";
import { Settings } from "../../src/config/settings";
import { resolveOwnerScopedSessionKey, type SessionOwners } from "../../src/eval/executor-base";
import { disposeAllVmContexts, disposeVmContextsByOwner } from "../../src/eval/js/context-manager";
import { executeJs } from "../../src/eval/js/executor";
import type { ToolSession } from "../../src/tools";

function makeSession(cwd: string): ToolSession {
	return {
		cwd,
		hasUI: false,
		settings: Settings.isolated({
			"async.enabled": false,
			"task.isolation.enabled": false,
			"task.enableLsp": true,
		}),
		taskDepth: 0,
		enableLsp: true,
		getSessionFile: () => null,
		getSessionSpawns: () => "*",
		getActiveModelString: () => "p/active",
		getModelString: () => "p/fallback",
		getArtifactsDir: () => null,
		getSessionId: () => "test-session",
		getEvalSessionId: () => "test-eval-session",
	};
}

describe("resolveOwnerScopedSessionKey", () => {
	const BASE = "sess\0/cwd\0interp";
	const FORK = `${BASE}\0fork\0owner-b`;

	function resolve(options: { ownerId?: string; reset?: boolean; live?: Record<string, SessionOwners> }): string {
		const live = options.live ?? {};
		return resolveOwnerScopedSessionKey({
			baseKey: BASE,
			ownerId: options.ownerId,
			reset: options.reset === true,
			hasSession: key => key in live,
			getOwners: key => live[key],
		});
	}

	it("keeps the base key when the caller has no owner identity", () => {
		expect(resolve({ reset: true, live: { [BASE]: { ownerIds: new Set(["x"]), hasFallbackOwner: false } } })).toBe(
			BASE,
		);
	});

	it("stays on an existing fork even without reset", () => {
		const live = {
			[BASE]: { ownerIds: new Set(["owner-a", "owner-b"]), hasFallbackOwner: false },
			[FORK]: { ownerIds: new Set(["owner-b"]), hasFallbackOwner: false },
		};
		expect(resolve({ ownerId: "owner-b", live })).toBe(FORK);
		expect(resolve({ ownerId: "owner-b", reset: true, live })).toBe(FORK);
	});

	it("forks a reset away from a co-owned base session", () => {
		const live = { [BASE]: { ownerIds: new Set(["owner-a", "owner-b"]), hasFallbackOwner: false } };
		expect(resolve({ ownerId: "owner-b", reset: true, live })).toBe(FORK);
	});

	it("forks a reset away from a fallback-owned base session", () => {
		// Fallback ownership means some session without an explicit owner uses
		// the context; a scoped reset must not destroy it.
		const live = { [BASE]: { ownerIds: new Set(["session-id"]), hasFallbackOwner: true } };
		expect(resolve({ ownerId: "owner-b", reset: true, live })).toBe(FORK);
	});

	it("resets in place when the requester exclusively owns the base session", () => {
		const live = { [BASE]: { ownerIds: new Set(["owner-b"]), hasFallbackOwner: false } };
		expect(resolve({ ownerId: "owner-b", reset: true, live })).toBe(BASE);
	});

	it("uses the base key for a reset when no live session exists", () => {
		expect(resolve({ ownerId: "owner-b", reset: true })).toBe(BASE);
		expect(resolve({ ownerId: "owner-b" })).toBe(BASE);
	});
});

describe("JS eval owner-scoped reset forking", () => {
	afterEach(async () => {
		await disposeAllVmContexts();
	});

	it("forks shared resets while resetting an exclusive owner in place", async () => {
		using tempDir = TempDir.createSync("@omp-js-owner-fork-");
		const session = makeSession(tempDir.path());
		const evalSessionId = `js-owner-fork:${crypto.randomUUID()}`;
		const run = (code: string, kernelOwnerId: string, reset?: boolean) =>
			executeJs(code, { cwd: tempDir.path(), sessionId: evalSessionId, session, kernelOwnerId, reset });

		await run("var shared = 41;", "agent-a");
		const joined = await run("return shared + 1;", "agent-b");
		expect(joined.output.trim()).toBe("42");

		// agent-b resets: it must land on a private fork with fresh state...
		const forked = await run("return typeof shared;", "agent-b", true);
		expect(forked.output.trim()).toBe("undefined");
		// ...while agent-a's shared context keeps its state.
		const preserved = await run("return shared + 1;", "agent-a");
		expect(preserved.output.trim()).toBe("42");

		// The fork is sticky: agent-b keeps resolving to it without reset.
		await run("var forkOnly = 7;", "agent-b");
		const sticky = await run("return forkOnly;", "agent-b");
		expect(sticky.output.trim()).toBe("7");

		// Disposing agent-b reaps only the fork; the shared context survives.
		await disposeVmContextsByOwner("agent-b");
		const survivor = await run("return shared + 1;", "agent-a");
		expect(survivor.output.trim()).toBe("42");

		// Once agent-a is the exclusive owner, reset reuses its process but clears
		// the context rather than needlessly forking another worker.
		const reset = await run("return typeof shared;", "agent-a", true);
		expect(reset.output.trim()).toBe("undefined");
	});
});
