import * as path from "node:path";
import { expect, it } from "bun:test";
import { TempDir } from "@harvest/pi-utils";

it("retains immediate init messages while the CLI imports its isolated evaluator", async () => {
	const directory = TempDir.createSync("@harvest-js-lazy-init-");
	const ready = Promise.withResolvers<void>();
	const proc = Bun.spawn(
		[process.execPath, path.resolve(import.meta.dir, "../../src/cli.ts"), "__harvest_worker_js_eval_process"],
		{
			cwd: directory.path(),
			env: { ...process.env, PI_CODING_AGENT_DIR: path.join(directory.path(), "agent") },
			stdin: "ignore",
			stdout: "pipe",
			stderr: "pipe",
			serialization: "advanced",
			ipc(message: unknown) {
				if (typeof message === "object" && message !== null && Reflect.get(message, "type") === "ready")
					ready.resolve();
			},
		},
	);
	try {
		proc.send({ type: "init", snapshot: { cwd: directory.path(), sessionId: "lazy-init" } });
		const outcome = await Promise.race([
			ready.promise.then(() => "ready"),
			proc.exited.then(() => "exited"),
			Bun.sleep(2_000).then(() => "timeout"),
		]);
		expect(outcome).toBe("ready");
	} finally {
		proc.kill("SIGKILL");
		await proc.exited;
		await directory.remove();
	}
});
