/**
 * Execution Grounding Engine & Verify-Before-Done Nudge Loop.
 *
 * Prevents agents from prematurely claiming task victory without running
 * tests or builds against mutated files.
 */

import * as path from "node:path";
import { detectVerificationRunner, formatRepairPrompt, parseVerificationDiagnostics } from "./verification";

export interface ExecutionRecord {
	readonly command: string;
	readonly exitCode: number;
	readonly stdout: string;
	readonly stderr: string;
	readonly timestamp: number;
}

export interface GroundingCheckOptions {
	readonly stopReason?: string;
	readonly role?: string;
	readonly isStandaloneDeliverable?: boolean;
}

export interface NudgeDecision {
	readonly shouldNudge: boolean;
	readonly steerPrompt?: string;
	readonly unverifiedFiles: readonly string[];
	readonly runnerCommand?: string;
}

export const MAX_GROUNDING_NUDGES = 2;

export class ExecutionGroundingEngine {
	readonly #mutatedFiles: Map<string, number> = new Map<string, number>(); // path -> lastMutationTime
	readonly #executions: ExecutionRecord[] = [];
	#nudgesUsed: number = 0;
	readonly #workspaceRoot: string;

	constructor(workspaceRoot: string = process.cwd()) {
		this.#workspaceRoot = path.resolve(workspaceRoot);
	}

	#normalizePath(filePath: string): string {
		const abs = path.isAbsolute(filePath) ? filePath : path.resolve(this.#workspaceRoot, filePath);
		const rel = path.relative(this.#workspaceRoot, abs).replace(/\\/g, "/");
		return rel || abs.replace(/\\/g, "/");
	}

	/** Record file mutation */
	recordFileMutation(filePath: string): void {
		const norm = this.#normalizePath(filePath);
		this.#mutatedFiles.set(norm, Date.now());
	}

	/** Record command execution (e.g. bash tool run) */
	recordCommandExecution(command: string, exitCode: number, stdout: string = "", stderr: string = ""): void {
		this.#executions.push({
			command,
			exitCode,
			stdout,
			stderr,
			timestamp: Date.now(),
		});
	}

	/**
	 * Check if any mutations exist that have not had a successful verification
	 * command run AFTER the mutation timestamp.
	 */
	getUnverifiedMutations(): string[] {
		if (this.#mutatedFiles.size === 0) return [];

		// Find latest successful verification command
		let latestSuccessTime = 0;
		for (const exec of this.#executions) {
			if (exec.exitCode === 0 && this.#isVerificationCommand(exec.command)) {
				const diag = parseVerificationDiagnostics(exec.stdout, exec.stderr);
				if (!diag.hasFailures && exec.timestamp > latestSuccessTime) {
					latestSuccessTime = exec.timestamp;
				}
			}
		}

		const unverified: string[] = [];
		for (const [file, mutationTime] of this.#mutatedFiles.entries()) {
			if (mutationTime > latestSuccessTime) {
				unverified.push(file);
			}
		}

		return unverified;
	}

	#isVerificationCommand(cmd: string): boolean {
		const lower = cmd.toLowerCase().trim();
		return (
			lower.includes("test") ||
			lower.includes("pytest") ||
			lower.includes("vitest") ||
			lower.includes("jest") ||
			lower.includes("cargo test") ||
			lower.includes("go test") ||
			lower.includes("check") ||
			lower.includes("build")
		);
	}

	/**
	 * Whether grounding has been validated for all mutations in the session.
	 */
	isGroundingValidated(): boolean {
		return this.#mutatedFiles.size > 0 && this.getUnverifiedMutations().length === 0;
	}

	/**
	 * Whether the agent had mutations, experienced a failure, but eventually passed.
	 */
	hasRecoveredFromFailure(): boolean {
		if (!this.isGroundingValidated()) return false;
		// Check if any prior verification run failed
		return this.#executions.some(e => e.exitCode !== 0 && this.#isVerificationCommand(e.command));
	}

	/**
	 * Verify-Before-Done Nudge interceptor.
	 * Returns steerPrompt if agent is stopping with unverified mutations.
	 */
	checkVerifyBeforeDoneNudge(options: GroundingCheckOptions = {}): NudgeDecision {
		// Only nudge when model wants to stop
		if (options.stopReason && options.stopReason !== "stop" && options.stopReason !== "end_turn") {
			return { shouldNudge: false, unverifiedFiles: [] };
		}

		// Frontend role and standalone deliverables are exempt
		if (options.role === "frontend" || options.isStandaloneDeliverable) {
			return { shouldNudge: false, unverifiedFiles: [] };
		}

		const unverified = this.getUnverifiedMutations();
		if (unverified.length === 0) {
			return { shouldNudge: false, unverifiedFiles: [] };
		}

		// Cap nudges at MAX_GROUNDING_NUDGES (2)
		if (this.#nudgesUsed >= MAX_GROUNDING_NUDGES) {
			return { shouldNudge: false, unverifiedFiles: unverified };
		}

		const runner = detectVerificationRunner(this.#workspaceRoot);
		const cmd = runner ? runner.command : "npm test";

		this.#nudgesUsed++;

		const fileList = unverified.map(f => `'${f}'`).join(", ");
		const steerPrompt = `You modified [${fileList}] but have not verified the changes successfully. Before finishing, run the project's verification command: \`${cmd}\`. Fix any failures it reports.`;

		return {
			shouldNudge: true,
			steerPrompt,
			unverifiedFiles: unverified,
			runnerCommand: cmd,
		};
	}

	/** Reset counters for a fresh user prompt */
	resetForNewPrompt(): void {
		this.#nudgesUsed = 0;
	}

	get nudgesUsed(): number {
		return this.#nudgesUsed;
	}

	get executions(): readonly ExecutionRecord[] {
		return this.#executions;
	}

	get mutatedFiles(): readonly string[] {
		return Array.from(this.#mutatedFiles.keys());
	}

	clear(): void {
		this.#mutatedFiles.clear();
		this.#executions.length = 0;
		this.#nudgesUsed = 0;
	}
}
export { formatRepairPrompt };
