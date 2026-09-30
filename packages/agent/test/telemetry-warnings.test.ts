import { afterEach, describe, expect, it, vi } from "bun:test";
import { logger } from "@harvest/pi-utils";
import { recordTelemetryWarning } from "../src/telemetry";

/**
 * P2-9: shared telemetry warnings must never write to the terminal.
 * Without a hook (or when the hook throws), they route through the
 * centralized logger instead of console.warn.
 */
describe("telemetry warning sink", () => {
	afterEach(() => {
		vi.restoreAllMocks();
	});

	it("routes hook-less warnings to the logger instead of the terminal", () => {
		const warn = vi.spyOn(logger, "warn").mockImplementation(() => {});
		const terminal = vi.spyOn(console, "warn").mockImplementation(() => {});
		try {
			recordTelemetryWarning(undefined, { code: "cost_estimator_failed", message: "estimator blew up" });
			expect(warn).toHaveBeenCalledTimes(1);
			expect(String(warn.mock.calls[0]?.[0] ?? "")).toContain("estimator blew up");
			expect(terminal).not.toHaveBeenCalled();
		} finally {
			warn.mockRestore();
			terminal.mockRestore();
		}
	});

	it("routes a throwing hook's failure to the logger instead of the terminal", () => {
		const warn = vi.spyOn(logger, "warn").mockImplementation(() => {});
		const terminal = vi.spyOn(console, "warn").mockImplementation(() => {});
		try {
			recordTelemetryWarning(
				{
					config: {
						onTelemetryWarning: () => {
							throw new Error("hook blew up");
						},
					},
				} as never,
				{ code: "on_run_end_failed", message: "run end blew up" },
			);
			expect(warn).toHaveBeenCalledTimes(1);
			expect(terminal).not.toHaveBeenCalled();
		} finally {
			warn.mockRestore();
			terminal.mockRestore();
		}
	});
});
