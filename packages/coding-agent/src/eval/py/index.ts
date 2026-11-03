import type { ToolSession } from "../../tools";
import { type ExecutorBackend, type ExecutorBackendExecOptions, type ExecutorBackendResult } from "../backend";
import {
	namespaceSessionId as sharedNamespace,
	readInterpreterSetting as sharedReadInterpreterSetting,
} from "../backend-helpers";
import type { BackendProbeOptions } from "../probe";
import { defaultEvalSessionId } from "../session-id";
import { PYTHON_EVAL_RETIRED } from "./executor";
import { checkPythonKernelAvailability } from "./kernel";

const PYTHON_SESSION_PREFIX = "python:";

export function namespaceSessionId(sessionId: string): string {
	return sharedNamespace(sessionId, PYTHON_SESSION_PREFIX);
}

function readInterpreterSetting(session: ToolSession): string | undefined {
	return sharedReadInterpreterSetting(session, "python.interpreter");
}

/** Resolve the retained Python kernel identity owned by a tool session. */
export function resolvePythonKernelIdentity(session: ToolSession): {
	cwd: string;
	sessionId: string;
	interpreter: string | undefined;
	kernelOwnerId: string | undefined;
} {
	return {
		cwd: session.cwd,
		sessionId: namespaceSessionId(session.getEvalSessionId?.() ?? defaultEvalSessionId(session)),
		interpreter: readInterpreterSetting(session),
		kernelOwnerId: session.getEvalKernelOwnerId?.() ?? undefined,
	};
}

export default {
	id: "python",
	label: "Python",
	highlightLang: "python",

	async isAvailable(session: ToolSession, opts?: BackendProbeOptions): Promise<boolean> {
		const availability = await checkPythonKernelAvailability(session.cwd, readInterpreterSetting(session), opts);
		return availability.ok;
	},

	async execute(_code: string, _opts: ExecutorBackendExecOptions): Promise<ExecutorBackendResult> {
		throw new Error(PYTHON_EVAL_RETIRED);
	},
} satisfies ExecutorBackend;
