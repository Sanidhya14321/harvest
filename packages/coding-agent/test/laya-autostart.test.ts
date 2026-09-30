import { afterEach, describe, expect, it } from "bun:test";
import { autostartInstalledLayaSidecar } from "../src/core/harvest/laya-service";
import { Settings } from "../src/config/settings";

/**
 * Autostart guard rails (plan §4.2): ordinary startup must never block on
 * Laya model loading. Autostart returns immediately when disabled, and when
 * a healthy sidecar already answers it reuses the instance instead of
 * spawning — so these paths never reach downloads, installs, or spawns.
 */
describe("autostartInstalledLayaSidecar guards", () => {
	let server: ReturnType<typeof Bun.serve> | undefined;

	afterEach(() => {
		server?.stop(true);
		server = undefined;
	});

	it("does nothing fast when Laya is disabled", async () => {
		let hits = 0;
		server = Bun.serve({
			hostname: "127.0.0.1",
			port: 0,
			fetch() {
				hits++;
				return Response.json({ status: "ok" });
			},
		});
		const settings = Settings.isolated({
			"laya.enabled": false,
			"laya.autostart": true,
			"laya.url": server.url.toString().replace(/\/$/, ""),
		});
		const start = performance.now();
		await autostartInstalledLayaSidecar(settings);
		expect(performance.now() - start).toBeLessThan(5000);
		expect(hits).toBe(0);
	});

	it("does nothing fast when autostart is off", async () => {
		let hits = 0;
		server = Bun.serve({
			hostname: "127.0.0.1",
			port: 0,
			fetch() {
				hits++;
				return Response.json({ status: "ok" });
			},
		});
		const settings = Settings.isolated({
			"laya.enabled": true,
			"laya.autostart": false,
			"laya.url": server.url.toString().replace(/\/$/, ""),
		});
		await autostartInstalledLayaSidecar(settings);
		expect(hits).toBe(0);
	});

	it("reuses a healthy sidecar without spawning or drifting the URL", async () => {
		let healthHits = 0;
		server = Bun.serve({
			hostname: "127.0.0.1",
			port: 0,
			fetch(request) {
				if (new URL(request.url).pathname === "/health") healthHits++;
				return Response.json({ status: "ok" });
			},
		});
		const baseUrl = server.url.toString().replace(/\/$/, "");
		const settings = Settings.isolated({
			"laya.enabled": true,
			"laya.autostart": true,
			"laya.url": baseUrl,
		});
		const start = performance.now();
		await autostartInstalledLayaSidecar(settings);
		expect(performance.now() - start).toBeLessThan(5000);
		expect(healthHits).toBeGreaterThanOrEqual(1);
		expect(settings.get("laya.url")).toBe(baseUrl);
	});
});
