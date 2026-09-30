import * as path from "node:path";

const SIDECAR_FILES = [
	"server.py",
	"hardware.py",
	"bucketing.py",
	"calibration.py",
	"calibration_params.json",
	"requirements.txt",
] as const;

/** Embed the runnable Python sidecar in npm bundles and standalone binaries. */
export async function buildLayaSidecarPayload(): Promise<string> {
	const sidecarDir = path.resolve(import.meta.dir, "../../../decision-sidecar");
	const files: Record<string, string> = {};
	for (const name of SIDECAR_FILES) files[name] = await Bun.file(path.join(sidecarDir, name)).text();
	return Buffer.from(Bun.gzipSync(new TextEncoder().encode(JSON.stringify(files)))).toString("base64");
}
