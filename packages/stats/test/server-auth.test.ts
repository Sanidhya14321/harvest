import { afterEach, describe, expect, it } from "bun:test";
import { startServer, STATS_TOKEN_ENV } from "../src/server";

/**
 * U6: a stats dashboard bound off loopback must require credentials, and
 * API reads must reject unauthenticated requests when a token is set.
 *
 * These tests never touch the stats DB: refusal throws before binding and
 * 401s short-circuit before the aggregator, so no agent-dir isolation (and
 * its Windows file-lock teardown) is involved. Authorized-200 and the
 * loopback-open default are covered by server-port-conflict.test.ts.
 */
describe("stats dashboard authentication", () => {
	const priorToken = process.env[STATS_TOKEN_ENV];

	afterEach(() => {
		if (priorToken === undefined) delete process.env[STATS_TOKEN_ENV];
		else process.env[STATS_TOKEN_ENV] = priorToken;
	});

	it("refuses a non-loopback bind without credentials before listening", async () => {
		delete process.env[STATS_TOKEN_ENV];
		await expect(startServer(0, "0.0.0.0")).rejects.toThrow("refusing non-loopback bind '0.0.0.0' without a token");
	});

	it("rejects unauthenticated API reads when a token is configured", async () => {
		delete process.env[STATS_TOKEN_ENV];
		const server = await startServer(0, "127.0.0.1", { token: "test-token" });
		try {
			const url = `http://127.0.0.1:${server.port}/api/stats/models`;

			const denied = await fetch(url);
			expect(denied.status).toBe(401);
			expect(await denied.json()).toEqual({ error: "unauthorized" });

			const wrong = await fetch(url, { headers: { authorization: "Bearer wrong-token" } });
			expect(wrong.status).toBe(401);
			await wrong.body?.cancel();
		} finally {
			server.stop();
		}
	});
});
