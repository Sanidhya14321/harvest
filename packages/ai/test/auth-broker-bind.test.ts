import { afterEach, beforeEach, describe, expect, it } from "bun:test";
import * as fs from "node:fs/promises";
import * as os from "node:os";
import * as path from "node:path";
import { AuthStorage, SqliteAuthCredentialStore } from "@harvest/pi-ai/auth-storage";
import { startAuthBroker, type AuthBrokerServerHandle } from "@harvest/pi-ai/auth-broker";

/**
 * Bind guard: an unauthenticated broker must never listen off-loopback.
 * The refusal throws before Bun.serve binds, so the non-loopback case
 * asserts without touching a real interface.
 */
describe("auth-broker bind guard", () => {
	let tempDir: string;
	let store: SqliteAuthCredentialStore;
	let storage: AuthStorage;
	let handle: AuthBrokerServerHandle | undefined;

	beforeEach(async () => {
		tempDir = await fs.mkdtemp(path.join(os.tmpdir(), "auth-broker-bind-"));
		store = await SqliteAuthCredentialStore.open(path.join(tempDir, "agent.db"));
		storage = new AuthStorage(store);
		await storage.reload();
	});

	afterEach(async () => {
		await handle?.close();
		handle = undefined;
		storage?.close();
		store?.close();
		await fs.rm(tempDir, { recursive: true, force: true });
	});

	it("refuses an unauthenticated non-loopback bind before listening", () => {
		expect(() => startAuthBroker({ storage, bind: "0.0.0.0:0", bearerTokens: [], disableRefresher: true })).toThrow(
			/non-loopback/,
		);
	});

	it("stays open on loopback without a token, including /v1/healthz", async () => {
		handle = startAuthBroker({ storage, bind: "127.0.0.1:0", bearerTokens: [], disableRefresher: true });
		const health = await fetch(`${handle.url}/v1/healthz`);
		expect(health.status).toBe(200);
		expect(await health.json()).toMatchObject({ ok: true });
	});
});
