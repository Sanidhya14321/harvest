import { describe, expect, it } from "bun:test";
import { RpcClient, RpcCommandError, type RpcAgentProcess } from "@harvest/pi-coding-agent/modes/rpc/rpc-client";

/**
 * U4: waitForIdle/collectEvents/promptAndWait must complete through
 * request-correlated signals — a local-only result, an already-idle agent,
 * a rejection, or a disconnect — instead of hanging until the timeout.
 *
 * A scripted fake agent process drives the real client: correlated prompt
 * responses, id-matched prompt_result frames, agent_end events, and a
 * closable stdout for the disconnect path.
 */
describe("RpcClient run-completion helpers", () => {
	function startFakeAgent() {
		let controller!: ReadableStreamDefaultController<Uint8Array>;
		const stdout = new ReadableStream<Uint8Array>({
			start(c) {
				controller = c;
			},
		});
		const textEncoder = new TextEncoder();
		const received: Array<{ type: string; id?: string }> = [];
		let streaming = false;
		let promptHandler: ((id: string) => void) | undefined;
		const emit = (frame: unknown): void => {
			controller.enqueue(textEncoder.encode(`${JSON.stringify(frame)}\n`));
		};
		const fakeProcess: RpcAgentProcess = {
			stdin: {
				write(data: string | Uint8Array) {
					const text = typeof data === "string" ? data : new TextDecoder().decode(data);
					for (const line of text.split("\n")) {
						if (!line.trim()) continue;
						const command = JSON.parse(line) as { type: string; id?: string };
						received.push(command);
						if (command.type === "get_state") {
							emit({
								id: command.id,
								type: "response",
								command: "get_state",
								success: true,
								data: { isStreaming: streaming },
							});
						} else if (command.type === "prompt") {
							promptHandler?.(command.id ?? "");
						}
					}
					return 0;
				},
			},
			stdout,
			peekStderr: () => "",
			kill: () => {},
			exited: new Promise<number>(() => {}),
		};
		const client = new RpcClient({ spawn: () => fakeProcess });
		const starting = client.start();
		emit({
			type: "ready",
			protocolVersion: 1,
			supportedProtocolVersions: [1],
			maxFrameBytes: 0,
			maxReassembledFrameBytes: 0,
		});
		return {
			client,
			started: starting,
			received,
			emit,
			setStreaming: (value: boolean): void => {
				streaming = value;
			},
			onPrompt: (handler: (id: string) => void): void => {
				promptHandler = handler;
			},
			closeStdout: (): void => {
				controller.close();
			},
		};
	}

	it("promptAndWait resolves a local-only prompt without waiting for agent_end", async () => {
		const fake = startFakeAgent();
		await fake.started;
		fake.onPrompt(id => {
			fake.emit({ id, type: "response", command: "prompt", success: true, data: { agentInvoked: false } });
		});

		const events = await fake.client.promptAndWait("/status", undefined, 2000);
		expect(events).toEqual([]);
		expect(fake.received.filter(command => command.type === "prompt")).toHaveLength(1);
	});

	it("promptAndWait resolves a deferred local-only prompt via its prompt_result id", async () => {
		const fake = startFakeAgent();
		await fake.started;
		fake.onPrompt(id => {
			fake.emit({ id, type: "response", command: "prompt", success: true });
			queueMicrotask(() => {
				fake.emit({ type: "prompt_result", id, agentInvoked: false });
			});
		});

		const events = await fake.client.promptAndWait("deferred local work", undefined, 2000);
		expect(events).toEqual([]);
	});

	it("promptAndWait waits for agent_end when the agent runs", async () => {
		const fake = startFakeAgent();
		await fake.started;
		fake.onPrompt(id => {
			fake.emit({ id, type: "response", command: "prompt", success: true });
			queueMicrotask(() => {
				fake.emit({ type: "agent_end", messages: [] });
			});
		});

		const events = await fake.client.promptAndWait("do work", undefined, 2000);
		expect(events).toEqual([{ type: "agent_end", messages: [] }]);
	});

	it("promptAndWait surfaces a rejected prompt instead of leaving the wait pending", async () => {
		const fake = startFakeAgent();
		await fake.started;
		fake.onPrompt(id => {
			fake.emit({ id, type: "response", command: "prompt", success: false, error: "agent is busy" });
		});

		const error = await fake.client.promptAndWait("do work", undefined, 2000).then(
			() => undefined,
			(reason: unknown) => reason,
		);
		expect(error).toBeInstanceOf(RpcCommandError);
	});

	it("waitForIdle resolves immediately when the agent is already idle", async () => {
		const fake = startFakeAgent();
		await fake.started;
		fake.setStreaming(false);

		await fake.client.waitForIdle(2000);
	});

	it("waitForIdle waits for agent_end while the agent streams", async () => {
		const fake = startFakeAgent();
		await fake.started;
		fake.setStreaming(true);

		const waiting = fake.client.waitForIdle(2000);
		await Bun.sleep(10);
		fake.emit({ type: "agent_end", messages: [] });
		await waiting;
	});

	it("a disconnect settles a pending wait instead of hanging until the timeout", async () => {
		const fake = startFakeAgent();
		await fake.started;
		fake.setStreaming(true);

		const waiting = fake.client.waitForIdle(30_000);
		const failure = waiting.then(
			() => undefined,
			(reason: unknown) => reason,
		);
		fake.closeStdout();
		const error = await failure;
		expect(error).toBeInstanceOf(Error);
		expect(String((error as Error).message)).toContain("disconnected");
	});
});
