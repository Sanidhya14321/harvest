import { afterEach, beforeAll, describe, expect, it } from "bun:test";
import { Agent } from "@harvest/pi-agent-core";
import { initTheme } from "../src/modes/theme/theme";
import { InteractiveMode } from "../src/modes/interactive-mode";
import { AgentSession } from "../src/session/agent-session";
import { SessionManager } from "../src/session/session-manager";
import { Settings, resetSettingsForTest } from "../src/config/settings";
import { TempDir } from "@harvest/pi-utils";

beforeAll(async () => {
	await initTheme();
	resetSettingsForTest();
	await Settings.init({ inMemory: true });
});

/**
 * Per-owner submission slots: submitting in one tab must never clobber (or
 * cancel dispatch of) another tab's submission. Dispatch matches by object
 * identity against the owning session's entry; the visible-session entry
 * drives optimistic/goal flows exactly as before.
 */
describe("interactive pending submissions per owner", () => {
	const tempDirs: TempDir[] = [];
	const modes: InteractiveMode[] = [];

	afterEach(async () => {
		for (const mode of modes.splice(0)) mode.stop();
		for (const dir of tempDirs.splice(0)) {
			try {
				dir.removeSync();
			} catch {}
		}
		resetSettingsForTest();
	});

	async function makeMode(): Promise<{ mode: InteractiveMode; session: AgentSession }> {
		resetSettingsForTest();
		await Settings.init({ inMemory: true });
		const tempDir = TempDir.createSync("@pi-pending-submission-");
		tempDirs.push(tempDir);
		const session = new AgentSession({
			agent: new Agent({ initialState: { systemPrompt: [], messages: [], tools: [] } }),
			sessionManager: SessionManager.create(tempDir.path(), tempDir.path()),
			settings: Settings.isolated({ "compaction.enabled": false }),
			modelRegistry: { isUsingOAuth: () => false } as never,
		});
		const mode = new InteractiveMode(session, "test");
		modes.push(mode);
		return { mode, session };
	}

	it("keeps both dispatches alive when two tabs submit back to back", async () => {
		const first = await makeMode();
		const second = await makeMode();
		const firstInput = first.mode.startPendingSubmission({ text: "first tab prompt" });
		// Navigate: the mode now points at another session while the first
		// submission still awaits dispatch.
		first.mode.session = second.session;
		const secondInput = first.mode.startPendingSubmission({ text: "second tab prompt" });

		expect(first.mode.markPendingSubmissionStarted(firstInput)).toBe(true);
		expect(first.mode.markPendingSubmissionStarted(secondInput)).toBe(true);
		expect(firstInput.started).toBe(true);
		expect(secondInput.started).toBe(true);

		first.mode.finishPendingSubmission(firstInput);
		first.mode.finishPendingSubmission(secondInput);
	});

	it("cancels only the visible session's pending submission", async () => {
		const first = await makeMode();
		const second = await makeMode();
		const firstInput = first.mode.startPendingSubmission({ text: "first tab prompt" });
		first.mode.session = second.session;
		const secondInput = first.mode.startPendingSubmission({ text: "second tab prompt" });

		expect(first.mode.cancelPendingSubmission()).toBe(true);
		expect(secondInput.cancelled).toBe(true);
		expect(firstInput.cancelled).toBe(false);
		expect(first.mode.markPendingSubmissionStarted(firstInput)).toBe(true);
	});

	it("keeps the visible optimistic row when a background tab's row is cleared", async () => {
		const first = await makeMode();
		const second = await makeMode();
		const firstInput = first.mode.startPendingSubmission({ text: "first tab prompt" });
		first.mode.session = second.session;
		first.mode.startPendingSubmission({ text: "second tab prompt" });

		// A background completion/error clearing its own optimistic state must
		// not erase the visible tab's row or its local-submission marker.
		first.mode.clearOptimisticUserMessage({ owner: first.session });
		expect(first.mode.optimisticUserMessageSignature).toBe("second tab prompt\u00000");
		expect(first.mode.locallySubmittedUserSignatures.has("second tab prompt\u00000")).toBe(true);
		expect(first.mode.locallySubmittedUserSignatures.has("first tab prompt\u00000")).toBe(false);

		// Both dispatches still resolve against their owning session.
		expect(first.mode.markPendingSubmissionStarted(firstInput)).toBe(true);
	});

	it("parks skill rows per owner so a background clear leaves the visible row", async () => {
		const first = await makeMode();
		const second = await makeMode();
		first.mode.renderOptimisticSkillMessage({ role: "custom", content: "first skill" } as never);
		first.mode.session = second.session;
		first.mode.renderOptimisticSkillMessage({ role: "custom", content: "second skill" } as never);

		first.mode.clearOptimisticSkillMessage({ owner: first.session });
		expect(first.mode.optimisticSkillMessagePending).toBe(true);
		expect(first.mode.hasPendingOptimisticSkill(first.session)).toBe(false);
		expect(first.mode.hasPendingOptimisticSkill(second.session)).toBe(true);
	});
});
