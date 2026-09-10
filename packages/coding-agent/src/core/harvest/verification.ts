/**
 * Runner Detection & Diagnostic Parsing for Local Execution Grounding.
 */

import * as fs from "node:fs";
import * as path from "node:path";

export interface DetectedRunner {
	readonly runner: "npm" | "cargo" | "pytest" | "go" | "shell" | "make" | "custom";
	readonly command: string;
	readonly configFile: string;
	readonly description: string;
}

export interface VerificationDiagnostic {
	readonly hasFailures: boolean;
	readonly failedTests: readonly string[];
	readonly errorExcerpts: readonly string[];
	readonly summary: string;
}

/**
 * Automatically inspect workspace to detect verification / test runner commands.
 */
export function detectVerificationRunner(workspaceDir: string = process.cwd()): DetectedRunner | null {
	const root = path.resolve(workspaceDir);

	// 1. Shell test script
	const testSh = path.join(root, "test.sh");
	if (fs.existsSync(testSh)) {
		return {
			runner: "shell",
			command: process.platform === "win32" ? "bash test.sh" : "./test.sh",
			configFile: "test.sh",
			description: "Custom test.sh script",
		};
	}

	// 2. Node / package.json
	const pkgJsonPath = path.join(root, "package.json");
	if (fs.existsSync(pkgJsonPath)) {
		try {
			const pkg = JSON.parse(fs.readFileSync(pkgJsonPath, "utf8"));
			const scripts = pkg.scripts ?? {};
			if (scripts.test) {
				return {
					runner: "npm",
					command: "npm test",
					configFile: "package.json",
					description: `npm test (${scripts.test.slice(0, 40)})`,
				};
			}
			if (scripts.check) {
				return {
					runner: "npm",
					command: "npm run check",
					configFile: "package.json",
					description: "npm run check",
				};
			}
		} catch {}
	}

	// 3. Rust Cargo
	const cargoToml = path.join(root, "Cargo.toml");
	if (fs.existsSync(cargoToml)) {
		return {
			runner: "cargo",
			command: "cargo test",
			configFile: "Cargo.toml",
			description: "Cargo test runner",
		};
	}

	// 4. Python pytest
	if (
		fs.existsSync(path.join(root, "pytest.ini")) ||
		fs.existsSync(path.join(root, "pyproject.toml")) ||
		fs.existsSync(path.join(root, "setup.py"))
	) {
		return {
			runner: "pytest",
			command: "pytest",
			configFile: "pytest.ini / pyproject.toml",
			description: "Python pytest runner",
		};
	}

	// 5. Go test
	const goMod = path.join(root, "go.mod");
	if (fs.existsSync(goMod)) {
		return {
			runner: "go",
			command: "go test ./...",
			configFile: "go.mod",
			description: "Go test runner",
		};
	}

	// 6. Makefile
	const makefile = path.join(root, "Makefile");
	if (fs.existsSync(makefile)) {
		try {
			const content = fs.readFileSync(makefile, "utf8");
			if (/^test:/m.test(content)) {
				return {
					runner: "make",
					command: "make test",
					configFile: "Makefile",
					description: "Makefile test target",
				};
			}
		} catch {}
	}

	return null;
}

/**
 * Parse diagnostics, test failures, and assertions from test runner output.
 */
export function parseVerificationDiagnostics(stdout: string, stderr: string): VerificationDiagnostic {
	const combined = `${stdout}\n${stderr}`;
	const lines = combined.split("\n");

	const failedTests: string[] = [];
	const errorExcerpts: string[] = [];

	let hasFailures = false;

	for (let i = 0; i < lines.length; i++) {
		const line = lines[i];

		// Jest/Vitest/Bun test failure pattern
		if (/\b(?:FAIL|FAILED|AssertionError|assert\.strictEqual)\b/i.test(line)) {
			hasFailures = true;
			const trimmed = line.trim();
			if (trimmed.length > 0 && !failedTests.includes(trimmed)) {
				failedTests.push(trimmed);
			}

			// Capture a few surrounding context lines as diagnostic excerpt
			const start = Math.max(0, i - 1);
			const end = Math.min(lines.length, i + 4);
			const excerpt = lines.slice(start, end).join("\n").trim();
			if (excerpt && !errorExcerpts.includes(excerpt)) {
				errorExcerpts.push(excerpt);
			}
		}

		// Pytest failures
		if (/^_{3,}\s+(.*?)\s+_{3,}$/.test(line)) {
			hasFailures = true;
			const testName = line.replace(/_/g, "").trim();
			if (testName && !failedTests.includes(testName)) {
				failedTests.push(testName);
			}
		}

		// Rust Cargo test failures
		if (line.startsWith("failures:")) {
			hasFailures = true;
		}
	}

	const summary = hasFailures
		? `Verification failed with ${failedTests.length || 1} detected error diagnostic(s).`
		: "Verification completed with no obvious assertion failure lines detected.";

	return {
		hasFailures,
		failedTests,
		errorExcerpts: errorExcerpts.slice(0, 5),
		summary,
	};
}

/**
 * Format structured repair prompt to feed back into model context.
 */
export function formatRepairPrompt(options: {
	readonly command: string;
	readonly exitCode: number;
	readonly stdout: string;
	readonly stderr: string;
}): string {
	const diag = parseVerificationDiagnostics(options.stdout, options.stderr);
	const excerptBlock =
		diag.errorExcerpts.length > 0
			? `Key Diagnostics:\n${diag.errorExcerpts.join("\n---\n")}`
			: `Captured Output:\n${(options.stderr || options.stdout).slice(-1500)}`;

	return `[EXECUTION GROUNDING FAILURE]
Command: \`${options.command}\` (Exit code: ${options.exitCode})
${diag.summary}

${excerptBlock}

Please inspect the failure diagnostics, fix the cause in the relevant files, and re-run verification.`;
}
