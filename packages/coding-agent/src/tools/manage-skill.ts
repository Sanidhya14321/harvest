import * as path from "node:path";
import { type } from "@harvest/omptype";
import type { AgentTool, AgentToolResult } from "@harvest/pi-agent-core";
import {
	createSkillDraft,
	deleteManagedSkill,
	evaluateSkillRevision,
	getManagedSkillsDir,
	getSkillPins,
	listSkillRevisions,
	promoteSkillRevision,
	readActiveSkillRevision,
	requireAutoImprovementBudget,
	rollbackSkillRevision,
	sanitizeSkillName,
	validateSkillRevisionId,
	writeManagedSkill,
} from "../autolearn/managed-skills";
import { isNameClaimedByAuthoredSkill } from "../extensibility/skills";
import manageSkillDescription from "../prompts/tools/manage-skill.md" with { type: "text" };
import type { ToolSession } from ".";

const manageSkillSchema = type({
	action: "'create' | 'update' | 'delete' | 'draft' | 'evaluate' | 'promote' | 'rollback' | 'list'",
	name: type("string").describe("kebab-case skill name"),
	"description?": type("string").describe(
		"one-line description of when to use the skill (required for create; update/draft merge omitted fields from current)",
	),
	"body?": type("string").describe(
		"the SKILL.md body in markdown, no frontmatter (required for create; update/draft merge omitted fields from current)",
	),
	"revisionId?": type("string").describe(
		"target revision for evaluate/promote/rollback (defaults to the latest draft)",
	),
	"expectedActive?": type("string").describe(
		"expected current active revision for create/update/draft; rejects on conflict",
	),
	"task?": type("string").describe("explicit evaluation task (required for evaluate)"),
	"expectedOutcome?": type("string").describe("explicit expected observable outcome (required for evaluate)"),
	"discloseUnevaluated?": type("boolean").describe(
		"explicitly activate unevaluated content for create/update/promote",
	),
}).narrow(
	(p, ctx) =>
		p.action === "delete" ||
		p.action === "list" ||
		(p.action === "create" && p.description !== undefined && p.body !== undefined) ||
		((p.action === "update" || p.action === "draft") && (p.description !== undefined || p.body !== undefined)) ||
		(p.action === "evaluate" && p.task !== undefined && p.expectedOutcome !== undefined) ||
		p.action === "promote" ||
		p.action === "rollback" ||
		// Enforce the action/field contract at validation time rather than only in
		// execute. Kept as a cross-field narrow (not a discriminated union) so the
		// wire schema stays a single root object — strict structured-output mode and
		// the Anthropic tool-schema builder both require that. Create needs both
		// fields; update/draft merge omitted fields from the current revision.
		ctx.mustBe(
			'used with description+body for "create", description and/or body for "update"/"draft", task+expectedOutcome for "evaluate"',
		),
);

export type ManageSkillParams = typeof manageSkillSchema.infer;

/**
 * Direct create/update/delete of isolated managed skills, plus explicit
 * revision verbs (draft/evaluate/promote/rollback/list). Gated behind
 * `autolearn.enabled`; backend-independent (the skill side is standalone).
 *
 * Every mutation routes through the shared revision store: the first managed
 * mutation of an existing artifact seeds its current content into history,
 * and updates mint new draft revisions (active revisions are immutable).
 * `create`/`update` preserve their immediate file-write semantics and record
 * + activate the new revision at once, surfacing `evaluated:false,
 * status:"unevaluated-active"` in details. `draft` stages a revision without
 * touching the live file; `evaluate` records a task/expected-outcome run
 * without promoting; `promote`/`rollback` move the active pointer and
 * materialize the file. Failed evaluations block promotion; unevaluated
 * promotion requires explicit `discloseUnevaluated`; expected-revision
 * conflicts reject.
 */
export class ManageSkillTool implements AgentTool<typeof manageSkillSchema> {
	readonly name = "manage_skill";
	readonly approval = "write" as const;
	readonly label = "Manage Skill";
	readonly description = manageSkillDescription;
	readonly parameters = manageSkillSchema;
	readonly strict = true;
	readonly loadMode = "essential" as const;
	readonly summary = "Create, update, or delete an isolated managed skill";

	constructor(private readonly refreshSkillsOrSession?: (() => Promise<void>) | ToolSession) {}

	static createIf(session: ToolSession): ManageSkillTool | null {
		if (!session.settings.get("autolearn.enabled")) return null;
		return new ManageSkillTool(session);
	}

	private refreshSkills(): Promise<void> {
		if (typeof this.refreshSkillsOrSession === "function") return this.refreshSkillsOrSession();
		return this.refreshSkillsOrSession?.refreshSkills?.() ?? Promise.resolve();
	}

	private evalParent(): ToolSession | undefined {
		return typeof this.refreshSkillsOrSession === "function" ? undefined : this.refreshSkillsOrSession;
	}

	async execute(_id: string, params: ManageSkillParams): Promise<AgentToolResult> {
		switch (params.action) {
			case "delete":
				return this.deleteSkill(params);
			case "list":
				return this.listRevisions(params);
			case "draft":
				return this.draftRevision(params);
			case "evaluate":
				return this.evaluateRevision(params);
			case "promote":
				return this.promoteRevision(params);
			case "rollback":
				return this.rollbackRevision(params);
			case "create":
			case "update":
				return this.writeSkill(params);
			default:
				throw new Error(`Unknown manage_skill action "${(params as { action: string }).action}".`);
		}
	}

	private async deleteSkill(params: ManageSkillParams): Promise<AgentToolResult> {
		await deleteManagedSkill(params.name);
		await this.refreshSkills();
		return {
			content: [{ type: "text", text: `Deleted managed skill "${params.name}".` }],
			details: { action: "delete", name: params.name },
		};
	}

	private async listRevisions(params: ManageSkillParams): Promise<AgentToolResult> {
		const { active, revisions } = await listSkillRevisions(params.name);
		const pins = getSkillPins(params.name);
		const lines = revisions.map(rev => {
			const passing =
				rev.evaluations.length === 0 ? "unevaluated" : rev.evaluations.every(e => e.passed) ? "passing" : "failing";
			const markers = [
				rev.state,
				rev.id === active ? "active" : null,
				pins.has(rev.id) ? "pinned" : null,
				`eval:${passing}`,
			]
				.filter(Boolean)
				.join(", ");
			return `- ${rev.id} (${markers}): ${rev.evaluations.length} evaluation(s), parent ${rev.parent ?? "none"}`;
		});
		return {
			content: [
				{
					type: "text",
					text:
						`Managed skill "${params.name}": active ${active ?? "none"}, ` +
						`${revisions.length} revision(s).\n${lines.join("\n")}\n` +
						`Draft stages without touching the live file; evaluate records pass/fail (never promotes); ` +
						`promote activates (failed blocks, unevaluated needs discloseUnevaluated); rollback reactivates a prior active revision.`,
				},
			],
			details: {
				action: "list",
				name: params.name,
				active,
				revisions: revisions.map(rev => ({
					id: rev.id,
					state: rev.state,
					active: rev.id === active,
					evaluations: rev.evaluations.length,
					evalStatus:
						rev.evaluations.length === 0
							? "unevaluated"
							: rev.evaluations.every(e => e.passed)
								? "passing"
								: "failing",
					pinned: pins.has(rev.id),
					parent: rev.parent,
				})),
			},
		};
	}

	private async draftRevision(params: ManageSkillParams): Promise<AgentToolResult> {
		if (!params.description && !params.body) {
			throw new Error(`"draft" requires "description" and/or "body" (omitted fields merge from current).`);
		}
		const parent = this.evalParent();
		if (parent) requireAutoImprovementBudget(parent, "candidate");
		if (params.action === "draft" && isNameClaimedByAuthoredSkill(sanitizeSkillName(params.name))) {
			const existing = await readActiveSkillRevision(params.name).catch(() => undefined);
			if (!existing) {
				return {
					content: [
						{
							type: "text",
							text: `Cannot draft managed skill "${params.name}": an authored skill of that name already exists, and managed skills cannot override authored ones. Choose a different name.`,
						},
					],
					isError: true,
					details: { action: "draft", name: params.name, shadowed: true },
				};
			}
		}
		const revision = await createSkillDraft({
			name: params.name,
			description: params.description,
			body: params.body,
			expectedActive: params.expectedActive,
		});
		return {
			content: [
				{
					type: "text",
					text:
						`Drafted managed skill "${params.name}" as revision ${revision.id} ` +
						`(parent ${revision.parent ?? "none"}). The live file is unchanged; ` +
						`evaluate it, then promote to activate.`,
				},
			],
			details: { action: "draft", name: params.name, revisionId: revision.id, parent: revision.parent },
		};
	}

	private async evaluateRevision(params: ManageSkillParams): Promise<AgentToolResult> {
		if (!params.task || !params.expectedOutcome) {
			throw new Error(`"evaluate" requires both "task" and "expectedOutcome".`);
		}
		const parent = this.evalParent();
		if (parent) requireAutoImprovementBudget(parent, "evaluation");
		const revId = await this.resolveTargetRevision(params);
		const { passed, summary } = await evaluateSkillRevision(params.name, revId, {
			task: params.task,
			expectedOutcome: params.expectedOutcome,
			parent: this.evalParent(),
		});
		const verdict = passed ? "passed" : "FAILED";
		return {
			content: [
				{
					type: "text",
					text:
						`Evaluation ${verdict} for managed skill "${params.name}" revision ${revId}: ${summary}` +
						(passed
							? ` Promote to activate.`
							: ` Failed evaluations block promotion; fix the content and draft a new revision.`),
				},
			],
			details: { action: "evaluate", name: params.name, revisionId: revId, passed },
		};
	}

	private async promoteRevision(params: ManageSkillParams): Promise<AgentToolResult> {
		const revId = await this.resolveTargetRevision(params);
		const { path: skillPath, disclosedUnevaluated } = await promoteSkillRevision(params.name, revId, {
			discloseUnevaluated: params.discloseUnevaluated,
		});
		await this.refreshSkills();
		const relativePath = path.relative(getManagedSkillsDir(), skillPath);
		const suffix = disclosedUnevaluated
			? ` Activated unevaluated content explicitly (no evaluations recorded yet).`
			: ``;
		return {
			content: [
				{
					type: "text",
					text: `Promoted managed skill "${params.name}" to revision ${revId} (managed-skills/${relativePath}).${suffix}`,
				},
			],
			details: {
				action: "promote",
				name: params.name,
				revisionId: revId,
				disclosedUnevaluated,
			},
		};
	}

	private async rollbackRevision(params: ManageSkillParams): Promise<AgentToolResult> {
		if (!params.revisionId) {
			throw new Error(`"rollback" requires "revisionId". Use action "list" to see candidates.`);
		}
		const cleanRev = validateSkillRevisionId(params.revisionId);
		const { path: skillPath } = await rollbackSkillRevision(params.name, cleanRev);
		await this.refreshSkills();
		const relativePath = path.relative(getManagedSkillsDir(), skillPath);
		return {
			content: [
				{
					type: "text",
					text: `Rolled back managed skill "${params.name}" to revision ${cleanRev} (managed-skills/${relativePath}).`,
				},
			],
			details: { action: "rollback", name: params.name, revisionId: cleanRev },
		};
	}

	private async resolveTargetRevision(params: ManageSkillParams): Promise<string> {
		if (params.revisionId) return validateSkillRevisionId(params.revisionId);
		const { active, revisions } = await listSkillRevisions(params.name);
		const drafts = revisions.filter(rev => rev.state === "draft" && rev.id !== active);
		const latest = drafts.length > 0 ? drafts[drafts.length - 1] : revisions[revisions.length - 1];
		if (!latest) throw new Error(`Managed skill "${params.name}" has no revisions. Draft one first.`);
		return latest.id;
	}

	private async writeSkill(params: ManageSkillParams): Promise<AgentToolResult> {
		// Create needs both fields; update merges omitted fields from current.
		if (params.action === "create" && (!params.description || !params.body)) {
			throw new Error(`"create" requires both "description" and "body".`);
		}
		if (params.action === "update" && !params.description && !params.body) {
			throw new Error(`"update" requires "description" and/or "body" (omitted fields merge from current).`);
		}
		const parent = this.evalParent();
		if (parent) requireAutoImprovementBudget(parent, "candidate");
		// A managed skill resolves below any authored skill of the same name
		// (authored always wins in discovery), so creating one under a name an
		// authored skill already claims writes a file that never surfaces. Refuse
		// up front rather than report a false "Created". `sanitizeSkillName`
		// normalizes to the on-disk name the discovery scan compares against.
		if (params.action === "create" && isNameClaimedByAuthoredSkill(sanitizeSkillName(params.name))) {
			return {
				content: [
					{
						type: "text",
						text: `Cannot create managed skill "${params.name}": an authored skill of that name already exists, and managed skills cannot override authored ones. Choose a different name.`,
					},
				],
				isError: true,
				details: { action: "create", name: params.name, shadowed: true },
			};
		}
		const { path: skillPath } = await writeManagedSkill({
			action: params.action === "create" ? "create" : "update",
			name: params.name,
			description: params.description,
			body: params.body,
			expectedActive: params.expectedActive,
		});
		await this.refreshSkills();
		const relativePath = path.relative(getManagedSkillsDir(), skillPath);
		const verb = params.action === "create" ? "Created" : "Updated";
		return {
			content: [{ type: "text", text: `${verb} managed skill "${params.name}" (managed-skills/${relativePath}).` }],
			details: {
				action: params.action,
				name: params.name,
				// Immediate create/update activates without evaluations; the
				// gated draft/evaluate/promote path (or an explicit
				// discloseUnevaluated promote) is the evaluation-first flow.
				evaluated: false,
				status: "unevaluated-active",
			},
		};
	}
}
