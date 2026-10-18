/** Transitional binding selection shared by terminal input consumers. */
import { TerminalInput } from "@harvest/pi-natives";

const candidate = typeof TerminalInput === "function" ? new TerminalInput() : null;
// Missing-addon installations expose proxy classes. Test method presence without
// executing a dummy function or interpreting valid empty results as unavailable.
export const nativeInput = candidate && "scan" in candidate ? candidate : null;
