import { afterEach, expect, test } from "bun:test";
import * as fs from "node:fs/promises";
import * as os from "node:os";
import * as path from "node:path";
import { startAuthGateway } from "@harvest/pi-ai/auth-gateway";
import type { AuthGatewayServerHandle } from "@harvest/pi-ai/auth-gateway";
import { AuthStorage } from "@harvest/pi-ai/auth-storage";
import { corsHeaders, withCors } from "@harvest/pi-ai/auth-gateway/http";

/**
 * Contracts under test:
 *  - In open (no-auth/bench) mode the gateway omits
 *    `Access-Control-Allow-Origin: *` so browsers can't treat it as a public
 *    cross-origin API — on both preflight and regular responses.
 *  - Authenticated mode keeps the existing wildcard CORS surface.
 *  - /healthz stays reachable without a token in both modes.
 */

const cleanups: Array<() => Promise<void>> = [];
afterEach(async () => {
	while (cleanups.length) await cleanups.pop()?.();
});

async function boot(
	bearerTokens: string[],
): Promise<{ url: string; storage: AuthStorage; handle: AuthGatewayServerHandle }> {
	const dir = await fs.mkdtemp(path.join(os.tmpdir(), "gw-cors-"));
	const storage = await AuthStorage.create(path.join(dir, "auth.db"));
	const handle = startAuthGateway({
		bind: "127.0.0.1:0",
		bearerTokens,
		storage,
		resolveModel: () => undefined,
		version: "test",
	});
	cleanups.push(async () => {
		await handle.close();
		storage.close();
		await fs.rm(dir, { recursive: true, force: true });
	});
	return { url: handle.url, storage, handle };
}

function dummyReq(): Request {
	return new Request("http://127.0.0.1:4000/v1/models");
}

test("corsHeaders omits the wildcard origin in open mode, keeps it when authenticated", () => {
	expect(corsHeaders(dummyReq())["Access-Control-Allow-Origin"]).toBe("*");
	expect(corsHeaders(dummyReq(), { authenticated: true })["Access-Control-Allow-Origin"]).toBe("*");
	const open = corsHeaders(dummyReq(), { authenticated: false });
	expect("Access-Control-Allow-Origin" in open).toBe(false);
	// The rest of the allow-list surface is preserved.
	expect(open["Access-Control-Allow-Methods"]).toContain("POST");
});

test("withCors omits the wildcard origin in open mode", () => {
	const makeBase = () =>
		new Response(JSON.stringify({ ok: true }), { headers: { "Content-Type": "application/json" } });
	const open = withCors(makeBase(), dummyReq(), { authenticated: false });
	expect(open.headers.get("Access-Control-Allow-Origin")).toBeNull();
	const authed = withCors(makeBase(), dummyReq(), { authenticated: true });
	expect(authed.headers.get("Access-Control-Allow-Origin")).toBe("*");
});

test("open mode: preflight and responses carry no wildcard origin, /healthz stays reachable", async () => {
	const { url } = await boot([]);

	const preflight = await fetch(`${url}/v1/chat/completions`, { method: "OPTIONS" });
	expect(preflight.status).toBe(204);
	expect(preflight.headers.get("Access-Control-Allow-Origin")).toBeNull();

	const health = await fetch(`${url}/healthz`);
	expect(health.status).toBe(200);
	expect((await health.json()) as unknown).toMatchObject({ ok: true });
	expect(health.headers.get("Access-Control-Allow-Origin")).toBeNull();

	const models = await fetch(`${url}/v1/models`);
	expect(models.status).toBe(200);
	expect(models.headers.get("Access-Control-Allow-Origin")).toBeNull();
});

test("authenticated mode: wildcard origin present, /healthz stays reachable without a token", async () => {
	const { url } = await boot(["secret"]);

	const preflight = await fetch(`${url}/v1/chat/completions`, { method: "OPTIONS" });
	expect(preflight.status).toBe(204);
	expect(preflight.headers.get("Access-Control-Allow-Origin")).toBe("*");

	const health = await fetch(`${url}/healthz`);
	expect(health.status).toBe(200);
	expect((await health.json()) as unknown).toMatchObject({ ok: true });

	const unauth = await fetch(`${url}/v1/models`);
	expect(unauth.status).toBe(401);
	expect(unauth.headers.get("Access-Control-Allow-Origin")).toBe("*");
});
