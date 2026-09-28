import { afterEach, describe, expect, it, vi } from "bun:test";
import * as layaService from "../src/core/harvest/laya-service";

/**
 * Contracts under test:
 *  - Concurrent configureLayaLocally calls for the same base URL share one
 *    in-flight pipeline run (singleflight) instead of spawning duplicate
 *    sidecar children behind a port drift.
 *  - A failed run clears the singleflight slot so the next call retries.
 */

afterEach(() => {
	vi.restoreAllMocks();
});

function deferred<T>(): { promise: Promise<T>; resolve: (v: T) => void } {
	let resolve!: (v: T) => void;
	const promise = new Promise<T>(res => {
		resolve = res;
	});
	return { promise, resolve };
}

describe("configureLayaLocally singleflight", () => {
	it("shares one in-flight pipeline run across concurrent callers", async () => {
		const gate = deferred<layaService.LayaSetupResult>();
		const spy = vi.spyOn(layaService, "runLayaConfiguration").mockImplementation(() => gate.promise);

		const first = layaService.configureLayaLocally({ baseUrl: "http://127.0.0.1:8199" });
		const second = layaService.configureLayaLocally({ baseUrl: "http://127.0.0.1:8199" });
		expect(spy).toHaveBeenCalledTimes(1);

		gate.resolve({ success: true, coreHarvestReady: true, effectiveUrl: "http://127.0.0.1:8199" });
		const [a, b] = await Promise.all([first, second]);
		expect(spy).toHaveBeenCalledTimes(1);
		expect(a).toBe(b);
	});

	it("retries after a failed run instead of caching the failure", async () => {
		const spy = vi
			.spyOn(layaService, "runLayaConfiguration")
			.mockResolvedValueOnce({ success: false, error: "boom" })
			.mockResolvedValueOnce({ success: true, coreHarvestReady: true });

		const first = await layaService.configureLayaLocally({ baseUrl: "http://127.0.0.1:8198" });
		expect(first.success).toBe(false);
		const second = await layaService.configureLayaLocally({ baseUrl: "http://127.0.0.1:8198" });
		expect(second.success).toBe(true);
		expect(spy).toHaveBeenCalledTimes(2);
	});
});
