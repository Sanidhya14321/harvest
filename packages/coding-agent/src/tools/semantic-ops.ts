/**
 * Semantic TUI renderers for tool operations that otherwise fall through to
 * the generic JSON fallback: `checkpoint`, `rewind`, `security_scan`,
 * `memory_edit`, `learn`, `manage_skill`, `search_code`, `sessions`, and
 * `presets`.
 *
 * Each renderer keeps the collapsed transcript to a compact status row and
 * reserves the framed panel for expanded detail, following the same
 * pending/success/warning/error conventions as the other built-in renderers.
 * Failed, denied, or unknown effects never render with success styling.
 */
import type { Component } from "@harvest/pi-tui";
import { Text } from "@harvest/pi-tui";
import type { RenderResultOptions } from "../extensibility/custom-tools/types";
import type { Theme } from "../modes/theme/theme";
import { fileHyperlink, framedBlock, renderStatusLine } from "../tui";
import {
	createCachedComponent,
	Ellipsis,
	formatErrorDetail,
	formatErrorMessage,
	formatExpandHint,
	formatMoreItems,
	PREVIEW_LIMITS,
	previewLine,
	replaceTabs,
	shortenPath,
	TRUNCATE_LENGTHS,
	truncateToWidth,
} from "./render-utils";
import type { ToolActivityContext, ToolActivitySummary } from "./renderers";

type ToolResult = {
	content: Array<{ type: string; text?: string }>;
	details?: unknown;
	isError?: boolean;
};

function resultText(result: ToolResult): string {
	return (result.content?.find(part => part.type === "text")?.text ?? "").trim();
}

function asRecord(value: unknown): Record<string, unknown> | undefined {
	return typeof value === "object" && value !== null && !Array.isArray(value)
		? (value as Record<string, unknown>)
		: undefined;
}

function asString(value: unknown): string | undefined {
	return typeof value === "string" ? value : undefined;
}

function asNumber(value: unknown): number | undefined {
	return typeof value === "number" && Number.isFinite(value) ? value : undefined;
}

function headerDetail(value: string | undefined, maxWidth: number = TRUNCATE_LENGTHS.CONTENT): string | undefined {
	const clean = replaceTabs((value ?? "").trim());
	if (!clean) return undefined;
	return truncateToWidth(clean, maxWidth, Ellipsis.Unicode);
}

function bodyLines(text: string, limit: number): string[] {
	return replaceTabs(text)
		.split("\n")
		.map(line => line.trimEnd())
		.filter(line => line.trim().length > 0)
		.slice(0, limit);
}

function errorComponent(message: string, theme: Theme): Component {
	return new Text(formatErrorMessage(message || "Tool failed", theme), 0, 0);
}

// =============================================================================
// checkpoint
// =============================================================================

interface CheckpointRenderArgs {
	goal?: unknown;
}

interface CheckpointRenderDetails {
	goal?: unknown;
	startedAt?: unknown;
}

function checkpointGoal(args: unknown, details: unknown): string | undefined {
	const fromArgs = asString(asRecord(args)?.goal)?.trim();
	if (fromArgs) return fromArgs;
	return asString(asRecord(details)?.goal)?.trim() || undefined;
}

export const checkpointToolRenderer = {
	inline: true,
	mergeCallAndResult: true,
	activitySummary(args: unknown): ToolActivitySummary {
		const detail = headerDetail(asString(asRecord(args)?.goal));
		return detail ? { label: "Checkpoint", detail } : { label: "Checkpoint" };
	},
	renderCall(args: CheckpointRenderArgs, options: RenderResultOptions, theme: Theme): Component {
		const header = renderStatusLine(
			{
				icon: options.spinnerFrame !== undefined ? "running" : "pending",
				spinnerFrame: options.spinnerFrame,
				title: "Checkpoint",
				description: headerDetail(asString(args?.goal)) ?? "saving context",
			},
			theme,
		);
		return new Text(header, 0, 0);
	},
	renderResult(result: ToolResult, options: RenderResultOptions, theme: Theme, args?: unknown): Component {
		if (result.isError) {
			return errorComponent(resultText(result) || "Checkpoint failed", theme);
		}
		const details = asRecord(result.details) as CheckpointRenderDetails | undefined;
		const goal = checkpointGoal(args, result.details);
		const header = renderStatusLine(
			{
				icon: "success",
				title: "Checkpoint",
				description: headerDetail(goal),
				meta: ["created"],
			},
			theme,
		);
		if (!options.expanded) {
			return new Text(header, 0, 0);
		}
		return framedBlock(theme, width => {
			const lines: string[] = [];
			if (goal) lines.push(theme.fg("toolOutput", truncateToWidth(replaceTabs(goal), width)));
			const startedAt = asString(details?.startedAt)?.trim();
			if (startedAt) lines.push(theme.fg("dim", `started ${truncateToWidth(startedAt, TRUNCATE_LENGTHS.LONG)}`));
			for (const line of bodyLines(resultText(result), PREVIEW_LIMITS.OUTPUT_EXPANDED)) {
				const rendered = theme.fg("dim", truncateToWidth(line, width));
				if (!lines.includes(rendered)) lines.push(rendered);
			}
			return { header, sections: lines.length > 0 ? [{ lines }] : [], state: "success", width };
		});
	},
};

// =============================================================================
// rewind
// =============================================================================

interface RewindRenderArgs {
	report?: unknown;
}

function rewindReport(args: unknown, details: unknown): string | undefined {
	const fromArgs = asString(asRecord(args)?.report)?.trim();
	if (fromArgs) return fromArgs;
	return asString(asRecord(details)?.report)?.trim() || undefined;
}

export const rewindToolRenderer = {
	inline: true,
	mergeCallAndResult: true,
	activitySummary(args: unknown): ToolActivitySummary {
		const report = asString(asRecord(args)?.report)?.trim();
		const preview = report ? previewLine(report, TRUNCATE_LENGTHS.CONTENT) : undefined;
		return preview ? { label: "Rewind", detail: `restored · ${preview}` } : { label: "Rewind", detail: "restored" };
	},
	renderCall(_args: RewindRenderArgs, options: RenderResultOptions, theme: Theme): Component {
		const header = renderStatusLine(
			{
				icon: options.spinnerFrame !== undefined ? "running" : "pending",
				spinnerFrame: options.spinnerFrame,
				title: "Rewind",
				description: "restoring checkpoint",
			},
			theme,
		);
		return new Text(header, 0, 0);
	},
	renderResult(result: ToolResult, options: RenderResultOptions, theme: Theme, args?: unknown): Component {
		if (result.isError) {
			return errorComponent(resultText(result) || "Rewind failed", theme);
		}
		const report = rewindReport(args, result.details);
		const header = renderStatusLine(
			{
				icon: "success",
				title: "Rewind",
				description: report ? previewLine(report, TRUNCATE_LENGTHS.CONTENT) : undefined,
				meta: ["restored"],
			},
			theme,
		);
		if (!options.expanded) {
			return new Text(header, 0, 0);
		}
		return framedBlock(theme, width => {
			const lines: string[] = [];
			for (const line of bodyLines(report ?? resultText(result), PREVIEW_LIMITS.OUTPUT_EXPANDED)) {
				lines.push(theme.fg("toolOutput", truncateToWidth(line, width)));
			}
			return { header, sections: lines.length > 0 ? [{ lines }] : [], state: "success", width };
		});
	},
};

// =============================================================================
// security_scan
// =============================================================================

interface SecurityScanRenderArgs {
	action?: unknown;
	target_kind?: unknown;
	plan_id?: unknown;
	operation_id?: unknown;
	scan_id?: unknown;
	finding_id?: unknown;
	repository_url?: unknown;
}

function securityAction(args: unknown, details: unknown): string {
	const fromArgs = asString(asRecord(args)?.action)?.trim();
	if (fromArgs) return fromArgs;
	const fromDetails = asString(asRecord(details)?.action)?.trim();
	return fromDetails || "scan";
}

function securityDisposition(
	action: string,
	args: Record<string, unknown> | undefined,
	details: Record<string, unknown> | undefined,
	text: string,
): { meta: string[]; state: "success" | "warning" | "error" | "info" } {
	const operation = asRecord(details?.operation);
	const phase = asString(operation?.phase);
	const findingCount = asNumber(operation?.findingCount);
	const operationError = asString(operation?.error)?.trim();
	if (operationError || phase === "failed") {
		return { meta: [action, phase ?? "failed"], state: "error" };
	}
	if (phase === "cancelled" || details?.cancelled === true) {
		return { meta: [action, "cancelled"], state: "warning" };
	}
	const plan = asRecord(details?.plan);
	if (action === "preflight" || plan) {
		const id = asString(plan?.id)?.trim();
		return { meta: id ? ["plan ready", id] : ["plan ready"], state: "success" };
	}
	if (action === "validate") {
		const finding = asRecord(details?.finding);
		const status = asString(finding?.validationStatus)?.trim();
		return { meta: status ? ["validated", status] : ["validated"], state: "success" };
	}
	const imported = asRecord(details?.importedScan);
	const importedCount = asNumber(imported?.findingCount);
	if (action === "cloud_pull" || imported) {
		if (importedCount !== undefined && importedCount > 0) {
			return { meta: [`${importedCount} findings`, "imported"], state: "warning" };
		}
		return { meta: ["imported"], state: "success" };
	}
	const cloudStats = asRecord(details?.cloudStats);
	if (action === "cloud_status" || cloudStats) {
		const finished = asNumber(cloudStats?.finishedCommits);
		const pending = asNumber(cloudStats?.pendingCommits);
		if (finished !== undefined || pending !== undefined) {
			return { meta: [`${finished ?? 0} finished`, `${pending ?? 0} pending`], state: "info" };
		}
		return { meta: ["cloud status"], state: "info" };
	}
	const cloudScan = asRecord(details?.cloudScan);
	if (action === "cloud_start" || cloudScan) {
		const id = asString(cloudScan?.id)?.trim();
		return { meta: id ? ["cloud scan started", id] : ["cloud scan started"], state: "success" };
	}
	if (action === "cloud_scans") {
		const configs = details?.cloudConfigurations;
		const count = Array.isArray(configs) ? configs.length : undefined;
		return { meta: [count !== undefined ? `${count} configurations` : "configurations"], state: "info" };
	}
	if (operation || action === "start" || action === "status") {
		if (findingCount !== undefined && findingCount > 0) {
			return { meta: [phase ?? action, `${findingCount} findings`], state: "warning" };
		}
		if (phase === "completed" || phase === "partial") {
			return {
				meta: [phase, findingCount === 0 ? "no findings" : (phase ?? "")].filter(Boolean),
				state: phase === "partial" ? "warning" : "success",
			};
		}
		if (phase) return { meta: [phase], state: "info" };
		const scanId = asString(operation?.scanId)?.trim();
		return { meta: scanId ? [action, scanId] : [action], state: "info" };
	}
	const target = asString(args?.target_kind)?.trim() ?? asString(args?.repository_url)?.trim();
	void text;
	return { meta: target ? [action, target] : [action], state: "info" };
}

function securitySeverityLine(details: Record<string, unknown> | undefined, theme: Theme): string | undefined {
	const cloudStats = asRecord(details?.cloudStats);
	const counts = asRecord(cloudStats?.findingCounts);
	if (!counts) return undefined;
	const parts: string[] = [];
	for (const level of ["critical", "high", "medium", "low", "informational"] as const) {
		const count = asNumber(counts[level]);
		if (count !== undefined && count > 0) parts.push(`${level}: ${count}`);
	}
	if (parts.length === 0) return undefined;
	return theme.fg("warning", `severity ${parts.join(" · ")}`);
}

export const securityScanToolRenderer = {
	inline: true,
	mergeCallAndResult: true,
	activitySummary(args: unknown, context: ToolActivityContext): ToolActivitySummary {
		const record = asRecord(args);
		const action = asString(record?.action)?.trim() || "scan";
		const target =
			asString(record?.operation_id)?.trim() ||
			asString(record?.plan_id)?.trim() ||
			asString(record?.scan_id)?.trim() ||
			asString(record?.target_kind)?.trim();
		void context;
		return target
			? { label: "Security Scan", detail: `${action} ${target}` }
			: { label: "Security Scan", detail: action };
	},
	renderCall(args: SecurityScanRenderArgs, options: RenderResultOptions, theme: Theme): Component {
		const action = asString(args?.action)?.trim() || "scan";
		const header = renderStatusLine(
			{
				icon: options.spinnerFrame !== undefined ? "running" : "pending",
				spinnerFrame: options.spinnerFrame,
				title: "Security Scan",
				description: headerDetail(action),
			},
			theme,
		);
		return new Text(header, 0, 0);
	},
	renderResult(result: ToolResult, options: RenderResultOptions, theme: Theme, args?: unknown): Component {
		const text = resultText(result);
		if (result.isError) {
			return errorComponent(text || "Security scan failed", theme);
		}
		const details = asRecord(result.details);
		const action = securityAction(args, result.details);
		const disposition = securityDisposition(action, asRecord(args), details, text);
		const icon = disposition.state === "success" ? ("success" as const) : disposition.state;
		const header = renderStatusLine(
			{ icon, title: "Security Scan", description: action, meta: disposition.meta },
			theme,
		);
		if (!options.expanded) {
			return new Text(header, 0, 0);
		}
		return framedBlock(theme, width => {
			const lines: string[] = [];
			const severity = securitySeverityLine(details, theme);
			if (severity) lines.push(truncateToWidth(severity, width, Ellipsis.Omit));
			const operation = asRecord(details?.operation);
			const snapshotParts: string[] = [];
			const scanId = asString(operation?.scanId)?.trim();
			const phase = asString(operation?.phase)?.trim();
			if (scanId) snapshotParts.push(`scan ${scanId}`);
			if (phase) snapshotParts.push(phase);
			const count = asNumber(operation?.findingCount);
			if (count !== undefined) snapshotParts.push(`${count} findings`);
			if (snapshotParts.length > 0) {
				lines.push(theme.fg("dim", truncateToWidth(snapshotParts.join(" · "), width, Ellipsis.Omit)));
			}
			for (const line of bodyLines(text, PREVIEW_LIMITS.OUTPUT_EXPANDED)) {
				lines.push(theme.fg("toolOutput", truncateToWidth(line, width, Ellipsis.Omit)));
			}
			return {
				header,
				sections: lines.length > 0 ? [{ lines }] : [],
				state: disposition.state === "info" ? "success" : disposition.state,
				borderColor: disposition.state === "error" ? "error" : "borderMuted",
				width,
			};
		});
	},
};

// =============================================================================
// memory_edit
// =============================================================================

interface MemoryEditRenderArgs {
	op?: unknown;
	id?: unknown;
}

const MEMORY_EDIT_DISPOSITION: Record<string, { label: string; settled: "success" | "warning" }> = {
	updated: { label: "updated", settled: "success" },
	deleted: { label: "forgotten", settled: "success" },
	invalidated: { label: "invalidated", settled: "success" },
	not_found: { label: "not found", settled: "warning" },
	not_editable: { label: "read-only", settled: "warning" },
};

export const memoryEditToolRenderer = {
	inline: true,
	mergeCallAndResult: true,
	activitySummary(args: unknown): ToolActivitySummary {
		const record = asRecord(args);
		const op = asString(record?.op)?.trim();
		const id = asString(record?.id)?.trim();
		const detail = [op, id ? shortenPath(id) : undefined].filter(Boolean).join(" ");
		return detail ? { label: "Memory Edit", detail } : { label: "Memory Edit" };
	},
	renderCall(args: MemoryEditRenderArgs, options: RenderResultOptions, theme: Theme): Component {
		const record = asRecord(args);
		const op = asString(record?.op)?.trim();
		const id = asString(record?.id)?.trim();
		const header = renderStatusLine(
			{
				icon: options.spinnerFrame !== undefined ? "running" : "pending",
				spinnerFrame: options.spinnerFrame,
				title: "Memory Edit",
				description: headerDetail([op, id ? shortenPath(id) : undefined].filter(Boolean).join(" ")),
			},
			theme,
		);
		return new Text(header, 0, 0);
	},
	renderResult(result: ToolResult, options: RenderResultOptions, theme: Theme, args?: unknown): Component {
		const text = resultText(result);
		if (result.isError) {
			return errorComponent(text || "Memory edit failed", theme);
		}
		const details = asRecord(result.details);
		const status = asString(details?.status)?.trim() ?? "";
		const disposition = MEMORY_EDIT_DISPOSITION[status] ?? { label: "unknown effect", settled: "warning" as const };
		const record = asRecord(args);
		const id = asString(record?.id)?.trim() ?? "";
		const header = renderStatusLine(
			{
				icon: disposition.settled,
				title: "Memory Edit",
				description: headerDetail(id ? shortenPath(id) : undefined),
				meta: [disposition.label],
			},
			theme,
		);
		if (!options.expanded) {
			return new Text(header, 0, 0);
		}
		return framedBlock(theme, width => {
			const lines: string[] = [];
			const bank = asString(details?.bank)?.trim();
			const store = asString(details?.store)?.trim();
			const location = [bank, store].filter(Boolean).join(" / ");
			if (location) lines.push(theme.fg("dim", truncateToWidth(location, width, Ellipsis.Omit)));
			for (const line of bodyLines(text, PREVIEW_LIMITS.OUTPUT_EXPANDED)) {
				lines.push(theme.fg("toolOutput", truncateToWidth(line, width, Ellipsis.Omit)));
			}
			return {
				header,
				sections: lines.length > 0 ? [{ lines }] : [],
				state: disposition.settled,
				borderColor: "borderMuted",
				width,
			};
		});
	},
};

// =============================================================================
// learn
// =============================================================================

interface LearnRenderArgs {
	memory?: unknown;
	context?: unknown;
	skill?: unknown;
}

function learnMemory(args: unknown): string | undefined {
	return asString(asRecord(args)?.memory)?.trim() || undefined;
}

function learnSkill(args: unknown, details: unknown): string | undefined {
	const skillArgs = asRecord(asRecord(args)?.skill);
	const name = asString(skillArgs?.name)?.trim();
	if (name) return name;
	return asString(asRecord(details)?.skill)?.trim() || undefined;
}

export const learnToolRenderer = {
	inline: true,
	mergeCallAndResult: true,
	activitySummary(args: unknown): ToolActivitySummary {
		const memory = learnMemory(args);
		const skill = learnSkill(args, undefined);
		const detail = skill ? `skill ${skill}` : memory ? previewLine(memory, TRUNCATE_LENGTHS.CONTENT) : undefined;
		return detail ? { label: "Learn", detail } : { label: "Learn" };
	},
	renderCall(args: LearnRenderArgs, options: RenderResultOptions, theme: Theme): Component {
		const header = renderStatusLine(
			{
				icon: options.spinnerFrame !== undefined ? "running" : "pending",
				spinnerFrame: options.spinnerFrame,
				title: "Learn",
				description: headerDetail(
					learnSkill(args, undefined) ? `skill ${learnSkill(args, undefined)}` : learnMemory(args),
				),
			},
			theme,
		);
		return new Text(header, 0, 0);
	},
	renderResult(result: ToolResult, options: RenderResultOptions, theme: Theme, args?: unknown): Component {
		const text = resultText(result);
		if (result.isError) {
			return errorComponent(text || "Learn failed", theme);
		}
		const memory = learnMemory(args);
		const skill = learnSkill(args, result.details);
		const header = renderStatusLine(
			{
				icon: "success",
				title: "Learn",
				description: headerDetail(skill ? `skill ${skill}` : memory),
				meta: skill ? ["lesson stored", `skill ${skill}`] : ["lesson stored"],
			},
			theme,
		);
		if (!options.expanded) {
			return new Text(header, 0, 0);
		}
		return framedBlock(theme, width => {
			const lines: string[] = [];
			if (memory) {
				for (const line of bodyLines(memory, PREVIEW_LIMITS.OUTPUT_EXPANDED)) {
					lines.push(theme.fg("toolOutput", truncateToWidth(line, width, Ellipsis.Omit)));
				}
			}
			for (const line of bodyLines(text, PREVIEW_LIMITS.OUTPUT_EXPANDED)) {
				const rendered = theme.fg("dim", truncateToWidth(line, width, Ellipsis.Omit));
				if (!lines.includes(rendered)) lines.push(rendered);
			}
			return { header, sections: lines.length > 0 ? [{ lines }] : [], state: "success", width };
		});
	},
};

// =============================================================================
// manage_skill
// =============================================================================

interface ManageSkillRenderArgs {
	action?: unknown;
	name?: unknown;
}

const MANAGE_SKILL_VERB: Record<string, string> = {
	create: "created",
	update: "updated",
	delete: "deleted",
};

export const manageSkillToolRenderer = {
	inline: true,
	mergeCallAndResult: true,
	activitySummary(args: unknown): ToolActivitySummary {
		const record = asRecord(args);
		const action = asString(record?.action)?.trim();
		const name = asString(record?.name)?.trim();
		const detail = [action, name].filter(Boolean).join(" ");
		return detail ? { label: "Manage Skill", detail } : { label: "Manage Skill" };
	},
	renderCall(args: ManageSkillRenderArgs, options: RenderResultOptions, theme: Theme): Component {
		const record = asRecord(args);
		const action = asString(record?.action)?.trim();
		const name = asString(record?.name)?.trim();
		const header = renderStatusLine(
			{
				icon: options.spinnerFrame !== undefined ? "running" : "pending",
				spinnerFrame: options.spinnerFrame,
				title: "Manage Skill",
				description: headerDetail([action, name].filter(Boolean).join(" ")),
			},
			theme,
		);
		return new Text(header, 0, 0);
	},
	renderResult(result: ToolResult, options: RenderResultOptions, theme: Theme, args?: unknown): Component {
		const text = resultText(result);
		if (result.isError) {
			return errorComponent(text || "Manage skill failed", theme);
		}
		const details = asRecord(result.details);
		const record = asRecord(args);
		const action = asString(details?.action)?.trim() ?? asString(record?.action)?.trim() ?? "";
		const name = asString(details?.name)?.trim() ?? asString(record?.name)?.trim() ?? "";
		const verb = MANAGE_SKILL_VERB[action] ?? action;
		const header = renderStatusLine(
			{
				icon: "success",
				title: "Manage Skill",
				description: headerDetail(name),
				meta: verb ? [verb] : undefined,
			},
			theme,
		);
		if (!options.expanded) {
			return new Text(header, 0, 0);
		}
		return framedBlock(theme, width => {
			const lines: string[] = [];
			for (const line of bodyLines(text, PREVIEW_LIMITS.OUTPUT_EXPANDED)) {
				lines.push(theme.fg("toolOutput", truncateToWidth(line, width, Ellipsis.Omit)));
			}
			return { header, sections: lines.length > 0 ? [{ lines }] : [], state: "success", width };
		});
	},
};

// =============================================================================
// search_code
// =============================================================================

interface SearchCodeRenderArgs {
	query?: unknown;
	mode?: unknown;
	limit?: unknown;
}

function searchCodeQuery(args: unknown, details: unknown): string {
	const fromArgs = asString(asRecord(args)?.query)?.trim();
	if (fromArgs) return fromArgs;
	return asString(asRecord(details)?.query)?.trim() ?? "";
}

interface SearchCodeHit {
	label: string;
	path?: string;
	line?: number;
}

function parseSearchCodeHits(text: string): SearchCodeHit[] {
	const hits: SearchCodeHit[] = [];
	for (const raw of text.split("\n")) {
		const line = raw.trim();
		if (!line.startsWith("- ")) continue;
		const location = line.match(/`([^`]+)`/);
		const name = line.match(/\*\*(.+?)\*\*/)?.[1]?.trim();
		const target = location?.[1]?.trim();
		if (!target) {
			hits.push({ label: name ?? line.slice(2).trim() });
			continue;
		}
		const fileMatch = target.match(/^(.*?):(\d+)$/);
		if (fileMatch?.[1]) {
			const lineNumber = fileMatch[2] !== undefined ? Number(fileMatch[2]) : undefined;
			hits.push({
				label: name ?? target,
				path: fileMatch[1].trim(),
				...(lineNumber !== undefined && Number.isFinite(lineNumber) ? { line: lineNumber } : {}),
			});
		} else {
			hits.push({ label: name ?? target, path: target });
		}
	}
	return hits;
}

function formatSearchCodeHit(hit: SearchCodeHit, width: number, theme: Theme): string {
	const label = truncateToWidth(replaceTabs(hit.label), TRUNCATE_LENGTHS.SHORT, Ellipsis.Unicode);
	if (!hit.path) return `${theme.fg("dim", "•")} ${theme.fg("toolOutput", label)}`;
	const short = shortenPath(hit.path);
	const display = hit.line !== undefined ? `${short}:${hit.line}` : short;
	const link = fileHyperlink(hit.path, display, hit.line !== undefined ? { line: hit.line } : undefined);
	const location = truncateToWidth(link, Math.max(8, width - Bun.stringWidth(label) - 4), Ellipsis.Omit);
	return `${theme.fg("dim", "•")} ${theme.fg("toolOutput", label)} ${theme.fg("muted", location)}`;
}

export const searchCodeToolRenderer = {
	inline: true,
	mergeCallAndResult: true,
	activitySummary(args: unknown): ToolActivitySummary {
		const query = asString(asRecord(args)?.query)?.trim();
		const detail = query ? previewLine(query, TRUNCATE_LENGTHS.CONTENT) : undefined;
		return detail ? { label: "Search Code", detail } : { label: "Search Code" };
	},
	renderCall(args: SearchCodeRenderArgs, options: RenderResultOptions, theme: Theme): Component {
		const header = renderStatusLine(
			{
				icon: options.spinnerFrame !== undefined ? "running" : "pending",
				spinnerFrame: options.spinnerFrame,
				title: "Search Code",
				description: headerDetail(asString(args?.query)),
			},
			theme,
		);
		return new Text(header, 0, 0);
	},
	renderResult(result: ToolResult, options: RenderResultOptions, theme: Theme, args?: unknown): Component {
		const text = resultText(result);
		if (result.isError) {
			const header = renderStatusLine({ icon: "error", title: "Search Code" }, theme);
			return framedBlock(theme, width => ({
				header,
				sections: [
					{
						lines: formatErrorDetail(text || "Code search failed", theme)
							.split("\n")
							.map(line => truncateToWidth(line, width, Ellipsis.Omit)),
					},
				],
				state: "error",
				borderColor: "error",
				width,
			}));
		}
		const details = asRecord(result.details);
		const query = searchCodeQuery(args, result.details);
		const total = asNumber(details?.totalMatches);
		const hits = parseSearchCodeHits(text);
		const count = total ?? hits.length;
		const noMatches = count === 0 || /no matches found/i.test(text);
		const header = renderStatusLine(
			noMatches
				? { icon: "warning", title: "Search Code", description: headerDetail(query), meta: ["no matches"] }
				: {
						icon: "success",
						title: "Search Code",
						description: headerDetail(query),
						meta: [`${count} ${count === 1 ? "match" : "matches"}`],
					},
			theme,
		);
		if (noMatches) {
			return new Text(header, 0, 0);
		}
		return createCachedComponent(
			() => options.expanded,
			width => {
				const lines = [header];
				const limit = options.expanded ? PREVIEW_LIMITS.OUTPUT_EXPANDED : PREVIEW_LIMITS.OUTPUT_COLLAPSED;
				const shown = hits.slice(0, limit);
				for (const hit of shown) {
					lines.push(truncateToWidth(formatSearchCodeHit(hit, width, theme), width, Ellipsis.Omit));
				}
				const remaining = hits.length - shown.length;
				if (remaining > 0) {
					lines.push(
						`${theme.fg("dim", formatMoreItems(remaining, "match"))} ${formatExpandHint(theme, options.expanded, true)}`,
					);
				} else if (!options.expanded && hits.length === 0) {
					for (const line of bodyLines(text, PREVIEW_LIMITS.OUTPUT_COLLAPSED)) {
						lines.push(theme.fg("toolOutput", truncateToWidth(line, width, Ellipsis.Omit)));
					}
				}
				return lines;
			},
		);
	},
};

// =============================================================================
// sessions
// =============================================================================

function sessionActionDetail(args: unknown, details: unknown): string | undefined {
	const record = asRecord(args);
	const detailRecord = asRecord(details);
	const action = asString(record?.action)?.trim() ?? asString(detailRecord?.action)?.trim() ?? "";
	const target = asString(record?.sessionId)?.trim() ?? asString(detailRecord?.id)?.trim() ?? "";
	const detail = [action, target].filter(Boolean).join(" ");
	return detail || undefined;
}

export const sessionsToolRenderer = {
	inline: true,
	mergeCallAndResult: true,
	activitySummary(args: unknown): ToolActivitySummary {
		const detail = sessionActionDetail(args, undefined);
		return detail ? { label: "Sessions", detail } : { label: "Sessions" };
	},
	renderCall(args: Record<string, unknown>, options: RenderResultOptions, theme: Theme): Component {
		const header = renderStatusLine(
			{
				icon: options.spinnerFrame !== undefined ? "running" : "pending",
				spinnerFrame: options.spinnerFrame,
				title: "Sessions",
				description: headerDetail(sessionActionDetail(args, undefined)),
			},
			theme,
		);
		return new Text(header, 0, 0);
	},
	renderResult(result: ToolResult, options: RenderResultOptions, theme: Theme, args?: unknown): Component {
		const text = resultText(result);
		if (result.isError) {
			return errorComponent(text || "Sessions failed", theme);
		}
		const header = renderStatusLine(
			{
				icon: "success",
				title: "Sessions",
				description: headerDetail(sessionActionDetail(args, result.details)),
			},
			theme,
		);
		if (!options.expanded) {
			return new Text(header, 0, 0);
		}
		return framedBlock(theme, width => {
			const lines: string[] = [];
			for (const line of bodyLines(text, PREVIEW_LIMITS.OUTPUT_EXPANDED)) {
				lines.push(theme.fg("toolOutput", truncateToWidth(line, width, Ellipsis.Omit)));
			}
			return { header, sections: lines.length > 0 ? [{ lines }] : [], state: "success", width };
		});
	},
};

// =============================================================================
// presets
// =============================================================================

function presetActionDetail(args: unknown, details: unknown): string | undefined {
	const record = asRecord(args);
	const detailRecord = asRecord(details);
	const action = asString(record?.action)?.trim() ?? asString(detailRecord?.action)?.trim() ?? "";
	const target = asString(record?.name)?.trim() ?? asString(detailRecord?.name)?.trim() ?? "";
	const detail = [action, target].filter(Boolean).join(" ");
	return detail || undefined;
}

export const presetsToolRenderer = {
	inline: true,
	mergeCallAndResult: true,
	activitySummary(args: unknown): ToolActivitySummary {
		const detail = presetActionDetail(args, undefined);
		return detail ? { label: "Presets", detail } : { label: "Presets" };
	},
	renderCall(args: Record<string, unknown>, options: RenderResultOptions, theme: Theme): Component {
		const header = renderStatusLine(
			{
				icon: options.spinnerFrame !== undefined ? "running" : "pending",
				spinnerFrame: options.spinnerFrame,
				title: "Presets",
				description: headerDetail(presetActionDetail(args, undefined)),
			},
			theme,
		);
		return new Text(header, 0, 0);
	},
	renderResult(result: ToolResult, options: RenderResultOptions, theme: Theme, args?: unknown): Component {
		const text = resultText(result);
		if (result.isError) {
			return errorComponent(text || "Presets failed", theme);
		}
		const header = renderStatusLine(
			{
				icon: "success",
				title: "Presets",
				description: headerDetail(presetActionDetail(args, result.details)),
			},
			theme,
		);
		if (!options.expanded) {
			return new Text(header, 0, 0);
		}
		return framedBlock(theme, width => {
			const lines: string[] = [];
			for (const line of bodyLines(text, PREVIEW_LIMITS.OUTPUT_EXPANDED)) {
				lines.push(theme.fg("toolOutput", truncateToWidth(line, width, Ellipsis.Omit)));
			}
			return { header, sections: lines.length > 0 ? [{ lines }] : [], state: "success", width };
		});
	},
};
