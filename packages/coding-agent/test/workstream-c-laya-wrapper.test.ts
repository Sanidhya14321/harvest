/**
 * Workstream C — extension wrapper ignores legacy Laya gating metadata.
 *
 * Contracts:
 * - Legacy `providerMetadata.layaGatingRequired/layaGatingReason` (on either
 *   a `computer` record or a `type: "laya"` record) must NOT change approval
 *   outcomes: identical behavior with and without it under yolo.
 * - Provider computer safety checks are preserved byte-for-byte: pending
 *   checks still force an interactive prompt even under yolo, fail closed
 *   without a UI, and never acknowledge via yolo/allow.
 */
import { afterEach, describe, expect, it, vi } from "bun:test";
import { ExtensionRunner } from "@harvest/pi-coding-agent/extensibility/extensions/runner";
import { ExtensionToolWrapper } from "@harvest/pi-coding-agent/extensibility/extensions/wrapper";
import { ExtensionRuntime, loadExtensions } from "@harvest/pi-coding-agent/extensibility/extensions/loader";
import { SessionManager } from "@harvest/pi-coding-agent/session/session-manager";
import { TempDir } from "@harvest/pi-utils";

afterEach(() => {
	vi.restoreAllMocks();
});

function yoloSettings(): never {
	return { get: (key: string) => (key === "tools.approvalMode" ? "yolo" : {}) } as never;
}

function baseContext(
	extra?: { toolCall?: Record<string, unknown> } & Record<string, unknown>,
): Record<string, unknown> {
	const { toolCall, ...rest } = extra ?? {};
	return {
		sessionManager: SessionManager.inMemory(),
		modelRegistry: undefined,
		model: undefined,
		isIdle: () => true,
		hasQueuedMessages: () => false,
		abort: () => {},
		settings: yoloSettings(),
		toolCall: toolCall ? { index: 0, toolCalls: [], ...toolCall } : { index: 0, toolCalls: [] },
		...rest,
	} as unknown as Record<string, unknown>;
}

async function makeRunner(options?: {
	select?: () => Promise<string>;
}): Promise<{ runner: ExtensionRunner; tempDir: TempDir; select: ReturnType<typeof vi.fn> }> {
	const tempDir = TempDir.createSync("@harvest-laya-wrapper-");
	const loaded = await loadExtensions([], tempDir.path());
	const runner = new ExtensionRunner(
		loaded.extensions,
		loaded.runtime,
		tempDir.path(),
		SessionManager.inMemory(),
		{} as never,
	);
	const select = vi.fn(async () => "Approve");
	if (options?.select) {
		select.mockImplementation(options.select);
	}
	runner.initialize(
		{
			sendMessage: () => {},
			sendUserMessage: () => {},
			appendEntry: () => {},
			setLabel: () => {},
			getActiveTools: () => [],
			getAllTools: () => [],
			setActiveTools: async () => {},
			getCommands: () => [],
			setModel: async () => false,
			getThinkingLevel: () => undefined,
			setThinkingLevel: () => {},
			getSessionName: () => undefined,
			setSessionName: async () => {},
		},
		{
			getModel: () => undefined,
			isIdle: () => true,
			abort: () => {},
			hasPendingMessages: () => false,
			shutdown: () => {},
			getContextUsage: () => undefined,
			compact: async () => {},
			getSystemPrompt: () => [],
		},
		undefined,
		{
			select,
			confirm: async () => false,
			input: async () => undefined,
			notify: () => {},
			onTerminalInput: () => () => {},
			setStatus: () => {},
			setWorkingMessage: () => {},
			setWidget: () => {},
			setFooter: () => {},
			setHeader: () => {},
			setTitle: () => {},
			custom: async <T>() => undefined as T,
			pasteToEditor: () => {},
			setEditorText: () => {},
			getEditorText: () => "",
			editor: async () => undefined,
			addAutocompleteProvider: () => {},
			setEditorComponent: () => {},
			get theme() {
				return {} as never;
			},
			getAllThemes: async () => [],
			getTheme: async () => undefined,
			setTheme: async () => ({ success: false, error: "not implemented" }),
			getToolsExpanded: () => false,
			setToolsExpanded: () => {},
		},
	);
	return { runner, tempDir, select };
}

const ordinaryTool = {
	name: "ordinary_tool",
	label: "Ordinary Tool",
	description: "Ordinary exec-tier tool",
	parameters: {} as never,
	approval: "exec" as const,
	execute: async () => ({ content: [{ type: "text" as const, text: "ok" }] }),
};

describe("wrapper ignores legacy laya metadata", () => {
	it("runs identically with and without legacy gating flags under yolo", async () => {
		// Failure mode: a stale `layaGatingRequired: true` record forces a
		// prompt (or denial) the configured yolo permissions never asked for.
		const { runner, tempDir, select } = await makeRunner();
		try {
			const plain = new ExtensionToolWrapper(ordinaryTool, runner);
			const plainResult = await (plain as ExtensionToolWrapper<any>).execute(
				"call-plain",
				{},
				undefined,
				undefined,
				baseContext() as never,
			);
			expect(plainResult.content).toEqual([{ type: "text", text: "ok" }]);

			const legacyComputer = new ExtensionToolWrapper(ordinaryTool, runner);
			const legacyComputerResult = await (legacyComputer as ExtensionToolWrapper<any>).execute(
				"call-legacy-computer",
				{},
				undefined,
				undefined,
				baseContext({
					toolCall: {
						providerMetadata: {
							type: "computer",
							providerItemId: "item-1",
							actions: [],
							pendingSafetyChecks: [],
							layaGatingRequired: true,
							layaGatingReason: "legacy risk verdict",
						},
					},
				}) as never,
			);
			expect(legacyComputerResult).toEqual(plainResult);

			const legacyLaya = new ExtensionToolWrapper(ordinaryTool, runner);
			const legacyLayaResult = await (legacyLaya as ExtensionToolWrapper<any>).execute(
				"call-legacy-laya",
				{},
				undefined,
				undefined,
				baseContext({
					toolCall: {
						providerMetadata: { type: "laya", layaGatingRequired: true, layaGatingReason: "legacy" },
					},
				}) as never,
			);
			expect(legacyLayaResult).toEqual(plainResult);
			expect(select).not.toHaveBeenCalled();
		} finally {
			tempDir.removeSync();
		}
	});

	it("never surfaces a legacy gating reason in the approval prompt", async () => {
		// Failure mode: the removed sidecar's reason string leaks into the
		// human-approval prompt for an unrelated computer safety check.
		const { runner, tempDir, select } = await makeRunner();
		try {
			const wrapper = new ExtensionToolWrapper(ordinaryTool, runner);
			await (wrapper as ExtensionToolWrapper<any>).execute(
				"call-safety-plus-legacy",
				{},
				undefined,
				undefined,
				baseContext({
					toolCall: {
						providerMetadata: {
							type: "computer",
							providerItemId: "item-2",
							actions: [],
							pendingSafetyChecks: [{ id: "check-1", code: "test", message: "safety check" }],
							layaGatingRequired: true,
							layaGatingReason: "LEGACY-REASON-MUST-NOT-SURFACE",
						},
					},
				}) as never,
			);
			expect(select).toHaveBeenCalledTimes(1);
			const prompt = String(select.mock.calls[0]?.[0] ?? "");
			expect(prompt).toContain("Provider safety checks:");
			expect(prompt).not.toContain("LEGACY-REASON-MUST-NOT-SURFACE");
		} finally {
			tempDir.removeSync();
		}
	});
});

describe("computer safety checks still gate under yolo", () => {
	it("prompts on pending safety checks and approves through the UI", async () => {
		// Failure mode: removing legacy gating accidentally weakens the
		// mandatory provider computer-safety gate.
		const { runner, tempDir, select } = await makeRunner();
		try {
			const wrapper = new ExtensionToolWrapper(ordinaryTool, runner);
			const context = baseContext({
				toolCall: {
					providerMetadata: {
						type: "computer",
						providerItemId: "item-3",
						actions: [],
						pendingSafetyChecks: [{ id: "check-1", code: "test", message: "confirm this action" }],
					},
				},
			}) as never;
			const result = await (wrapper as ExtensionToolWrapper<any>).execute(
				"call-safety",
				{},
				undefined,
				undefined,
				context,
			);
			expect(select).toHaveBeenCalledTimes(1);
			expect(result.content).toEqual([{ type: "text", text: "ok" }]);
			expect((context as { providerSafetyApproved?: boolean }).providerSafetyApproved).toBe(true);
		} finally {
			tempDir.removeSync();
		}
	});

	it("fails closed on pending safety checks without an interactive UI", async () => {
		// Failure mode: a headless host silently auto-approves a provider
		// safety check yolo must never acknowledge.
		const tempDir = TempDir.createSync("@harvest-laya-wrapper-noui-");
		try {
			const loaded = await loadExtensions([], tempDir.path());
			const runner = new ExtensionRunner(
				loaded.extensions,
				loaded.runtime,
				tempDir.path(),
				SessionManager.inMemory(),
				{} as never,
			);
			const wrapper = new ExtensionToolWrapper(ordinaryTool, runner);
			const error = await (wrapper as ExtensionToolWrapper<any>)
				.execute(
					"call-safety-noui",
					{},
					undefined,
					undefined,
					baseContext({
						toolCall: {
							providerMetadata: {
								type: "computer",
								providerItemId: "item-4",
								actions: [],
								pendingSafetyChecks: [{ id: "check-1" }],
							},
						},
					}) as never,
				)
				.then(
					() => null,
					(error: unknown) => error,
				);
			expect(error).toBeInstanceOf(Error);
			expect(String((error as Error).message)).toContain("pending provider safety checks");
		} finally {
			tempDir.removeSync();
		}
	});
});

// Keep the unused import referenced for intent: the runtime type documents
// which shared runtime the wrapper tests construct runners against.
void ExtensionRuntime;
