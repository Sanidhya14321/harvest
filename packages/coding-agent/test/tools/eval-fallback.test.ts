import { afterEach, beforeEach, describe, expect, it, vi } from "bun:test";
import { Settings } from "@harvest/pi-coding-agent/config/settings";
import * as evalIndex from "@harvest/pi-coding-agent/eval";
import type { ToolSession } from "@harvest/pi-coding-agent/tools";
import { EvalRunner, type EvalRunnerHost } from "../../src/session/eval-runner";
import { EvalTool } from "@harvest/pi-coding-agent/tools/eval";
import { resolveEvalBackends } from "@harvest/pi-coding-agent/tools/eval-backends";

let originalPiPy: string | undefined;
let originalPiJs: string | undefined;

function restoreEnv(name: "PI_PY" | "PI_JS", value: string | undefined): void {
	if (value === undefined) {
		delete Bun.env[name];
		return;
	}
	Bun.env[name] = value;
}
function makeSession(settings = Settings.isolated()): ToolSession {
	return {
		cwd: "/tmp/eval-test",
		hasUI: false,
		getSessionFile: () => null,
		getSessionSpawns: () => null,
		settings,
	};
}

const mockResult = {
	output: "ok",
	exitCode: 0,
	cancelled: false,
	truncated: false,
	artifactId: undefined,
	totalLines: 1,
	totalBytes: 2,
	outputLines: 1,
	outputBytes: 2,
	displayOutputs: [],
};

describe("EvalTool language dispatch", () => {
	beforeEach(() => {
		originalPiPy = Bun.env.PI_PY;
		originalPiJs = Bun.env.PI_JS;
		delete Bun.env.PI_PY;
		delete Bun.env.PI_JS;
	});

	afterEach(() => {
		vi.restoreAllMocks();
		restoreEnv("PI_PY", originalPiPy);
		restoreEnv("PI_JS", originalPiJs);
	});

	it('dispatches to the JS backend when cell.language === "js"', async () => {
		const jsExecuteSpy = vi.spyOn(evalIndex.jsBackend, "execute").mockResolvedValue(mockResult);
		const pythonExecuteSpy = vi.spyOn(evalIndex.pythonBackend, "execute");

		const tool = new EvalTool(makeSession());
		await tool.execute("call-js", {
			language: "js",
			code: "const x = 1;",
		});

		expect(jsExecuteSpy).toHaveBeenCalledTimes(1);
		expect(pythonExecuteSpy).not.toHaveBeenCalled();
	});

	it("rejects Python cells even when settings and environment request a kernel", async () => {
		Bun.env.PI_PY = "1";
		const settings = Settings.isolated({ "eval.py": true });
		const python = vi.spyOn(evalIndex.pythonBackend, "execute");
		const js = vi.spyOn(evalIndex.jsBackend, "execute");
		const probe = vi.spyOn(evalIndex.pythonBackend, "isAvailable");
		const tool = new EvalTool(makeSession(settings));
		await expect(tool.execute("retired", { language: "py", code: "never run" })).rejects.toThrow(
			"Python evaluation was retired",
		);
		expect(python).not.toHaveBeenCalled();
		expect(js).not.toHaveBeenCalled();
		expect(probe).not.toHaveBeenCalled();
		expect(resolveEvalBackends(makeSession(settings)).python).toBe(false);
	});

	it("rejects js cells when eval.js is disabled", async () => {
		const settings = Settings.isolated();
		settings.set("eval.js", false);
		const tool = new EvalTool(makeSession(settings));
		await expect(
			tool.execute("call-js-disabled", {
				language: "js",
				code: "const x = 1;",
			}),
		).rejects.toThrow(/eval\.js = false/);
	});

	it("uses settings for eval backends whose env flag is unset", () => {
		Bun.env.PI_PY = "1";
		const settings = Settings.isolated();
		settings.set("eval.py", false);
		settings.set("eval.js", false);

		expect(resolveEvalBackends(makeSession(settings))).toEqual({
			python: false,
			js: false,
		});
	});

	it("lets PI_JS disable js execution even when eval.js is enabled", async () => {
		Bun.env.PI_JS = "0";
		const settings = Settings.isolated();
		settings.set("eval.js", true);
		const tool = new EvalTool(makeSession(settings));

		await expect(
			tool.execute("call-js-env-disabled", {
				language: "js",
				code: "const x = 1;",
			}),
		).rejects.toThrow(/PI_JS=0/);
	});
});

it("direct session Python rejects before hooks, callbacks or transcript writes", async () => {
	const hook = vi.fn();
	const append = vi.fn();
	const chunk = vi.fn();
	const host = { extensionRunner: hook, appendSessionMessage: append } as unknown as EvalRunnerHost;
	const runner = new EvalRunner(host, { kernelOwnerId: "retired", parentSessionId: undefined });
	await expect(runner.executePython("opaque retired request", chunk)).rejects.toThrow("Python evaluation was retired");
	runner.beginDispose();
	await expect(runner.executePython("opaque retired request", chunk)).rejects.toThrow("Python evaluation was retired");
	expect(hook).not.toHaveBeenCalled();
	expect(append).not.toHaveBeenCalled();
	expect(chunk).not.toHaveBeenCalled();
	expect(runner.isRunning).toBe(false);
	expect(runner.hasPendingMessages).toBe(false);
});
