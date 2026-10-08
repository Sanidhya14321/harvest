/**
 * Shared utilities for deterministic compaction and verification extraction.
 */

export interface VerificationAuditRecord {
	readonly command: string;
	readonly status: "PASS" | "FAIL" | "STALE";
	readonly timestamp: number;
	readonly exitCode: number;
	readonly stalenessReason?: string;
}

export interface ExtractedTranscriptOperations {
	readonly readFiles: readonly string[];
	readonly modifiedFiles: readonly string[];
	readonly verificationRecords: readonly VerificationAuditRecord[];
}

export interface TranscriptToolCall {
	readonly name: string;
	readonly args: Record<string, unknown>;
	readonly result?: {
		readonly stdout?: string;
		readonly stderr?: string;
		readonly exitCode?: number;
		readonly success?: boolean;
	};
	readonly timestamp?: number;
}

/**
 * Deterministically extract file operations and verification records from transcript tool calls.
 * Applies staleness tracking: marks a passed test STALE if any file was modified after the test run.
 */
export function extractTranscriptEvidence(toolCalls: readonly TranscriptToolCall[]): ExtractedTranscriptOperations {
	const read = new Set<string>();
	const modified = new Map<string, number>(); // path -> latest modification time
	const rawVerifications: Array<{
		command: string;
		passed: boolean;
		timestamp: number;
		exitCode: number;
	}> = [];

	let fallbackTimestamp = 1;

	for (const call of toolCalls) {
		const name = call.name.toLowerCase();
		const args = call.args ?? {};
		const timestamp = call.timestamp ?? fallbackTimestamp++;

		const rawPath =
			(typeof args.path === "string" && args.path) ||
			(typeof args.filePath === "string" && args.filePath) ||
			(typeof args.targetFile === "string" && args.targetFile) ||
			(typeof args.TargetFile === "string" && args.TargetFile) ||
			(typeof args.target_file === "string" && args.target_file) ||
			(typeof args.file === "string" && args.file) ||
			(typeof args.file_path === "string" && args.file_path) ||
			(typeof args.AbsolutePath === "string" && args.AbsolutePath) ||
			null;

		if ((name === "read" || name === "view_file" || name === "read_file") && rawPath) {
			read.add(rawPath);
		} else if (
			(name === "write" ||
				name === "edit" ||
				name === "replace_file_content" ||
				name === "write_to_file" ||
				name === "ast-edit" ||
				name === "ast_edit") &&
			rawPath
		) {
			modified.set(rawPath, timestamp);
		} else if (name === "bash" || name === "exec" || name === "command") {
			const cmd = (args.command || args.cmd || args.CommandLine || "") as string;
			const lower = cmd.toLowerCase();

			if (
				lower.includes("test") ||
				lower.includes("check") ||
				lower.includes("vitest") ||
				lower.includes("jest") ||
				lower.includes("pytest") ||
				lower.includes("cargo test") ||
				lower.includes("go test")
			) {
				const exitCode = call.result?.exitCode ?? (call.result?.success === false ? 1 : 0);
				const passed = exitCode === 0;
				rawVerifications.push({
					command: cmd,
					passed,
					timestamp,
					exitCode,
				});
			}
		}
	}

	// Compute staleness for verification records
	const verificationRecords: VerificationAuditRecord[] = [];

	for (const v of rawVerifications) {
		if (!v.passed) {
			verificationRecords.push({
				command: v.command,
				status: "FAIL",
				timestamp: v.timestamp,
				exitCode: v.exitCode,
			});
			continue;
		}

		// Check if any file was modified AFTER this verification passed
		const staleFiles: string[] = [];
		for (const [file, modTime] of modified.entries()) {
			if (modTime > v.timestamp) {
				staleFiles.push(file);
			}
		}

		if (staleFiles.length > 0) {
			verificationRecords.push({
				command: v.command,
				status: "STALE",
				timestamp: v.timestamp,
				exitCode: v.exitCode,
				stalenessReason: `Modified after test passed: ${staleFiles.map(f => `'${f}'`).join(", ")}`,
			});
		} else {
			verificationRecords.push({
				command: v.command,
				status: "PASS",
				timestamp: v.timestamp,
				exitCode: v.exitCode,
			});
		}
	}

	return {
		readFiles: Array.from(read),
		modifiedFiles: Array.from(modified.keys()),
		verificationRecords,
	};
}
