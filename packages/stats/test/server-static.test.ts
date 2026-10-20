import { afterEach, describe, expect, it } from "bun:test";
import { TempDir } from "@harvest/pi-utils";
import { handleStatic } from "../src/server";

const directories: TempDir[] = [];

async function fixture(): Promise<TempDir> {
	const directory = await TempDir.create("@omp-stats-static-");
	directories.push(directory);
	await Bun.write(directory.join("index.html"), "<main>stats dashboard</main>");
	return directory;
}

afterEach(async () => {
	await Promise.all(directories.splice(0).map(directory => directory.remove()));
});

describe("stats dashboard static responses", () => {
	it("keeps missing assets as 404s while preserving extensionless SPA navigation", async () => {
		const directory = await fixture();
		for (const asset of ["dashboard.wasm", "app.js", "styles.css", "logo.svg", "app.js.map"]) {
			const missing = await handleStatic(`/assets/${asset}`, directory.path());
			expect(missing.status).toBe(404);
			expect(await missing.text()).toBe("Not Found");
		}
		const route = await handleStatic("/traces/session", directory.path());
		expect(route.status).toBe(200);
		expect(await route.text()).toBe("<main>stats dashboard</main>");
	});

	it("serves a valid WASM module with the MIME type required by streaming instantiation", async () => {
		const directory = await fixture();
		const bytes = new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0]);
		await Bun.write(directory.join("assets/dashboard.wasm"), bytes);
		const response = await handleStatic("/assets/dashboard.wasm", directory.path());
		expect(response.status).toBe(200);
		expect(response.headers.get("content-type")).toBe("application/wasm");
		const module = await WebAssembly.compileStreaming(Promise.resolve(response));
		expect(WebAssembly.Module.exports(module)).toEqual([]);
	});
});
