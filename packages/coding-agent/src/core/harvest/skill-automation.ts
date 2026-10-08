/**
 * Automatic Tool-Call Sequence Mining & Skill Automation.
 *
 * Observes normalized tool call signatures (e.g. read:*.ts, write:*.ts, bash:npm test).
 * Mines repeated n-gram sequences (length 3 to 6, occurrences >= 3).
 * Stages proposals in .harvest/skill-proposals.json under strict human approval.
 * Compiles approved skills into .harvest/skills/<name>.md checklists.
 */

import * as crypto from "node:crypto";
import * as fs from "node:fs";
import * as path from "node:path";

export interface ToolCallStep {
	readonly tool: string;
	readonly args: Record<string, unknown>;
	readonly timestamp: number;
}

export interface SkillProposal {
	readonly id: string;
	readonly name: string;
	readonly description: string;
	readonly sequence: readonly string[];
	readonly occurrences: number;
	readonly proposedAt: number;
	readonly status: "pending" | "approved" | "rejected";
}

export interface SkillProposalsData {
	readonly version: number;
	readonly proposals: Record<string, SkillProposal>;
}

export const MIN_SEQUENCE_LENGTH = 3;
export const MAX_SEQUENCE_LENGTH = 6;
export const MIN_OCCURRENCES = 3;

/**
 * Normalize tool call into a compact signature string.
 * e.g.
 * read "src/core/auth.ts" -> "read:*.ts"
 * bash "npm test -- -t foo" -> "bash:npm test"
 */
export function normalizeToolSignature(toolName: string, args: Record<string, unknown>): string {
	const name = toolName.toLowerCase();

	if (
		name === "read" ||
		name === "write" ||
		name === "edit" ||
		name === "replace_file_content" ||
		name === "write_to_file" ||
		name === "view_file" ||
		name === "ast-edit" ||
		name === "ast_edit"
	) {
		const filePath = (args.path ||
			args.filePath ||
			args.targetFile ||
			args.TargetFile ||
			args.target_file ||
			args.file ||
			args.file_path ||
			args.AbsolutePath ||
			"") as string;
		const ext = path.extname(filePath).toLowerCase();
		const canonicalTool =
			name === "read" || name === "view_file"
				? "read"
				: name === "write" || name === "write_to_file"
					? "write"
					: "edit";
		return ext ? `${canonicalTool}:*${ext}` : `${canonicalTool}:file`;
	}

	if (name === "bash" || name === "exec" || name === "command") {
		const cmd = (args.command || args.cmd || args.CommandLine || "") as string;
		const tokens = cmd.trim().split(/\s+/);
		const baseCmd = tokens.slice(0, 2).join(" ");
		return baseCmd ? `bash:${baseCmd}` : "bash:cmd";
	}

	return name;
}

export class HarvestSkillAutomation {
	readonly #workspaceRoot: string;
	readonly #harvestDir: string;
	readonly #proposalsPath: string;
	readonly #history: string[] = [];
	#proposalsData: SkillProposalsData;

	constructor(workspaceRoot: string = process.cwd()) {
		this.#workspaceRoot = path.resolve(workspaceRoot);
		this.#harvestDir = path.join(this.#workspaceRoot, ".harvest");
		this.#proposalsPath = path.join(this.#harvestDir, "skill-proposals.json");
		this.#proposalsData = this.#loadProposals();
	}

	#loadProposals(): SkillProposalsData {
		try {
			if (fs.existsSync(this.#proposalsPath)) {
				const content = fs.readFileSync(this.#proposalsPath, "utf8");
				return JSON.parse(content);
			}
		} catch {}
		return { version: 1, proposals: {} };
	}

	#saveProposals(): void {
		try {
			fs.mkdirSync(this.#harvestDir, { recursive: true });
			fs.writeFileSync(this.#proposalsPath, JSON.stringify(this.#proposalsData, null, 2), "utf8");
		} catch {}
	}

	/**
	 * Record an executed tool call and run sequence mining.
	 */
	recordToolCall(toolName: string, args: Record<string, unknown>): SkillProposal | null {
		const signature = normalizeToolSignature(toolName, args);
		this.#history.push(signature);

		return this.mineSequences();
	}

	/**
	 * Mine repeated n-gram sequences from history.
	 */
	mineSequences(): SkillProposal | null {
		const n = this.#history.length;
		if (n < MIN_SEQUENCE_LENGTH * MIN_OCCURRENCES) {
			return null;
		}

		// Check lengths 3 to 6
		for (let len = MIN_SEQUENCE_LENGTH; len <= Math.min(MAX_SEQUENCE_LENGTH, n); len++) {
			const counts = new Map<string, number>();

			for (let i = 0; i <= n - len; i++) {
				const seq = this.#history.slice(i, i + len);
				const key = seq.join(" -> ");
				counts.set(key, (counts.get(key) || 0) + 1);
			}

			for (const [key, count] of counts.entries()) {
				if (count >= MIN_OCCURRENCES) {
					const seq = key.split(" -> ");
					const id = `proposal-${crypto.createHash("sha256").update(key).digest("hex").slice(0, 8)}`;

					if (!this.#proposalsData.proposals[id]) {
						const name = `auto-${seq[0].replace(/:/g, "-")}-${seq[seq.length - 1].replace(/:/g, "-")}`;
						const proposal: SkillProposal = {
							id,
							name,
							description: `Automated repeated workflow sequence: ${key}`,
							sequence: seq,
							occurrences: count,
							proposedAt: Date.now(),
							status: "pending",
						};

						this.#proposalsData = {
							...this.#proposalsData,
							proposals: {
								...this.#proposalsData.proposals,
								[id]: proposal,
							},
						};
						this.#saveProposals();
						return proposal;
					}
				}
			}
		}

		return null;
	}

	/**
	 * Strictly human-gated approval: compiles proposal into .harvest/skills/<name>.md.
	 */
	async approveProposal(
		proposalId: string,
		customName?: string,
	): Promise<{ success: boolean; filePath?: string; error?: string }> {
		const proposal = this.#proposalsData.proposals[proposalId];
		if (!proposal) {
			return { success: false, error: `Proposal '${proposalId}' not found.` };
		}

		const rawName = customName ?? proposal.name;
		const skillName = rawName
			.replace(/[^a-zA-Z0-9_-]/g, "-")
			.replace(/-+/g, "-")
			.replace(/^-|-$/g, "");
		const skillsDir = path.join(this.#harvestDir, "skills");
		await fs.promises.mkdir(skillsDir, { recursive: true });

		const filePath = path.join(skillsDir, `${skillName}.md`);

		const checklistLines = proposal.sequence.map((step, idx) => `- [ ] Step ${idx + 1}: Execute \`${step}\``);

		const markdown = [
			`---`,
			`name: ${skillName}`,
			`description: ${proposal.description}`,
			`automated: true`,
			`mined_from_occurrences: ${proposal.occurrences}`,
			`---`,
			``,
			`# Skill: ${skillName}`,
			``,
			`## Purpose`,
			proposal.description,
			``,
			`## Required Checklist Sequence`,
			...checklistLines,
			``,
			`## Constraints & Invariants`,
			`- Strictly execute steps in sequence without skipping verification.`,
			`- Read all target files before modifying.`,
			`- Verify execution outcome before finishing.`,
		].join("\n");

		await fs.promises.writeFile(filePath, markdown, "utf8");

		this.#proposalsData = {
			...this.#proposalsData,
			proposals: {
				...this.#proposalsData.proposals,
				[proposalId]: {
					...proposal,
					status: "approved",
				},
			},
		};
		this.#saveProposals();

		return { success: true, filePath };
	}

	rejectProposal(proposalId: string): boolean {
		const proposal = this.#proposalsData.proposals[proposalId];
		if (!proposal) return false;

		this.#proposalsData = {
			...this.#proposalsData,
			proposals: {
				...this.#proposalsData.proposals,
				[proposalId]: {
					...proposal,
					status: "rejected",
				},
			},
		};
		this.#saveProposals();
		return true;
	}

	getPendingProposals(): SkillProposal[] {
		return Object.values(this.#proposalsData.proposals).filter(p => p.status === "pending");
	}

	get history(): readonly string[] {
		return this.#history;
	}
}
