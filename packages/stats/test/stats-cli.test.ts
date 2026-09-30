import { describe, expect, it, spyOn } from "bun:test";
import { emitSyncSummary, parseStandaloneStatsArgs } from "../src/index";
import { formatStatsDashboardUrl } from "../src/server";

describe("standalone stats CLI", () => {
	it("keeps the production parser loopback-only by default", () => {
		expect(parseStandaloneStatsArgs([])).toEqual({
			port: 3847,
			host: "127.0.0.1",
			json: false,
			sync: false,
			help: false,
			token: null,
		});
	});

	it("forwards explicit bind hosts and formats IPv6 dashboard URLs", () => {
		expect(parseStandaloneStatsArgs(["--host", "::", "--port", "3850"])).toMatchObject({
			port: 3850,
			host: "::",
		});
		expect(formatStatsDashboardUrl("::", 3850)).toBe("http://[::]:3850");
		expect(formatStatsDashboardUrl("2001:db8::1", 3850)).toBe("http://[2001:db8::1]:3850");
	});

	it("keeps stdout JSON-parseable by sending the summary to stderr in json mode", () => {
		const logSpy = spyOn(console, "log").mockImplementation(() => {});
		const errSpy = spyOn(process.stderr, "write").mockImplementation(() => true as never);
		try {
			emitSyncSummary(3, 2, 10, true);
			expect(logSpy).not.toHaveBeenCalled();
			expect(errSpy).toHaveBeenCalledTimes(1);
			expect(String(errSpy.mock.calls[0]?.[0] ?? "")).toContain("Synced 3 new entries");

			logSpy.mockClear();
			errSpy.mockClear();
			emitSyncSummary(3, 2, 10, false);
			expect(logSpy).toHaveBeenCalledTimes(1);
			expect(errSpy).not.toHaveBeenCalled();
		} finally {
			logSpy.mockRestore();
			errSpy.mockRestore();
		}
	});
});
