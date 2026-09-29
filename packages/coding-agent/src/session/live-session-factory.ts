import type { Model } from "@harvest/pi-ai";
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
	/** Session factory seam (defaults to {@link createAgentSession}); tests inject a stub. */
	createSession?: (options: Record<string, unknown>) => Promise<CreateAgentSessionResult>;
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
	const { session } = await createSession({
		cwd: options.cwd,
		sessionManager,
		settings,
		authStorage: options.authStorage,
		modelRegistry: options.modelRegistry,
		model: options.model,
		rebindModelAfterDiscovery: true,
		hasUI: true,
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
}
