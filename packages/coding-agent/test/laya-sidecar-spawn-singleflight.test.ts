import { describe, expect, it } from "bun:test";
import { singleflight } from "../src/core/harvest/laya-service";

/**
 * Contracts: concurrent starters for one key share a single run (no duplicate
 * sidecar spawns behind a port drift); keys stay isolated; the slot clears on
 * success and on failure so later starters run fresh; failures reach every
 * sharer; a synchronously throwing start never poisons the map.
 *
 * Tested against the helper directly: exercising it through
 * startLayaSidecarProcess would execute the real spawn pipeline.
 */
describe("singleflight", () => {
	it("shares one in-flight run between concurrent starters", async () => {
		const runs = new Map<string, Promise<string>>();
		const gate = Promise.withResolvers<void>();
		let starts = 0;
		const start = () => {
			starts++;
			return gate.promise.then(() => "spawned");
		};
		const first = singleflight(runs, "http://127.0.0.1:8177", start);
		const second = singleflight(runs, "http://127.0.0.1:8177", start);
		gate.resolve();
		expect(await Promise.all([first, second])).toEqual(["spawned", "spawned"]);
		expect(starts).toBe(1);
	});

	it("isolates keys and starts fresh after the run settles", async () => {
		const runs = new Map<string, Promise<string>>();
		let starts = 0;
		const start = async () => `run-${++starts}`;
		expect(await singleflight(runs, "a", start)).toBe("run-1");
		expect(await singleflight(runs, "b", start)).toBe("run-2");
		expect(await singleflight(runs, "a", start)).toBe("run-3");
		expect(starts).toBe(3);
		expect(runs.size).toBe(0);
	});

	it("clears the slot on failure and delivers the rejection to every sharer", async () => {
		const runs = new Map<string, Promise<string>>();
		const gate = Promise.withResolvers<void>();
		let starts = 0;
		const start = () => {
			starts++;
			return gate.promise.then(() => {
				throw new Error("spawn failed");
			});
		};
		const first = singleflight(runs, "a", start);
		const second = singleflight(runs, "a", start);
		gate.resolve();
		await expect(first).rejects.toThrow("spawn failed");
		await expect(second).rejects.toThrow("spawn failed");
		expect(starts).toBe(1);
		let settled = false;
		await singleflight(runs, "a", async () => {
			settled = true;
			return "recovered";
		});
		expect(settled).toBe(true);
	});

	it("propagates a synchronously throwing start without poisoning the map", () => {
		const runs = new Map<string, Promise<string>>();
		expect(() =>
			singleflight(runs, "a", () => {
				throw new Error("sync boom");
			}),
		).toThrow("sync boom");
		expect(runs.size).toBe(0);
	});
});
