/** Legacy import boundary; no interpreter lookup, subprocess or kernel remains. */
import { PYTHON_EVAL_RETIRED } from "./executor";
export type { KernelDisplayOutput, PythonStatusEvent } from "./display";
export { renderKernelDisplay } from "./display";
export type { KernelExecuteOptions, KernelExecuteResult, KernelShutdownResult } from "../kernel-base";
export interface PythonPreludeSource {
	name: string;
	exports: string[];
	source: string;
}
export interface PythonKernelAvailability {
	ok: boolean;
	pythonPath?: string;
	reason?: string;
}
export async function checkPythonKernelAvailability(
	_cwd: string,
	_interpreter?: string,
	_options?: unknown,
): Promise<PythonKernelAvailability> {
	return { ok: false, reason: PYTHON_EVAL_RETIRED };
}
