import { describe, expect, it, mock } from "bun:test";
import type { InteractiveModeContext } from "../../src/modes/types";
import type { TuiSlashCommandRuntime } from "../../src/slash-commands/types";
import { executeBuiltinSlashCommand } from "../../src/slash-commands/builtin-registry";

interface ExitHarness {
	runtime: TuiSlashCommandRuntime;
	shutdown: ReturnType<typeof mock>;
	confirm: ReturnType<typeof mock>;
	setText: ReturnType<typeof mock>;
	resolveConfirm: (value: boolean) => void;
}

function createHarness(options?: { streaming?: boolean; bashRunning?: boolean; approvalOpen?: boolean }): ExitHarness {
	let resolveConfirm!: (value: boolean) => void;
	const confirmGate = new Promise<boolean>(resolve => {
		resolveConfirm = resolve;
	});
	const shutdown = mock(async () => {});
	const confirm = mock(() => confirmGate);
	const setText = mock((_text: string) => {});
	let editorText = "unsent draft";
	const session = {
		isStreaming: options?.streaming ?? false,
		isBashRunning: options?.bashRunning ?? false,
		isEvalRunning: false,
		hasPendingAsyncWork: () => false,
	};
	const ctx = {
		session,
		editor: {
			getText: () => editorText,
			setText: (text: string) => {
				editorText = text;
				setText(text);
			},
		},
		hookSelector: options?.approvalOpen ? {} : undefined,
		hookInput: undefined,
		hookEditor: undefined,
		showHookConfirm: confirm,
		shutdown,
	} as unknown as InteractiveModeContext;
	return { runtime: { ctx }, shutdown, confirm, setText, resolveConfirm };
}

describe("/exit with work in progress", () => {
	it("shuts down immediately without asking when the session is idle", async () => {
		const harness = createHarness();
		expect(await executeBuiltinSlashCommand("/exit", harness.runtime)).toBe(true);
		expect(harness.confirm).not.toHaveBeenCalled();
		expect(harness.setText).toHaveBeenCalledWith("");
		expect(harness.shutdown).toHaveBeenCalledTimes(1);
	});

	it("asks once and stops only after confirmation while a run streams", async () => {
		const harness = createHarness({ streaming: true });
		expect(await executeBuiltinSlashCommand("/exit", harness.runtime)).toBe(true);
		expect(harness.confirm).toHaveBeenCalledTimes(1);
		expect(harness.shutdown).not.toHaveBeenCalled();
		harness.resolveConfirm(true);
		await Bun.sleep(0);
		expect(harness.setText).toHaveBeenCalledWith("");
		expect(harness.shutdown).toHaveBeenCalledTimes(1);
	});

	it("leaves the run and the draft untouched when the confirmation is denied", async () => {
		const harness = createHarness({ streaming: true });
		expect(await executeBuiltinSlashCommand("/exit", harness.runtime)).toBe(true);
		expect(harness.confirm).toHaveBeenCalledTimes(1);
		harness.resolveConfirm(false);
		await Bun.sleep(0);
		expect(harness.shutdown).not.toHaveBeenCalled();
		expect(harness.setText).not.toHaveBeenCalled();
	});

	it("asks while an approval waits even when the turn itself is idle", async () => {
		const harness = createHarness({ approvalOpen: true });
		expect(await executeBuiltinSlashCommand("/exit", harness.runtime)).toBe(true);
		expect(harness.confirm).toHaveBeenCalledTimes(1);
		expect(harness.shutdown).not.toHaveBeenCalled();
	});
});
