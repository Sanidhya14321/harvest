import { describe, expect, it } from "bun:test";
import * as fs from "node:fs/promises";
import * as os from "node:os";
import * as path from "node:path";
import { buildLayaSidecarPayload } from "../scripts/laya-sidecar-payload";
import { materializeBundledLayaSidecar } from "../src/core/harvest/laya-service";

describe("bundled Laya sidecar", () => {
	it("materializes runnable sources outside a source checkout", async () => {
		const agentDir = await fs.mkdtemp(path.join(os.tmpdir(), "harvest-laya-bundle-"));
		try {
			const payload = await buildLayaSidecarPayload();
			const directory = materializeBundledLayaSidecar(payload, agentDir);
			expect(materializeBundledLayaSidecar(payload, agentDir)).toBe(directory);
			for (const name of [
				"server.py",
				"hardware.py",
				"bucketing.py",
				"calibration.py",
				"requirements.txt",
				"calibration_params.json",
			]) {
				expect((await fs.stat(path.join(directory, name))).isFile()).toBe(true);
			}
		} finally {
			await fs.rm(agentDir, { recursive: true, force: true });
		}
	});
});
