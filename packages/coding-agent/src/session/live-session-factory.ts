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
	/** Session factory seam (defaults to {@link createAgentSession}); tests inject a stub. */
	createSession?: (options: Record<string, unknown>) => Promise<CreateAgentSessionResult>;
}

/** Options without the factory seam, built from live interactive state. */
export type LiveSessionFactoryOptions = Omit<OpenLiveSessionOptions, "createSession">;

/** Minimal live state a tab opener is built from (implemented by InteractiveModeContext). */
export interface LiveSessionSource {
	sessionManager: Pick<SessionManager, "getCwd" | "getSessionDir">;
	session: Pick<AgentSession, "settings" | "modelRegistry" | "model" | "effectiveExtensionRoots"> & {
		getAvailableModels(): Model[];
	};
	mcpManager?: MCPManager;
}

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
 * explicitly — MCP connections with its parent. The caller adopts the result
 * into {@link LiveSessionRegistry}; the previous runtime keeps running
 * untouched.
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
			eventBus: options.eventBus ?? new EventBus(),
			subagentEventBus: options.subagentEventBus,
			agentRegistry: options.agentRegistry,
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
