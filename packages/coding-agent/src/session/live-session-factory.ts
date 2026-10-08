import type { Model } from "@harvest/pi-ai";
import { logger } from "@harvest/pi-utils";
import { EventBus } from "../utils/event-bus";
import type { MCPManager } from "../mcp";
import type { AgentRegistry } from "../registry/agent-registry";
import type { AuthStorage } from "./auth-storage";
import type { ModelRegistry } from "../config/model-registry";
import type { Settings } from "../config/settings";
import type { EffectiveExtensionRoots } from "../capability/types";
import { createAgentSession, type CreateAgentSessionResult } from "../sdk";
import type { AgentSession } from "./agent-session";
import { SessionManager } from "./session-manager";
import type { ToolSession } from "../tools/index";

/** Inputs for opening one live tab runtime. */
export interface OpenLiveSessionOptions {
	/** Canonical project directory; every live tab shares one project. */
	cwd: string;
	/** Project session directory backing tab persistence. */
	sessionDir: string;
	/** Existing file to cold-open; omitted for a fresh tab. */
	sessionPath?: string;
	/** Parent settings; cloned per session so session-scoped overrides never leak. */
	settings: Settings;
	/** Shared credential storage (pinned to the model registry). */
	authStorage: AuthStorage;
	/** Shared model registry. */
	modelRegistry: ModelRegistry;
	/** Model the new tab inherits (normally the current session's). */
	model: Model;
	/** Unique agent identity for IRC routing; defaults to `tab:<sessionId>`. */
	agentId?: string;
	/** Live extension-root policy inherited from the parent session. */
	extensionRoots?: () => EffectiveExtensionRoots;
	/** Pre-discovered extension paths (skips the fs scan; each session still binds its own API). */
	preloadedExtensionPaths?: string[];
	/** Shared MCP connections; omit with `enableMCP: false` for headless parity. */
	mcpManager?: MCPManager;
	enableMCP?: boolean;
	/**
	 * Tool surface the child may use. Inherited from the trusted caller
	 * snapshot: a child never gains tools its caller lacks. Omitted for a
	 * full-surface user-opened tab.
	 */
	toolNames?: string[];
	/**
	 * Constrain the child to its explicit tool names. Inherited, never
	 * relaxed: a restricted caller cannot spawn an unrestricted child.
	 */
	restrictToolNames?: boolean;
	/**
	 * Spawn frontmatter for the child (`*` default). Children of a
	 * spawn-disabled caller stay disabled; the tool layer additionally caps
	 * depth via `task.maxRecursionDepth`.
	 */
	spawns?: string;
	/**
	 * Auto-approve policy for the child. Never escalated: defaults to false
	 * unless the trusted caller snapshot explicitly carries approval.
	 */
	autoApprove?: boolean;
	/** Fresh buses per tab; default to new instances. */
	eventBus?: EventBus;
	subagentEventBus?: EventBus;
	/** Private IRC registry; defaults to the process-global one. */
	agentRegistry?: AgentRegistry;
	/**
	 * Registry identity of the spawning agent, recorded as the child's
	 * parent for lineage checks. Resolved from trusted runtime facts by the
	 * model-session factory; never from model params.
	 */
	parentAgentId?: string;
	/**
	 * Task depth for the child (caller's depth + 1, computed by the trusted
	 * caller, not the model). Default: 0.
	 */
	taskDepth?: number;
	/**
	 * Background model-created session: no UI ownership, never steals focus,
	 * adopted into the owner registry without a selection change.
	 */
	background?: boolean;
	/**
	 * Owner-scoped live-session factory carried onto the child's ToolSession
	 * (S3): model-created descendants create through the same owning host
	 * instead of whichever global was installed last. Omitted for ordinary
	 * user-opened tabs, which resolve the seam normally.
	 */
	openManagedSession?: ToolSession["openManagedSession"];
	/** Session factory seam (defaults to {@link createAgentSession}); tests inject a stub. */
	createSession?: (options: Record<string, unknown>) => Promise<CreateAgentSessionResult>;
}

/** Tool, spawn, and permission policy inherited from a trusted caller. A child never exceeds its caller. */
export interface TrustedCallerPolicy {
	/** Explicit tool surface; undefined means the caller's full surface. */
	toolNames?: string[];
	/** Restrict to explicit tool names; inherited, never relaxed. */
	restrictToolNames?: boolean;
	/** Spawn frontmatter (`*` default); spawn-disabled callers beget disabled children. */
	spawns?: string | null;
	/** Auto-approve; never escalated by the child path. */
	autoApprove?: boolean;
	/** Whether MCP capabilities may be forwarded; false prohibits inherited managers. */
	enableMCP?: boolean;
}

/**
 * Trusted snapshot of the spawning caller, built from live runtime facts —
 * never from model params. The factory applies it verbatim; narrowing
 * (depth + 1, background hasUI:false) happens here, once, so every owner
 * (interactive tabs, background model-created sessions) inherits identical
 * limits.
 */
export interface TrustedCallerSnapshot {
	/** Canonical project directory; the child cannot escape it. */
	cwd: string;
	/** Project session directory backing tab persistence. */
	sessionDir: string;
	/** Parent settings; cloned per session so overrides never leak. */
	settings: Settings;
	/** Shared credential storage (pinned to the model registry). */
	authStorage: AuthStorage;
	/** Shared model registry. */
	modelRegistry: ModelRegistry;
	/** Model the child inherits (normally the caller's). */
	model: Model;
	/** Stable session UUID of the caller, when known from runtime facts. */
	callerSessionId?: string;
	/** Registry identity of the caller, recorded as the child's parent. */
	callerAgentId?: string;
	/** Task depth for the child: caller depth + 1. */
	taskDepth: number;
	/** Inherited tool/spawn/permission policy. */
	policy: TrustedCallerPolicy;
	/** Live extension-root policy inherited from the caller. */
	extensionRoots?: () => EffectiveExtensionRoots;
	/** Pre-discovered extension paths (each session still binds its own API). */
	preloadedExtensionPaths?: string[];
	/** Shared MCP connections; absent with `enableMCP: false`. */
	mcpManager?: MCPManager;
	/** Fresh buses; default to new instances. */
	eventBus?: EventBus;
	subagentEventBus?: EventBus;
	/** Private IRC registry; defaults to the process-global one. */
	agentRegistry?: AgentRegistry;
}

/**
 * Requested tool/spawn/permission limits for one child open, from the party
 * asking for the child (model-facing tool params or a ToolSession forward).
 * Intersected with the live caller's limits by {@link intersectCallerPolicy};
 * omission on either side never widens the other side.
 */
export interface RequestedChildPolicy {
	toolNames?: string[];
	restrictToolNames?: boolean;
	spawns?: string | null;
	autoApprove?: boolean;
	enableMCP?: boolean;
}

/**
 * Intersect live-caller limits with requested limits into the EFFECTIVE child
 * policy. The child never exceeds either side:
 *
 * - `toolNames`: both sides defined → caller-ordered intersection (an empty
 *   intersection stays an explicitly empty allowlist, never widened back to
 *   a full surface); one side defined → that side verbatim; neither →
 *   undefined (the caller's full surface).
 * - `restrictToolNames`: OR — a restriction stated by either side is kept,
 *   never relaxed.
 * - `spawns`: the live caller's frontmatter wins when stated (it is the
 *   authoritative runtime fact; the ToolSession forward carries the same
 *   caller's value). An unstated caller falls back to the request, then to
 *   `*`. A stated disable (`""`/`null`) is never widened by the other side.
 *   Requested-side narrowing below a stated caller value stays owned by the
 *   spawn-policy owner (see the handoff patch request).
 * - `autoApprove`: both sides defined → AND (never escalated); one side →
 *   that side verbatim; neither → false.
 * - `enableMCP`: both sides defined → AND (false prohibits forwarding);
 *   one side → that side verbatim (undefined preserved: only an explicit
 *   false drops the inherited manager).
 *
 * Omission never widens: an absent side imposes no constraint of its own and
 * takes the other side's limits as-is.
 */
export function intersectCallerPolicy(
	caller: TrustedCallerPolicy,
	requested: RequestedChildPolicy | undefined,
): TrustedCallerPolicy {
	const ask = requested ?? {};
	let toolNames: string[] | undefined;
	if (caller.toolNames !== undefined && ask.toolNames !== undefined) {
		const allowed = new Set(ask.toolNames);
		toolNames = caller.toolNames.filter(name => allowed.has(name));
	} else if (caller.toolNames !== undefined) {
		toolNames = [...caller.toolNames];
	} else if (ask.toolNames !== undefined) {
		toolNames = [...ask.toolNames];
	}
	const restrictToolNames = caller.restrictToolNames === true || ask.restrictToolNames === true;
	// Live runtime fact wins when stated; the forward only fills gaps, so a
	// stated disable (`""`/`null`) is never widened and an unstated caller
	// takes the request. Undefined-only chaining: `??` would also fall
	// through a stated `null` disable.
	const spawns = caller.spawns !== undefined ? caller.spawns : ask.spawns !== undefined ? ask.spawns : "*";
	let autoApprove: boolean;
	if (caller.autoApprove !== undefined && ask.autoApprove !== undefined) {
		autoApprove = caller.autoApprove && ask.autoApprove;
	} else {
		autoApprove = caller.autoApprove ?? ask.autoApprove ?? false;
	}
	let enableMCP: boolean | undefined;
	if (caller.enableMCP !== undefined && ask.enableMCP !== undefined) {
		enableMCP = caller.enableMCP && ask.enableMCP;
	} else {
		enableMCP = caller.enableMCP ?? ask.enableMCP;
	}
	return { toolNames, restrictToolNames: restrictToolNames || undefined, spawns, autoApprove, enableMCP };
}
export interface LiveSessionSource {
	sessionManager: Pick<SessionManager, "getCwd" | "getSessionDir">;
	session: Pick<AgentSession, "settings" | "modelRegistry" | "model" | "effectiveExtensionRoots"> & {
		getAvailableModels(): Model[];
	};
	mcpManager?: MCPManager;
}

/**
 * Richer caller surface for background (model-created) sessions: the trusted
 * ToolSession facts the sessions tool resolves server-side. Every field is
 * optional — absent fields fall back to the safe default (caller's full
 * surface for tools, `*` for spawns, no auto-approve, depth 0) — so older
 * owners keep working while newer ones propagate exact limits.
 */
export interface TrustedCallerSource extends LiveSessionSource {
	session: LiveSessionSource["session"] & {
		getSessionSpawns?: () => string | null;
		getAgentId?: () => string | null | undefined;
		taskDepth?: number;
		toolNames?: string[];
		restrictToolNames?: boolean;
		autoApprove?: boolean;
		enableMCP?: boolean;
	};
	getSessionId?: () => string | null;
	getAgentId?: () => string | null;
	taskDepth?: number;
}

/**
 * Build a trusted caller snapshot from live state: same project and session
 * dir, cloned-later parent settings, shared credentials/routing, the current
 * model (or the first available one at startup), the caller's tool/spawn/
 * permission policy, and fresh buses. The child depth is caller depth + 1.
 * Throws when no model resolves so callers fall back to the legacy flow.
 */
export function snapshotTrustedCaller(
	source: TrustedCallerSource,
	overrides?: {
		sessionPath?: string;
		callerSessionId?: string;
		callerAgentId?: string;
		/** Caller depth; the child runs one level deeper. */
		taskDepth?: number;
		/**
		 * Trusted ToolSession facts forwarded by the sessions tool. Merged
		 * UNDER live-session facts: any limit the live caller states wins;
		 * the fallback only fills gaps the live runtime does not expose
		 * (AgentSession carries no tool/spawn accessors). A restricted
		 * background caller therefore still begets a restricted child.
		 */
		policyFallback?: TrustedCallerPolicy;
	},
): TrustedCallerSnapshot & { sessionPath?: string } {
	const base = liveSessionFactoryOptions(source, overrides?.sessionPath);
	const session = source.session as TrustedCallerSource["session"];
	const callerDepth = overrides?.taskDepth ?? session.taskDepth ?? source.taskDepth ?? 0;
	const fallback = overrides?.policyFallback;
	const livePolicy: TrustedCallerPolicy = {
		toolNames: session.toolNames !== undefined ? [...session.toolNames] : undefined,
		restrictToolNames: session.restrictToolNames,
		spawns: session.getSessionSpawns?.(),
		autoApprove: session.autoApprove,
		enableMCP: session.enableMCP,
	};
	return {
		cwd: base.cwd,
		sessionDir: base.sessionDir,
		settings: base.settings,
		authStorage: base.authStorage,
		modelRegistry: base.modelRegistry,
		model: base.model,
		callerSessionId: overrides?.callerSessionId ?? source.getSessionId?.() ?? undefined,
		callerAgentId: overrides?.callerAgentId ?? session.getAgentId?.() ?? source.getAgentId?.() ?? undefined,
		taskDepth: callerDepth + 1,
		// EFFECTIVE policy: live-caller limits intersected with the forwarded
		// ToolSession facts, so a restricted background caller still begets a
		// restricted child and omission on either side never widens the other.
		policy: intersectCallerPolicy(livePolicy, fallback),
		extensionRoots: base.extensionRoots,
		preloadedExtensionPaths: base.preloadedExtensionPaths,
		mcpManager: base.mcpManager,
		eventBus: base.eventBus,
		subagentEventBus: base.subagentEventBus,
		agentRegistry: base.agentRegistry,
		sessionPath: base.sessionPath,
	};
}

/** Options without the factory seam, built from live interactive state. */
export type LiveSessionFactoryOptions = Omit<OpenLiveSessionOptions, "createSession">;

/**
 * Build cold-open options from live state: same project and session dir,
 * cloned-later parent settings, shared credentials/routing, the current
 * model (or the first available one at startup), and fresh buses. Throws
 * when no model resolves so callers fall back to the legacy in-place flow.
 */
export function liveSessionFactoryOptions(source: LiveSessionSource, sessionPath?: string): LiveSessionFactoryOptions {
	const parent = source.session;
	const model = parent.model ?? parent.getAvailableModels()[0];
	if (!model) throw new Error("Cannot open a live tab before a model is resolved");
	return {
		cwd: source.sessionManager.getCwd(),
		sessionDir: source.sessionManager.getSessionDir(),
		sessionPath,
		settings: parent.settings,
		authStorage: parent.modelRegistry.authStorage,
		modelRegistry: parent.modelRegistry,
		model,
		extensionRoots: () => parent.effectiveExtensionRoots,
		mcpManager: source.mcpManager,
	};
}

/**
 * Open an independent runtime for one live tab: a fresh settings clone, a
 * dedicated session manager (opened from `sessionPath` for cold reopens),
 * and a fully-wired session sharing only credentials, models, and —
 * explicitly — MCP connections with its parent. The trusted caller policy
 * (tools, spawns, auto-approve) is inherited verbatim, never widened: a
 * restricted caller cannot spawn an unrestricted child. The caller adopts
 * the result into {@link LiveSessionRegistry}; the previous runtime keeps
 * running untouched.
 */
export async function openLiveAgentSession(options: OpenLiveSessionOptions): Promise<AgentSession> {
	const settings = await options.settings.cloneForCwd(options.cwd);
	const sessionManager = options.sessionPath
		? await SessionManager.open(options.sessionPath, options.sessionDir)
		: SessionManager.create(options.cwd, options.sessionDir);
	const createSession = options.createSession ?? createAgentSession;
	try {
		const { session } = await createSession({
			cwd: options.cwd,
			sessionManager,
			settings,
			authStorage: options.authStorage,
			modelRegistry: options.modelRegistry,
			model: options.model,
			rebindModelAfterDiscovery: true,
			hasUI: options.background ? false : true,
			parentAgentId: options.parentAgentId,
			taskDepth: options.taskDepth,
			agentId: options.agentId ?? `tab:${sessionManager.getSessionId()}`,
			extensionRoots: options.extensionRoots,
			preloadedExtensionPaths: options.preloadedExtensionPaths,
			mcpManager: options.mcpManager,
			enableMCP: options.enableMCP,
			toolNames: options.toolNames,
			restrictToolNames: options.restrictToolNames,
			spawns: options.spawns,
			autoApprove: options.autoApprove,
			eventBus: options.eventBus ?? new EventBus(),
			subagentEventBus: options.subagentEventBus,
			agentRegistry: options.agentRegistry,
			openManagedSession: options.openManagedSession,
		});
		return session;
	} catch (error) {
		try {
			await sessionManager.close();
		} catch (closeError) {
			logger.warn("Failed to release unopened live session", { error: String(closeError) });
		}
		throw error;
	}
}

/**
 * Open a live session directly from a {@link snapshotTrustedCaller} result:
 * the snapshot's inherited policy becomes the factory input, so owners pass
 * one object instead of re-threading every limit. `overrides` covers only
 * per-open identity (agent ID, parent link, background); policy always comes
 * from the snapshot.
 */
export async function openLiveAgentSessionFromSnapshot(
	snapshot: TrustedCallerSnapshot & { sessionPath?: string },
	overrides?: {
		agentId?: string;
		parentAgentId?: string;
		background?: boolean;
		openManagedSession?: OpenLiveSessionOptions["openManagedSession"];
		createSession?: OpenLiveSessionOptions["createSession"];
	},
): Promise<AgentSession> {
	return openLiveAgentSession({
		cwd: snapshot.cwd,
		sessionDir: snapshot.sessionDir,
		sessionPath: snapshot.sessionPath,
		settings: snapshot.settings,
		authStorage: snapshot.authStorage,
		modelRegistry: snapshot.modelRegistry,
		model: snapshot.model,
		agentId: overrides?.agentId,
		parentAgentId: overrides?.parentAgentId ?? snapshot.callerAgentId,
		taskDepth: snapshot.taskDepth,
		toolNames: snapshot.policy.toolNames,
		restrictToolNames: snapshot.policy.restrictToolNames,
		spawns: snapshot.policy.spawns ?? undefined,
		autoApprove: snapshot.policy.autoApprove,
		extensionRoots: snapshot.extensionRoots,
		preloadedExtensionPaths: snapshot.preloadedExtensionPaths,
		mcpManager: snapshot.policy.enableMCP === false ? undefined : snapshot.mcpManager,
		enableMCP: snapshot.policy.enableMCP,
		eventBus: snapshot.eventBus,
		subagentEventBus: snapshot.subagentEventBus,
		agentRegistry: snapshot.agentRegistry,
		background: overrides?.background,
		openManagedSession: overrides?.openManagedSession,
		createSession: overrides?.createSession,
	});
}
