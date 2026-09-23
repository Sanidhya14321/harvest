import { afterEach, describe, expect, test } from "bun:test";
import * as fs from "node:fs/promises";
import * as os from "node:os";
import * as path from "node:path";
import * as logger from "../src/logger";
import { RotatingFileSink } from "../src/logger/rotating-file";

const roots: string[] = [];

afterEach(async () => {
	logger.setLogLevel(undefined);
	logger.setTransports({ console: false, file: false });
	await Promise.all(roots.splice(0).map(root => fs.rm(root, { recursive: true, force: true })));
});

async function makeTempDir(): Promise<string> {
	const dir = await fs.mkdtemp(path.join(os.tmpdir(), "omp-logger-buffer-test-"));
	roots.push(dir);
	return dir;
}

describe("logger level gate early-return contract", () => {
	test("defaults to info level when no env or override is set", () => {
		const originalHarvest = process.env.HARVEST_LOG_LEVEL;
		const originalPi = process.env.PI_LOG_LEVEL;
		const originalLog = process.env.LOG_LEVEL;
		try {
			delete process.env.HARVEST_LOG_LEVEL;
			delete process.env.PI_LOG_LEVEL;
			delete process.env.LOG_LEVEL;
			logger.setLogLevel(undefined);

			expect(logger.getLogLevel()).toBe("info");
			expect(logger.isLevelEnabled("error")).toBe(true);
			expect(logger.isLevelEnabled("warn")).toBe(true);
			expect(logger.isLevelEnabled("info")).toBe(true);
			expect(logger.isLevelEnabled("debug")).toBe(false);
		} finally {
			process.env.HARVEST_LOG_LEVEL = originalHarvest;
			process.env.PI_LOG_LEVEL = originalPi;
			process.env.LOG_LEVEL = originalLog;
		}
	});

	test("respects HARVEST_LOG_LEVEL env with precedence over PI_LOG_LEVEL and LOG_LEVEL", () => {
		const originalHarvest = process.env.HARVEST_LOG_LEVEL;
		const originalPi = process.env.PI_LOG_LEVEL;
		const originalLog = process.env.LOG_LEVEL;
		try {
			logger.setLogLevel(undefined);

			process.env.LOG_LEVEL = "error";
			expect(logger.getLogLevel()).toBe("error");

			process.env.PI_LOG_LEVEL = "warn";
			expect(logger.getLogLevel()).toBe("warn");

			process.env.HARVEST_LOG_LEVEL = "debug";
			expect(logger.getLogLevel()).toBe("debug");
		} finally {
			process.env.HARVEST_LOG_LEVEL = originalHarvest;
			process.env.PI_LOG_LEVEL = originalPi;
			process.env.LOG_LEVEL = originalLog;
			logger.setLogLevel(undefined);
		}
	});

	test("short-circuits before argument evaluation or JSON.stringify when filtered out", async () => {
		const logsDir = await makeTempDir();
		logger.setLogLevel("info");
		logger.setTransports({ console: false, file: logsDir });

		let getterCalled = false;
		let factoryCalled = false;

		// 1. Factory function should never be invoked
		logger.debug("filtered-message", () => {
			factoryCalled = true;
			return { expensive: "data" };
		});
		expect(factoryCalled).toBe(false);

		// 2. Property getter on context object should never be accessed
		const contextWithTrap = {
			get trap() {
				getterCalled = true;
				throw new Error("Getter must never be evaluated for filtered level!");
			},
		};
		expect(() => {
			logger.debug("trap-message", contextWithTrap);
		}).not.toThrow();
		expect(getterCalled).toBe(false);

		// 3. Out-of-band sinks should receive nothing
		let sinkReceived = false;
		const dispose = logger.registerLogSink(() => {
			sinkReceived = true;
		});
		logger.debug("sink-test", { a: 1 });
		expect(sinkReceived).toBe(false);
		dispose();

		// Flush and verify log file has 0 records
		logger.flushSync();
		const files = await fs.readdir(logsDir);
		const logFile = files.find(f => f.endsWith(".log"));
		if (logFile) {
			const content = await fs.readFile(path.join(logsDir, logFile), "utf8");
			expect(content).toBe("");
		}
	});

	test("evaluates context and emits when level is enabled", async () => {
		const logsDir = await makeTempDir();
		logger.setLogLevel("debug");
		logger.setTransports({ console: false, file: logsDir });

		let getterCalled = false;
		let factoryCalled = false;

		logger.debug("enabled-factory", () => {
			factoryCalled = true;
			return { evaluated: true };
		});
		expect(factoryCalled).toBe(true);

		logger.debug("enabled-getter", {
			get dynamicVal() {
				getterCalled = true;
				return 42;
			},
		});
		expect(getterCalled).toBe(true);

		logger.flushSync();
		const files = await fs.readdir(logsDir);
		const logFile = files.find(f => f.endsWith(".log"));
		expect(logFile).toBeDefined();
		const content = await fs.readFile(path.join(logsDir, logFile!), "utf8");
		expect(content).toContain("enabled-factory");
		expect(content).toContain("enabled-getter");
		expect(content).toContain('"dynamicVal":42');
	});
});

describe("RotatingFileSink buffered async sink contract", () => {
	test("flushes asynchronously on interval", async () => {
		const dir = await makeTempDir();
		const auditFile = path.join(dir, "audit.json");
		const sink = new RotatingFileSink({
			directory: dir,
			filenamePrefix: "test",
			filenameSuffix: "int",
			auditFile,
			maxBytes: 1024 * 1024,
			maxFiles: 5,
			flushIntervalMs: 60,
			bufferThresholdBytes: 1024 * 1024,
		});

		try {
			sink.write('{"msg":"line-1"}');
			sink.write('{"msg":"line-2"}');
			expect(sink.queueLength).toBe(2);

			const logFiles = (await fs.readdir(dir)).filter(f => f.endsWith(".log"));
			expect(logFiles.length).toBe(1);
			const initialContent = await fs.readFile(path.join(dir, logFiles[0]), "utf8");
			expect(initialContent).toBe("");

			// Wait for the 60ms interval to flush
			await Bun.sleep(100);

			expect(sink.queueLength).toBe(0);
			const flushedContent = await fs.readFile(path.join(dir, logFiles[0]), "utf8");
			expect(flushedContent).toContain('{"msg":"line-1"}');
			expect(flushedContent).toContain('{"msg":"line-2"}');
		} finally {
			sink.close();
		}
	});

	test("flushes when buffer exceeds byte threshold", async () => {
		const dir = await makeTempDir();
		const auditFile = path.join(dir, "audit.json");
		const sink = new RotatingFileSink({
			directory: dir,
			filenamePrefix: "test",
			filenameSuffix: "thresh",
			auditFile,
			maxBytes: 1024 * 1024,
			maxFiles: 5,
			flushIntervalMs: 10_000, // Very long interval, won't fire
			bufferThresholdBytes: 200, // Small threshold
		});

		try {
			// Write small record (< 200 bytes)
			sink.write("small-1");
			expect(sink.queueLength).toBe(1);

			// Write records that exceed 200 bytes
			sink.write("x".repeat(150));
			sink.write("y".repeat(150));

			// Allow setImmediate / async flush to complete
			await Bun.sleep(30);

			expect(sink.queueLength).toBe(0);
			const logFiles = (await fs.readdir(dir)).filter(f => f.endsWith(".log"));
			const content = await fs.readFile(path.join(dir, logFiles[0]), "utf8");
			expect(content).toContain("small-1");
			expect(content).toContain("x".repeat(150));
			expect(content).toContain("y".repeat(150));
		} finally {
			sink.close();
		}
	});

	test("preserves exact chronological ordering across rapid bursts", async () => {
		const dir = await makeTempDir();
		const auditFile = path.join(dir, "audit.json");
		const sink = new RotatingFileSink({
			directory: dir,
			filenamePrefix: "test",
			filenameSuffix: "order",
			auditFile,
			maxBytes: 10 * 1024 * 1024,
			maxFiles: 5,
			flushIntervalMs: 50,
		});

		try {
			const count = 500;
			for (let i = 0; i < count; i++) {
				sink.write(JSON.stringify({ seq: i }));
			}
			sink.close(); // Sync flush on close

			const logFiles = (await fs.readdir(dir)).filter(f => f.endsWith(".log"));
			expect(logFiles.length).toBe(1);
			const text = await fs.readFile(path.join(dir, logFiles[0]), "utf8");
			const lines = text.trim().split(os.EOL);
			expect(lines.length).toBe(count);

			for (let i = 0; i < count; i++) {
				const parsed = JSON.parse(lines[i]) as { seq: number };
				expect(parsed.seq).toBe(i);
			}
		} finally {
			sink.close();
		}
	});

	test("handles rotation boundary cleanly with buffered writes", async () => {
		const dir = await makeTempDir();
		const auditFile = path.join(dir, "audit.json");
		const sink = new RotatingFileSink({
			directory: dir,
			filenamePrefix: "test",
			filenameSuffix: "rot",
			auditFile,
			maxBytes: 500, // Small maxBytes to trigger rotation
			maxFiles: 5,
			flushIntervalMs: 10_000,
		});

		try {
			// Write 4 records of 200 bytes each (total ~800 bytes, crosses 500)
			sink.write("A".repeat(200));
			sink.write("B".repeat(200));
			sink.write("C".repeat(200));
			sink.write("D".repeat(200));

			sink.close(); // Drains synchronously

			const logFiles = (await fs.readdir(dir)).filter(f => f.includes("rot.log")).sort();
			expect(logFiles.length).toBe(2); // base and .1

			const baseContent = await fs.readFile(path.join(dir, logFiles[0]), "utf8");
			const rotContent = await fs.readFile(path.join(dir, logFiles[1]), "utf8");

			expect(baseContent).toContain("A".repeat(200));
			expect(baseContent).toContain("B".repeat(200));
			expect(rotContent).toContain("D".repeat(200));
		} finally {
			sink.close();
		}
	});
});
