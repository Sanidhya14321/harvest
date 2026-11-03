/** Transitional rejection boundary; historical result types remain readable. */
import type { ToolSession } from "../../tools";
import type { JsStatusEvent } from "../js/shared/types";
import type { EvalToolDescriptor, EvalToolInvokeResult } from "../types";
import type { KernelExecuteOptions, KernelExecuteResult } from "../kernel-base";
import type { KernelDisplayOutput } from "./display";
import type { PythonPreludeSource } from "./kernel";
import retired from "../../prompts/python-eval-retired.md" with { type: "text" };
export const PYTHON_EVAL_RETIRED = retired.trim();
export type PythonKernelMode = "session" | "per-call";

/** Raw request sent to a retained Python kernel's tool registry. */
export type PythonToolRequest =
	| { op: "describe"; names: string[] }
	| { op: "call"; name: string; args: Record<string, unknown> };

/** Session identity and bridge context for invoking a retained Python tool. */
export interface PythonToolInvokeOptions {
	cwd: string;
	sessionId: string;
	interpreter?: string;
	kernelOwnerId?: string;
	toolSession: ToolSession;
	signal?: AbortSignal;
}

export interface PythonExecutorOptions {
	/** Working directory for command execution */
	cwd?: string;
	/** Timeout in milliseconds */
	timeoutMs?: number;
	/** Absolute wall-clock deadline in milliseconds since epoch */
	deadlineMs?: number;
	/**
	 * Runtime-work budget (ms). Used only for timeout-annotation text when the
	 * caller drives cancellation via the eval watchdog `signal` instead of a
	 * wall-clock `deadlineMs`/`timeoutMs`. Does not arm a timer.
	 */
	idleTimeoutMs?: number;
	/** Callback for streaming output chunks (already sanitized) */
	onChunk?: (chunk: string) => Promise<void> | void;
	/** AbortSignal for cancellation */
	signal?: AbortSignal;
	/** Session identifier for kernel reuse */
	sessionId?: string;
	/** Logical owner identifier for retained kernel cleanup */
	kernelOwnerId?: string;
	/** Kernel mode (session reuse vs per-call) */
	kernelMode?: PythonKernelMode;
	/**
	 * Explicit interpreter path (`python.interpreter` resolved from the
	 * session's settings). Skips automatic runtime discovery when set.
	 */
	interpreter?: string;
	/** Restart the kernel before executing */
	reset?: boolean;
	/** Session file path for accessing task outputs */
	sessionFile?: string;
	/**
	 * Effective artifacts directory for the current session. Subagents share
	 * the parent's directory, so this can differ from `sessionFile`'s sibling
	 * dir. When present, exported to the kernel as `PI_ARTIFACTS_DIR` and
	 * preferred over `PI_SESSION_FILE`-derived paths.
	 */
	artifactsDir?: string;
	/** Artifact path/id for full output storage */
	artifactPath?: string;
	artifactId?: string;
	/**
	 * On-disk roots the prelude helpers (`read`/`write`) substitute for
	 * internal-URL schemes (e.g. `{ local: "/…/artifacts/local" }`). Exported to
	 * the kernel as `PI_EVAL_LOCAL_ROOTS` (JSON) so `write("local://x")` lands
	 * where `read local://x` resolves instead of a literal `local:/` directory.
	 */
	localRoots?: Record<string, string>;
	/**
	 * ToolSession used to resolve host-side `tool.<name>(args)` calls made from
	 * the Python prelude's bridge proxy. When omitted, the bridge env vars are
	 * not injected and any `tool.foo(...)` raises in Python.
	 */
	toolSession?: ToolSession;
	/** Callback for status events emitted by tool bridge invocations. */
	emitStatus?: (event: JsStatusEvent) => void;
	/**
	 * Live status events streamed as they are emitted (both host-side bridge
	 * helpers like `agent()` and kernel-side `display`/`log`/`phase`). Mirrors
	 * what lands in `displayOutputs` so callers can render progress before the
	 * cell finishes.
	 */
	onStatus?: (event: JsStatusEvent) => void;
	/** @internal Bridge session id, set by `executePython` before delegating. */
	bridgeSessionId?: string;
	/** @internal Bridge endpoint info, set by `executePython` before delegating. */
	bridge?: { url: string; token: string };
}

export interface PythonKernelExecutor {
	execute: (code: string, options?: KernelExecuteOptions) => Promise<KernelExecuteResult>;
	syncPreludes?: (
		preludes: readonly PythonPreludeSource[],
		signal: AbortSignal | undefined,
		timeoutMs: number,
	) => Promise<void>;
}

export interface PythonResult {
	/** Combined stdout + stderr output (sanitized, possibly truncated) */
	output: string;
	/** Execution exit code (0 ok, 1 error, undefined if cancelled) */
	exitCode: number | undefined;
	/** Whether the execution was cancelled via signal */
	cancelled: boolean;
	/** Whether the output was truncated */
	truncated: boolean;
	/** Artifact ID if full output was saved to artifact storage */
	artifactId?: string;
	/** Total number of lines in the output stream */
	totalLines: number;
	/** Total number of bytes in the output stream */
	totalBytes: number;
	/** Number of lines included in the output text */
	outputLines: number;
	/** Number of bytes included in the output text */
	outputBytes: number;
	/** Rich display outputs captured from display_data/execute_result */
	displayOutputs: KernelDisplayOutput[];
	/** Whether stdin was requested */
	stdinRequested: boolean;
}

export async function describePythonTools(
	_names: string[],
	_options: PythonToolInvokeOptions,
): Promise<{ tools: EvalToolDescriptor[]; missing: string[] }> {
	throw new Error(PYTHON_EVAL_RETIRED);
}
export async function callPythonTool(
	_name: string,
	_args: Record<string, unknown>,
	_options: PythonToolInvokeOptions,
): Promise<EvalToolInvokeResult> {
	throw new Error(PYTHON_EVAL_RETIRED);
}
export async function disposeAllKernelSessions(): Promise<void> {}
export async function disposeKernelSessionsByOwner(_ownerId: string): Promise<void> {}
export async function executePythonWithKernel(
	_kernel: PythonKernelExecutor,
	_code: string,
	_options?: PythonExecutorOptions,
): Promise<PythonResult> {
	throw new Error(PYTHON_EVAL_RETIRED);
}
export async function executePython(_code: string, _options?: PythonExecutorOptions): Promise<PythonResult> {
	throw new Error(PYTHON_EVAL_RETIRED);
}
