import { type } from "@harvest/omptype";
import type { AgentTool, AgentToolResult } from "@harvest/pi-agent-core";
import { Serial } from "@harvest/pi-utils";
import type { AgentRef, AgentRegistry } from "../registry/agent-registry";
import { resolveSpawnPolicy } from "../task/spawn-policy";
import { canSpawnAtDepth } from "../task/types";
import sessionsDescription from "../prompts/tools/sessions.md" with { type: "text" };
import type { ToolSession } from ".";

const sessionsSchema = type({
	action: "'list' | 'inspect' | 'create' | 'rename' | 'send' | 'stop'",
	"sessionId?": type("string").describe("target session id (required except for list/create)"),
	"title?": type("string").describe("new title for rename"),
	"message?": type("string").describe("message to deliver for send"),
	"task?": type("string").describe("first task for a created session to run"),
	"confirm?": type("boolean").describe("explicit confirmation for stopping a running session"),
}).narrow(
	(p, ctx) =>
		p.action === "list" ||
		p.action === "create" ||
		p.sessionId !== undefined ||
		ctx.mustBe('used with "sessionId" for every action except "list" and "create"'),
);

export type SessionsParams = typeof sessionsSchema.infer;

/** Minimal live-session surface the sessions tool operates on. */
export interface ManagedLiveSessionLike {
	getSessionId(): string;
	abort(): Promise<void> | void;
	setSessionName?(title: string, source?: string): Promise<void> | void;
	prompt?(message: string, options?: Record<string, unknown>): Promise<unknown>;
}

export interface OpenManagedSessionInput {
	cwd: string;
	task?: string;
	callerId: string | null;
	/** Caller task depth + 1, computed by the trusted tool (never model input). */
	taskDepth?: number;
}

export interface OpenedManagedSession {
	/** Public stable identity: the session UUID. */
	id: string;
	/** Owner-registry identity (AgentRegistry/IRC), mapped explicitly — never invented by callers. */
	registryId: string;
	/** True when the supplied initial task was dispatched exactly once in the background. */
	taskAccepted: boolean;
	session: ManagedLiveSessionLike;
}

export interface SessionToolDeps {
	/**
	 * Live-session factory seam. Production wiring opens an independent
	 * runtime via the live-session factory pattern and adopts it into the
	 * existing owner registry with hasUI:false — background, never stealing
	 * focus (owner patch: sdk-provided factory on ToolSession).
	 */
	openSession?: (input: OpenManagedSessionInput) => Promise<OpenedManagedSession>;
	/** Message delivery seam (defaults to the live session's prompt entry). */
	deliverMessage?: (target: ManagedLiveSessionLike, message: string) => Promise<string>;
	/** Registry override (defaults to the session's agent registry). */
	registry?: AgentRegistry;
}

let sessionToolDeps: SessionToolDeps = {};

/** Serializes capacity check + creation so parallel creates cannot oversubscribe maxConcurrency. */
const createSerial = new Serial();

/** Override sessions-tool seams (tests inject deterministic fixtures). */
export function setSessionToolDeps(deps: SessionToolDeps): void {
	sessionToolDeps = { ...sessionToolDeps, ...deps };
}

/** Test-only: release sessions-tool seams. */
export function clearSessionToolDepsForTests(): void {
	sessionToolDeps = {};
}

/**
 * Model-callable live-session management: list/inspect/create/rename/send/
 * stop over the existing owner registry. Approvals: list/inspect are read,
 * everything else is exec. Creation honors `task.maxConcurrency`,
 * `task.maxRecursionDepth`/`canSpawnAtDepth`, and the session spawn policy;
 * targets stay within the current owned lineage + authorized project unless
 * the host provided a trusted grant. Self-stop is always refused (no
 * synchronous self-stop/delete deadlock).
 */
export class SessionsTool implements AgentTool<typeof sessionsSchema> {
	readonly name = "sessions";
	readonly approval = (args: unknown): "read" | "exec" => {
		const action = (args as Partial<SessionsParams>).action;
		return action === "list" || action === "inspect" ? "read" : "exec";
	};
	readonly label = "Sessions";
	readonly description = sessionsDescription;
	readonly parameters = sessionsSchema;
	readonly strict = true;
	readonly loadMode = "essential" as const;
	readonly summary = "List, inspect, create, rename, message, or stop background sessions";

	constructor(private readonly session: ToolSession) {}

	static createIf(session: ToolSession): SessionsTool | null {
		if (!session.settings.get("autolearn.enabled")) return null;
		return new SessionsTool(session);
	}

	async execute(_id: string, params: SessionsParams): Promise<AgentToolResult> {
		switch (params.action) {
			case "list":
				return this.listSessions();
			case "inspect":
				return this.inspectSession(params);
			case "create":
				return this.createSession(params);
			case "rename":
				return this.renameSession(params);
			case "send":
				return this.sendMessage(params);
			case "stop":
				return this.stopSession(params);
			default:
				throw new Error(`Unknown sessions action "${(params as { action: string }).action}".`);
		}
	}

	private registry(): AgentRegistry {
		const registry = sessionToolDeps.registry ?? this.session.agentRegistry;
		if (!registry) throw new Error("No agent registry is wired for session management.");
		return registry;
	}

	private callerId(): string | null {
		return this.session.getSessionId?.() ?? this.session.getAgentId?.() ?? null;
	}

	private findRef(sessionId: string): AgentRef {
		const registry = this.registry();
		const direct = registry.get(sessionId);
		if (direct) return direct;
		// Public stable identity is the session UUID; map it explicitly via
		// the live runtime fact. Callers never invent aliases.
		for (const ref of registry.list()) {
			if (this.liveUuid(ref) === sessionId) return ref;
		}
		throw new Error(`Session ${sessionId} is not live.`);
	}

	/** Host-provided grant authorizing broader targets; model params can never mint one. */
	private trustedGrant(): { scope: string; expiresAt: number } | undefined {
		const grant = this.session.managedSessionGrant;
		if (!grant || grant.scope !== "sessions:manage" || Date.now() >= grant.expiresAt) return undefined;
		return grant;
	}

	/**
	 * Default access: the current owned lineage (self + direct children) in
	 * the authorized project, verified against trusted runtime facts.
	 * Model-supplied flags are intentions, not authorization: broader targets
	 * require a host-provided grant or the existing trusted approval path.
	 * Returns an error message when access is denied, null when allowed.
	 */
	private checkTargetAccess(ref: AgentRef): string | null {
		const caller = this.callerId();
		if (caller && ref.id === caller) return null;
		if (caller && ref.parentId === caller) return null;
		if (this.trustedGrant()) return null;
		if (!caller) {
			return `Session ${ref.id} is outside the current lineage; broader targets require a host-provided grant or the existing approval path.`;
		}
		if (ref.parentId && ref.parentId !== caller) {
			return `Session ${ref.id} belongs to another lineage (parent ${ref.parentId}); broader targets require a host-provided grant or the existing approval path.`;
		}
		return `Session ${ref.id} is outside the current owned lineage; broader targets require a host-provided grant or the existing approval path.`;
	}

	/** Stable UUID behind a registry ref, when its runtime exposes one. */
	private liveUuid(ref: AgentRef): string | undefined {
		try {
			const live = ref.session as unknown as { sessionManager?: { getSessionId?: () => string } } | null;
			return live?.sessionManager?.getSessionId?.();
		} catch {
			// A racing detach must not break lookup of the other refs.
			return undefined;
		}
	}

	private liveSession(ref: AgentRef): ManagedLiveSessionLike {
		const live = ref.session as unknown as ManagedLiveSessionLike | null;
		if (!live || typeof live.abort !== "function") {
			throw new Error(`Session ${ref.id} has no live runtime (status: ${ref.status}).`);
		}
		return live;
	}

	private async listSessions(): Promise<AgentToolResult> {
		const caller = this.callerId();
		const refs = this.registry().list();
		const lines = refs.map(
			ref =>
				`- ${ref.id} (${ref.kind}, ${ref.status}${ref.id === caller ? ", caller" : ""}${ref.parentId ? `, parent ${ref.parentId}` : ""}): ${ref.displayName}`,
		);
		return {
			content: [{ type: "text", text: `Live sessions (${refs.length}):\n${lines.join("\n")}` }],
			details: {
				action: "list",
				sessions: refs.map(ref => ({
					id: ref.id,
					kind: ref.kind,
					status: ref.status,
					parentId: ref.parentId,
					caller: ref.id === caller,
				})),
			},
		};
	}

	private async inspectSession(params: SessionsParams): Promise<AgentToolResult> {
		if (!params.sessionId) throw new Error(`"inspect" requires "sessionId".`);
		const ref = this.findRef(params.sessionId);
		return {
			content: [
				{
					type: "text",
					text:
						`Session ${ref.id}: ${ref.displayName} (${ref.kind}, ${ref.status}).` +
						(ref.sessionFile ? ` File: ${ref.sessionFile}.` : ` No session file.`) +
						(ref.activity ? ` Activity: ${ref.activity}.` : ``),
				},
			],
			details: {
				action: "inspect",
				id: ref.id,
				kind: ref.kind,
				status: ref.status,
				parentId: ref.parentId,
				sessionFile: ref.sessionFile,
				caller: ref.id === this.callerId(),
			},
		};
	}

	private async createSession(params: SessionsParams): Promise<AgentToolResult> {
		// Spawn policy: the caller must be allowed to spawn at all.
		const spawnPolicy = resolveSpawnPolicy(this.session.getSessionSpawns());
		if (!spawnPolicy.enabled) {
			throw new Error("Spawning is disabled for this session; cannot create a managed session.");
		}
		// Recursion depth: mirrors the task-tool availability gate. Children
		// run one level deeper than the caller.
		const maxDepth = this.session.settings.get("task.maxRecursionDepth") ?? 2;
		const callerDepth = this.session.taskDepth ?? 0;
		if (!canSpawnAtDepth(maxDepth, callerDepth)) {
			throw new Error(`Cannot create a managed session at depth ${callerDepth} (maxRecursionDepth ${maxDepth}).`);
		}
		// Project ownership: stay in the authorized project unless explicitly granted.
		const cwd = this.session.cwd;
		const openSession =
			(this.session.openManagedSession as SessionToolDeps["openSession"]) ?? sessionToolDeps.openSession;
		if (!openSession) {
			throw new Error(
				"Live session creation is not wired for this host (interactive TUI registers the live-session factory; headless hosts report this instead of creating).",
			);
		}
		// Capacity check + creation are serialized so two concurrent callers
		// cannot both observe a free slot and oversubscribe maxConcurrency.
		return createSerial.run(async () => {
			const max = this.session.settings.get("task.maxConcurrency") ?? 32;
			if (max > 0) {
				const running = this.registry()
					.list()
					.filter(ref => ref.status === "running").length;
				if (running >= max) {
					throw new Error(
						`Cannot create a managed session: ${running} session(s) already running (task.maxConcurrency ${max}).`,
					);
				}
			}
			// Model-created independent sessions start in the background and never
			// steal focus: the factory seam contract requires hasUI:false and no
			// selection change; this path never touches tab visibility itself.
			const opened = await openSession({
				cwd,
				task: params.task,
				callerId: this.callerId(),
				taskDepth: callerDepth + 1,
			});
			const taskNote = params.task?.trim()
				? opened.taskAccepted
					? ` Its initial task was dispatched once in the background.`
					: ` Its initial task was NOT dispatched; send it explicitly.`
				: ``;
			return {
				content: [
					{
						type: "text",
						text:
							`Created background session ${opened.id} in ${cwd} (registry ${opened.registryId}).` +
							` It runs independently and does not steal focus.${taskNote}`,
					},
				],
				details: {
					action: "create",
					id: opened.id,
					registryId: opened.registryId,
					taskAccepted: opened.taskAccepted,
					background: true,
					cwd,
				},
			};
		});
	}

	private async renameSession(params: SessionsParams): Promise<AgentToolResult> {
		if (!params.sessionId) throw new Error(`"rename" requires "sessionId".`);
		if (!params.title?.trim()) throw new Error(`"rename" requires "title".`);
		const ref = this.findRef(params.sessionId);
		const denied = this.checkTargetAccess(ref);
		if (denied) throw new Error(denied);
		const live = this.liveSession(ref);
		if (typeof live.setSessionName !== "function") {
			throw new Error(`Session ${ref.id} cannot be renamed through this host.`);
		}
		await live.setSessionName(params.title.trim(), "user");
		return {
			content: [{ type: "text", text: `Renamed session ${ref.id} to "${params.title.trim()}".` }],
			details: { action: "rename", id: ref.id },
		};
	}

	private async sendMessage(params: SessionsParams): Promise<AgentToolResult> {
		if (!params.sessionId) throw new Error(`"send" requires "sessionId".`);
		if (!params.message?.trim()) throw new Error(`"send" requires "message".`);
		const ref = this.findRef(params.sessionId);
		const denied = this.checkTargetAccess(ref);
		if (denied) throw new Error(denied);
		const live = this.liveSession(ref);
		const deliver = sessionToolDeps.deliverMessage;
		const receipt = deliver
			? await deliver(live, params.message.trim())
			: typeof live.prompt === "function"
				? await live.prompt(params.message.trim()).then(() => `delivered to ${ref.id}`)
				: null;
		if (!receipt) throw new Error(`Session ${ref.id} cannot receive messages through this host.`);
		return {
			content: [{ type: "text", text: `Sent message to session ${ref.id} (${receipt}).` }],
			details: { action: "send", id: ref.id },
		};
	}

	private async stopSession(params: SessionsParams): Promise<AgentToolResult> {
		if (!params.sessionId) throw new Error(`"stop" requires "sessionId".`);
		const ref = this.findRef(params.sessionId);
		// No synchronous self-stop/delete deadlock: the caller can never abort
		// its own run through this path — matched by registry ID, public UUID,
		// or live runtime identity.
		const caller = this.callerId();
		const callerUuid = this.session.getSessionId?.() ?? null;
		if (params.sessionId === caller || ref.id === caller || (callerUuid && this.liveUuid(ref) === callerUuid)) {
			throw new Error(
				`Refusing to stop the calling session itself through the sessions tool (would deadlock the synchronous call).`,
			);
		}
		const denied = this.checkTargetAccess(ref);
		if (denied) throw new Error(denied);
		// Destructive targets need explicit confirmation while running.
		if (ref.status === "running" && params.confirm !== true) {
			throw new Error(
				`Session ${ref.id} is running; pass confirm:true to stop it explicitly. Idle sessions stop without confirmation.`,
			);
		}
		await this.liveSession(ref).abort();
		return {
			content: [{ type: "text", text: `Stopped session ${ref.id}.` }],
			details: { action: "stop", id: ref.id },
		};
	}
}
