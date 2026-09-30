import { afterEach, describe, expect, it } from "bun:test";
import * as fs from "node:fs/promises";
import * as os from "node:os";
import * as path from "node:path";
import { getLayaClient, LayaClient, resetLayaClient } from "../src/core/harvest/laya-client";

describe("Laya sidecar authentication", () => {
	let server: ReturnType<typeof Bun.serve> | undefined;
	let tempDir: string | undefined;

	afterEach(async () => {
		server?.stop(true);
		if (tempDir) await fs.rm(tempDir, { recursive: true, force: true });
	});

	it("sends the managed token and follows rotation after sidecar restart", async () => {
		tempDir = await fs.mkdtemp(path.join(os.tmpdir(), "harvest-laya-auth-"));
		const tokenFilePath = path.join(tempDir, "token");
		await Bun.write(tokenFilePath, "first-token");
		let expectedToken = "first-token";
		const received: string[] = [];
		server = Bun.serve({
			hostname: "127.0.0.1",
			port: 0,
			fetch(request) {
				const authorization = request.headers.get("authorization") ?? "";
				received.push(authorization);
				if (authorization !== `Bearer ${expectedToken}`) return new Response(null, { status: 401 });
				return Response.json({
					answers: { ping: { type: "noul", noul: 0.1, confidence: 0.9 } },
					latency_ms: 1,
					model: "test",
				});
			},
		});
		const client = new LayaClient({
			baseUrl: server.url.toString().replace(/\/$/, ""),
			tokenFilePath,
		});
		const questions = { ping: { type: "noul" as const, instructions: "Is this a check?" } };
		expect((await client.decide("check", questions, { callSite: "test" })).success).toBe(true);

		expectedToken = "second-token";
		await Bun.write(tokenFilePath, expectedToken);
		expect((await client.decide("check", questions, { callSite: "test" })).success).toBe(true);
		expect(received).toEqual(["Bearer first-token", "Bearer second-token"]);
	});
});

describe("Laya client singleton URL tracking", () => {
	afterEach(() => {
		delete process.env.LAYA_SIDECAR_URL;
		resetLayaClient();
	});

	it("reuses the default while the effective URL is unchanged", () => {
		process.env.LAYA_SIDECAR_URL = "http://127.0.0.1:8177";
		const first = getLayaClient();
		const second = getLayaClient();
		expect(second).toBe(first);
		expect(first.baseUrl).toBe("http://127.0.0.1:8177");
	});

	it("rebuilds the default when the effective URL moves (e.g. setup port fallback)", () => {
		process.env.LAYA_SIDECAR_URL = "http://127.0.0.1:8177";
		const before = getLayaClient();
		process.env.LAYA_SIDECAR_URL = "http://127.0.0.1:8178";
		const after = getLayaClient();
		expect(after).not.toBe(before);
		expect(after.baseUrl).toBe("http://127.0.0.1:8178");
	});

	it("serves an explicit URL without disturbing the shared default", () => {
		process.env.LAYA_SIDECAR_URL = "http://127.0.0.1:8177";
		const shared = getLayaClient();
		const explicit = getLayaClient("http://127.0.0.1:8179");
		expect(explicit.baseUrl).toBe("http://127.0.0.1:8179");
		expect(getLayaClient()).toBe(shared);
	});
});
