import { describe, expect, it } from "bun:test";
import {
	annotateMcpDetails,
	McpOutcomeLedger,
	outcomeDetailsFor,
} from "@harvest/pi-coding-agent/mcp/mcp-outcome-ledger";

describe("mcp outcome ledger", () => {
	it("records and looks up committed/unknown/failed outcomes by idempotency key", () => {
		const ledger = new McpOutcomeLedger();
		expect(
			ledger.record({ key: "call-1", status: "committed", serverName: "billing", toolName: "publish" })?.status,
		).toBe("committed");
		expect(
			ledger.record({
				key: "call-2",
				status: "unknown",
				serverName: "billing",
				toolName: "publish",
				failure: "eof after commit",
			})?.status,
		).toBe("unknown");
		expect(
			ledger.record({ key: "call-3", status: "failed", serverName: "billing", toolName: "publish" })?.status,
		).toBe("failed");
		expect(ledger.lookup("call-2")).toMatchObject({
			key: "call-2",
			status: "unknown",
			serverName: "billing",
			toolName: "publish",
			failure: "eof after commit",
		});
		expect(ledger.lookup("missing")).toBeUndefined();
	});

	it("surfaces the outcome in result details without mutating the input", () => {
		const ledger = new McpOutcomeLedger();
		ledger.record({ key: "call-9", status: "unknown", serverName: "billing", toolName: "publish" });
		const details = { serverName: "billing", mcpToolName: "publish", isError: true };
		const annotated = annotateMcpDetails(details, ledger, "call-9");
		expect(annotated).toMatchObject({ outcome: "unknown", outcomeKey: "call-9" });
		expect(typeof annotated.outcomeAt).toBe("number");
		// Input untouched; unknown keys add no fields.
		expect(details).not.toHaveProperty("outcome");
		expect(outcomeDetailsFor(ledger, "missing")).toEqual({});
		expect(annotateMcpDetails(details, ledger, "missing")).toEqual(details);
	});

	it("evicts the oldest entry past capacity", () => {
		const ledger = new McpOutcomeLedger(3);
		for (const key of ["k1", "k2", "k3", "k4"]) {
			ledger.record({ key, status: "committed", serverName: "s", toolName: "t" });
		}
		expect(ledger.size).toBe(3);
		expect(ledger.lookup("k1")).toBeUndefined();
		expect(ledger.entries().map(entry => entry.key)).toEqual(["k2", "k3", "k4"]);
	});

	it("drops invalid keys fail-closed without recording", () => {
		const ledger = new McpOutcomeLedger();
		expect(ledger.record({ key: "   ", status: "unknown", serverName: "s", toolName: "t" })).toBeUndefined();
		expect(ledger.record({ key: "has space", status: "unknown", serverName: "s", toolName: "t" })).toBeUndefined();
		expect(ledger.record({ key: "ok-1", status: "unknown", serverName: "", toolName: "t" })).toBeUndefined();
		expect(ledger.size).toBe(0);
		expect(ledger.lookup("   ")).toBeUndefined();
	});
});
