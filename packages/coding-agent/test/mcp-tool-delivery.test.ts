import { describe, expect, it } from "bun:test";
import type { CustomToolContext } from "@harvest/pi-coding-agent/extensibility/custom-tools";
import { MCPTransportError } from "@harvest/pi-coding-agent/mcp/errors";
import { DeferredMCPTool, MCPTool } from "@harvest/pi-coding-agent/mcp/tool-bridge";
import type { MCPToolDefinition } from "@harvest/pi-coding-agent/mcp/types";
import { createMockConnection, createMockTransport } from "./mcp-test-utils";

const context = {} as CustomToolContext;
const definition: MCPToolDefinition = {
	name: "publish",
	inputSchema: { type: "object" },
	annotations: { idempotentHint: false, readOnlyHint: false },
};
const readDefinition: MCPToolDefinition = {
	name: "read",
	inputSchema: { type: "object" },
	annotations: { readOnlyHint: true },
};

/** Idempotency key the bridge stamped on a `tools/call` params envelope. */
function sentIdempotencyKey(params: Record<string, unknown> | undefined): unknown {
	if (!params || typeof params._meta !== "object" || params._meta === null) return undefined;
	return (params._meta as Record<string, unknown>)["mcp/idempotency-key"];
}

describe("MCP tool delivery", () => {
	for (const deferred of [false, true]) {
		const mode = deferred ? "deferred" : "eager";
		it(`${mode}: a committed action with a lost response is not replayed after reconnect`, async () => {
			let effects = 0;
			let reconnects = 0;
			const transport = createMockTransport(new Map(), () => {
				effects++;
				throw new MCPTransportError({
					transport: "stdio",
					stage: "receive",
					failure: "eof",
					retryable: true,
					message: "response stream closed after commit",
				});
			});
			const connection = createMockConnection({ tools: {} }, transport);
			const recovered = createMockConnection(
				{ tools: {} },
				createMockTransport(new Map([["tools/call", [{ content: [{ type: "text", text: "published" }] }]]]), () => {
					effects++;
				}),
			);
			const reconnect = async () => {
				reconnects++;
				return recovered;
			};
			const tool = deferred
				? new DeferredMCPTool(connection.name, definition, async () => connection, undefined, reconnect)
				: new MCPTool(connection, definition, reconnect);
			const result = await tool.execute("publish-1", {}, undefined, context);
			expect(effects).toBe(1);
			expect(reconnects).toBe(0);
			expect(result.isError).toBe(true);
			expect(result.details?.outcomeUnknown).toBe(true);
			expect(result.content).toContainEqual(
				expect.objectContaining({ type: "text", text: expect.stringContaining("Verify the remote state") }),
			);
		});

		it(`${mode}: a transport-confirmed pre-dispatch failure reconnects and executes once`, async () => {
			let effects = 0;
			const connection = createMockConnection(
				{ tools: {} },
				createMockTransport(new Map(), () => {
					throw new MCPTransportError({
						transport: "http",
						stage: "connect",
						failure: "closed",
						retryable: true,
						message: "Transport not connected",
					});
				}),
			);
			const recovered = createMockConnection(
				{ tools: {} },
				createMockTransport(new Map([["tools/call", [{ content: [{ type: "text", text: "published" }] }]]]), () => {
					effects++;
				}),
			);
			const tool = deferred
				? new DeferredMCPTool(
						connection.name,
						definition,
						async () => connection,
						undefined,
						async () => recovered,
					)
				: new MCPTool(connection, definition, async () => recovered);
			const result = await tool.execute("publish-2", {}, undefined, context);
			expect(effects).toBe(1);
			expect(result.isError).not.toBe(true);
			expect(result.content).toEqual([{ type: "text", text: "published" }]);
		});
	}

	it("does not treat server idempotency hints as proof of safe replay after a reset", async () => {
		let calls = 0;
		const transport = createMockTransport(new Map(), () => {
			calls++;
			throw new MCPTransportError({
				transport: "http",
				stage: "send",
				failure: "reset",
				retryable: true,
				message: "ECONNRESET",
			});
		});
		const connection = createMockConnection({ tools: {} }, transport);
		const tool = new MCPTool(
			connection,
			{ ...definition, annotations: { idempotentHint: true } },
			async () => connection,
		);
		const result = await tool.execute("publish-3", {}, undefined, context);
		expect(calls).toBe(1);
		expect(result.details?.outcomeUnknown).toBe(true);
	});

	describe("read/write split and server-provided idempotency keys", () => {
		for (const deferred of [false, true]) {
			const mode = deferred ? "deferred" : "eager";
			it(`${mode}: a read-only tool replays once after a post-send EOF`, async () => {
				let firstAttempts = 0;
				let recoveredCalls = 0;
				const firstKeys: unknown[] = [];
				const retryKeys: unknown[] = [];
				const transport = createMockTransport(new Map(), (_method, params) => {
					firstAttempts++;
					firstKeys.push(sentIdempotencyKey(params));
					throw new MCPTransportError({
						transport: "http",
						stage: "receive",
						failure: "eof",
						retryable: true,
						message: "response stream closed after dispatch",
					});
				});
				const connection = createMockConnection({ tools: {} }, transport);
				const recovered = createMockConnection(
					{ tools: {} },
					createMockTransport(
						new Map([["tools/call", [{ content: [{ type: "text", text: "rows" }] }]]]),
						(_method, params) => {
							recoveredCalls++;
							retryKeys.push(sentIdempotencyKey(params));
						},
					),
				);
				const tool = deferred
					? new DeferredMCPTool(
							connection.name,
							readDefinition,
							async () => connection,
							undefined,
							async () => recovered,
						)
					: new MCPTool(connection, readDefinition, async () => recovered);
				// Reads cannot double-execute, so one reconnect+replay recovers them.
				const result = await tool.execute("read-1", {}, undefined, context);
				expect(firstAttempts).toBe(1);
				expect(recoveredCalls).toBe(1);
				expect(result.isError).not.toBe(true);
				expect(result.content).toEqual([{ type: "text", text: "rows" }]);
				// The stable call key travels on both attempts for server-side dedupe.
				expect(firstKeys).toEqual(["read-1"]);
				expect(retryKeys).toEqual(["read-1"]);
			});

			it(`${mode}: a write replays once when the server scopes the failure to an idempotency key`, async () => {
				const firstKeys: unknown[] = [];
				const retryKeys: unknown[] = [];
				const transport = createMockTransport(new Map(), (_method, params) => {
					firstKeys.push(sentIdempotencyKey(params));
					throw new MCPTransportError({
						transport: "http",
						stage: "receive",
						failure: "http_status",
						code: 503,
						retryable: true,
						message: "HTTP 503: Service Unavailable",
						idempotencyKey: "op-123",
					});
				});
				const connection = createMockConnection({ tools: {} }, transport);
				const recovered = createMockConnection(
					{ tools: {} },
					createMockTransport(
						new Map([["tools/call", [{ content: [{ type: "text", text: "published" }] }]]]),
						(_method, params) => {
							retryKeys.push(sentIdempotencyKey(params));
						},
					),
				);
				const tool = deferred
					? new DeferredMCPTool(
							connection.name,
							definition,
							async () => connection,
							undefined,
							async () => recovered,
						)
					: new MCPTool(connection, definition, async () => recovered);
				// The server opted this operation into dedupe, so echoing its
				// key on exactly one retry cannot double-execute.
				const result = await tool.execute("publish-key-1", {}, undefined, context);
				expect(result.isError).not.toBe(true);
				expect(result.details?.outcomeUnknown).toBeUndefined();
				expect(firstKeys).toEqual(["publish-key-1"]);
				expect(retryKeys).toEqual(["op-123"]);
			});

			it(`${mode}: a write timeout without a server key is not replayed`, async () => {
				let attempts = 0;
				let reconnects = 0;
				const transport = createMockTransport(new Map(), () => {
					attempts++;
					throw new MCPTransportError({
						transport: "http",
						stage: "receive",
						failure: "timeout",
						retryable: false,
						message: "Request timeout after 50ms",
					});
				});
				const connection = createMockConnection({ tools: {} }, transport);
				const tool = deferred
					? new DeferredMCPTool(
							connection.name,
							definition,
							async () => connection,
							undefined,
							async () => {
								reconnects++;
								throw new Error("must not reconnect after uncertain delivery");
							},
						)
					: new MCPTool(connection, definition, async () => {
							reconnects++;
							throw new Error("must not reconnect after uncertain delivery");
						});
				const result = await tool.execute("publish-timeout-1", {}, undefined, context);
				expect(attempts).toBe(1);
				expect(reconnects).toBe(0);
				expect(result.isError).toBe(true);
				expect(result.details?.outcomeUnknown).toBe(true);
			});
		}

		it("a read-only tool does not replay a deterministic server rejection", async () => {
			let calls = 0;
			let reconnects = 0;
			const transport = createMockTransport(new Map(), () => {
				calls++;
				throw new MCPTransportError({
					transport: "http",
					stage: "protocol",
					failure: "json_rpc",
					retryable: false,
					message: "MCP error -32602: Invalid params",
					code: -32602,
				});
			});
			const connection = createMockConnection({ tools: {} }, transport);
			const tool = new MCPTool(connection, readDefinition, async () => {
				reconnects++;
				throw new Error("must not reconnect after a deterministic rejection");
			});
			const result = await tool.execute("read-rejected-1", {}, undefined, context);
			expect(calls).toBe(1);
			expect(reconnects).toBe(0);
			expect(result.isError).toBe(true);
			expect(result.details?.outcomeUnknown).toBeUndefined();
		});
	});

	it("recovers a deferred connection failure before tools/call is attempted", async () => {
		let effects = 0;
		const recovered = createMockConnection(
			{ tools: {} },
			createMockTransport(new Map([["tools/call", [{ content: [{ type: "text", text: "published" }] }]]]), () => {
				effects++;
			}),
		);
		const tool = new DeferredMCPTool(
			"publish-server",
			definition,
			async () => {
				throw new Error("MCP server not connected");
			},
			undefined,
			async () => recovered,
		);
		const result = await tool.execute("publish-4", {}, undefined, context);
		expect(effects).toBe(1);
		expect(result.isError).not.toBe(true);
	});

	describe("post-send failure classes never replay", () => {
		const cases: Array<{ label: string; failure: Record<string, unknown> }> = [
			{
				label: "http reset after send",
				failure: { transport: "http", stage: "send", failure: "reset", retryable: true, message: "ECONNRESET" },
			},
			{
				label: "http 502 after send",
				failure: {
					transport: "http",
					stage: "receive",
					failure: "http_status",
					code: 502,
					retryable: true,
					message: "HTTP 502: Bad Gateway",
				},
			},
			{
				label: "http 503 after send",
				failure: {
					transport: "http",
					stage: "receive",
					failure: "http_status",
					code: 503,
					retryable: true,
					message: "HTTP 503: Service Unavailable",
				},
			},
			{
				label: "response timeout",
				failure: {
					transport: "http",
					stage: "receive",
					failure: "timeout",
					retryable: false,
					message: "Request timeout after 50ms",
				},
			},
		];
		for (const deferred of [false, true]) {
			const mode = deferred ? "deferred" : "eager";
			for (const { label, failure } of cases) {
				it(`${mode}: ${label} reports outcome unknown without replay`, async () => {
					let effects = 0;
					let reconnects = 0;
					const transport = createMockTransport(new Map(), () => {
						effects++;
						throw new MCPTransportError(failure as never);
					});
					const connection = createMockConnection({ tools: {} }, transport);
					const tool = deferred
						? new DeferredMCPTool(
								connection.name,
								definition,
								async () => connection,
								undefined,
								async () => {
									reconnects++;
									throw new Error("must not reconnect after uncertain delivery");
								},
							)
						: new MCPTool(connection, definition, async () => {
								reconnects++;
								throw new Error("must not reconnect after uncertain delivery");
							});
					const result = await tool.execute("publish-x", {}, undefined, context);
					expect(effects).toBe(1);
					expect(reconnects).toBe(0);
					expect(result.isError).toBe(true);
					expect(result.details?.outcomeUnknown).toBe(true);
				});
			}
		}
	});

	for (const deferred of [false, true]) {
		const mode = deferred ? "deferred" : "eager";
		it(`${mode}: mid-flight cancellation throws without replay or error result`, async () => {
			let effects = 0;
			let reconnects = 0;
			const controller = new AbortController();
			const transport = createMockTransport(new Map(), () => {
				effects++;
				controller.abort();
				throw new DOMException("The operation was aborted", "AbortError");
			});
			const connection = createMockConnection({ tools: {} }, transport);
			const tool = deferred
				? new DeferredMCPTool(
						connection.name,
						definition,
						async () => connection,
						undefined,
						async () => {
							reconnects++;
							throw new Error("must not reconnect after cancellation");
						},
					)
				: new MCPTool(connection, definition, async () => {
						reconnects++;
						throw new Error("must not reconnect after cancellation");
					});
			await expect(tool.execute("publish-x", {}, undefined, context, controller.signal)).rejects.toThrow();
			expect(effects).toBe(1);
			expect(reconnects).toBe(0);
		});

		it(`${mode}: OAuth challenge recovery executes exactly once on the fresh connection`, async () => {
			let firstCalls = 0;
			let recoveredCalls = 0;
			const first = createMockConnection(
				{ tools: {} },
				createMockTransport(
					new Map([
						[
							"tools/call",
							[
								{
									content: [{ type: "text", text: "unauthorized" }],
									isError: true,
									_meta: { "mcp/www_authenticate": ["Bearer realm=test"] },
								},
							],
						],
					]),
					() => {
						firstCalls++;
					},
				),
			);
			const recovered = createMockConnection(
				{ tools: {} },
				createMockTransport(new Map([["tools/call", [{ content: [{ type: "text", text: "published" }] }]]]), () => {
					recoveredCalls++;
				}),
			);
			const tool = deferred
				? new DeferredMCPTool(
						first.name,
						definition,
						async () => first,
						undefined,
						async () => recovered,
					)
				: new MCPTool(first, definition, async () => recovered);
			// The 401 challenge definitively rejected the first attempt, so one
			// replay on the re-authenticated connection is safe.
			const result = await tool.execute("publish-x", {}, undefined, context);
			expect(firstCalls).toBe(1);
			expect(recoveredCalls).toBe(1);
			expect(result.isError).not.toBe(true);
			expect(result.details?.outcomeUnknown).toBeUndefined();
		});
	}
});
