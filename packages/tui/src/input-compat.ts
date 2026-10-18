/** Transitional scanner for installations without the portable Rust payload. */
const ESC = "\x1b";
const MAX_CSI_BYTES = 4096;
const MAX_STRING_SEQ_BYTES = 16 * 1024 * 1024;
const SGR_MOUSE_COMPLETE = /^<\d+;\d+;\d+[Mm]$/;

function resolveEscapeEnd(buffer: string, pos: number, length: number, resumeSearchFrom: number): number {
	if (pos + 1 >= length) return -1;
	const next = buffer.charCodeAt(pos + 1);

	switch (next) {
		case 0x1b /* ESC */:
			// Meta-ESC handled by the caller.
			return -1;
		case 0x5b /* [ */: {
			// CSI: ESC [ ... final byte in 0x40-0x7E.
			if (pos + 2 >= length) return -1;
			// Old-style X10 mouse: ESC [ M + 3 arbitrary bytes.
			if (buffer.charCodeAt(pos + 2) === 0x4d /* M */) {
				if (pos + 6 <= length) return pos + 6;
				// Fewer than 6 bytes buffered is always under MAX_CSI_BYTES,
				// so this is a plain "wait for more", never a cap flush.
				return -1;
			}
			const capEnd = Math.min(length, pos + MAX_CSI_BYTES);
			const isSgrMouse = buffer.charCodeAt(pos + 2) === 0x3c; /* < */
			// No resume hint for CSI: `extractCompleteSequences` records
			// hints only for OSC/DCS/APC. A partial CSI rescans from its
			// head, bounded by the tight MAX_CSI_BYTES cap.
			let i = pos + 2;
			while (i < capEnd) {
				const code = buffer.charCodeAt(i);
				if (code >= 0x40 && code <= 0x7e) {
					if (isSgrMouse) {
						// SGR mouse only terminates on M/m. Any other final
						// byte would be a malformed body — keep scanning to
						// match the prior `isCompleteCsiSequence` semantics.
						if (code !== 0x4d && code !== 0x6d) {
							i++;
							continue;
						}
						const payload = buffer.slice(pos + 2, i + 1);
						if (SGR_MOUSE_COMPLETE.test(payload)) return i + 1;
						// Malformed body ending in M/m — keep scanning for a
						// real terminator. Bounded by capEnd.
						i++;
						continue;
					}
					return i + 1;
				}
				i++;
			}
			return length - pos >= MAX_CSI_BYTES ? -2 : -1;
		}
		case 0x5d /* ] */: {
			// OSC: ESC ] ... BEL or ST (ESC \). Scan is bounded to
			// [searchFrom, scanLimit): `String#indexOf` has no end bound, so
			// an unterminated payload delivered as one huge chunk would
			// otherwise be scanned to the end of the buffer — past the cap
			// this function exists to enforce. `resumeSearchFrom - 1` keeps
			// the one-byte overlap so an `ESC \` split across chunks is
			// still found (the prior call's trailing ESC is re-inspected).
			const searchFrom = Math.max(pos + 2, resumeSearchFrom - 1);
			const scanLimit = Math.min(length, pos + MAX_STRING_SEQ_BYTES);
			for (let i = searchFrom; i < scanLimit; i++) {
				const code = buffer.charCodeAt(i);
				if (code === 0x07 /* BEL */) return i + 1;
				if (code === 0x1b /* ESC */) {
					// `ESC \` (ST) must end within the cap; a lone trailing
					// ESC at the buffer edge stays incomplete and is
					// re-examined next call via the resume overlap.
					if (i + 1 < scanLimit && buffer.charCodeAt(i + 1) === 0x5c /* \ */) return i + 2;
				}
			}
			return length - pos >= MAX_STRING_SEQ_BYTES ? -2 : -1;
		}
		case 0x50 /* P */:
		case 0x5f /* _ */: {
			// DCS / APC: ESC P/_ ... ST (ESC \). Same bounded scan and
			// split-ST overlap as the OSC branch, minus BEL.
			const searchFrom = Math.max(pos + 2, resumeSearchFrom - 1);
			const scanLimit = Math.min(length, pos + MAX_STRING_SEQ_BYTES);
			for (let i = searchFrom; i < scanLimit; i++) {
				if (
					buffer.charCodeAt(i) === 0x1b /* ESC */ &&
					i + 1 < scanLimit &&
					buffer.charCodeAt(i + 1) === 0x5c /* \ */
				) {
					return i + 2;
				}
			}
			return length - pos >= MAX_STRING_SEQ_BYTES ? -2 : -1;
		}
		case 0x4f /* O */:
			// SS3: ESC O + 1 char.
			return pos + 3 <= length ? pos + 3 : -1;
		default:
			// Meta chord: ESC + 1 char.
			return pos + 2;
	}
}

export function extractCompleteSequences(
	buffer: string,
	resumeSearchFrom: number,
): { sequences: string[]; remainder: string; resumeSearchFrom: number; discardFrom?: number } {
	const sequences: string[] = [];
	const length = buffer.length;
	let pos = 0;

	// Index-based scanning: this is the input hot path. Slicing the remaining
	// buffer (or Array.from-ing it) per iteration would make plain-text bursts
	// O(n²) — a 100KB non-bracketed paste must stay O(n).
	//
	// `resumeSearchFrom` applies only when the buffer starts with an
	// incomplete OSC/DCS/APC we buffered on the previous call; once any
	// bytes are consumed (pos advances past the leading escape), the hint no
	// longer maps to the current buffer offsets and is discarded.
	let hint = resumeSearchFrom;

	while (pos < length) {
		if (buffer.charCodeAt(pos) !== 0x1b) {
			// Not an escape sequence - take one Unicode scalar, not a UTF-16 code unit.
			const codePoint = buffer.codePointAt(pos)!;
			const charLength = codePoint > 0xffff ? 2 : 1;
			sequences.push(buffer.slice(pos, pos + charLength));
			pos += charLength;
			hint = 0;
			continue;
		}

		// `\x1b\x1b` is one of three things — see the outer switch below.
		// Kept in the outer loop because it interacts with flush timing
		// (bare `\x1b\x1b` is held for the timer chain) and with the SGR
		// mouse split that splits `\x1b\x1b[<…` into `\x1b` + `\x1b[<…`.
		if (pos + 1 < length && buffer.charCodeAt(pos + 1) === 0x1b) {
			if (pos + 2 >= length) {
				//   Two real Esc keypresses bursted by terminal input batching:
				//   when the buffer ends here, hold the partial for the flush
				//   window so cases 1/2 can still arrive; if no follower
				//   arrives, `flush()` splits the held remainder into two ESC
				//   events (#3857).
				return { sequences, remainder: buffer.slice(pos), resumeSearchFrom: 0 };
			}
			const third = buffer.charCodeAt(pos + 2);
			if (third !== 0x5b && third !== 0x4f) {
				//   ESC followed by a legacy Alt chord (`\x1bd`, `\x1b\x7f`, …):
				//   emit the first ESC, then restart at the second ESC so
				//   downstream parsing still sees the Alt chord as one
				//   keypress (#3860 review).
				sequences.push(ESC);
				pos += 1;
				hint = 0;
				continue;
			}
			//   ESC prefixing CSI/SS3 (meta-CSI, held Esc joined by a follower):
			//   resolve the inner escape's end from `pos + 1`. Consuming two
			//   bytes here would tear the follower and leak its tail as typed
			//   text (settings search filling with "[B" or "[<35;22;17M").
			const innerEnd = resolveEscapeEnd(buffer, pos + 1, length, 0);
			if (innerEnd === -1) {
				return { sequences, remainder: buffer.slice(pos), resumeSearchFrom: 0 };
			}
			if (innerEnd === -2) {
				// `third` is CSI/SS3 here, never a string type.
				const flushEnd = Math.min(length, pos + MAX_CSI_BYTES);
				sequences.push(buffer.slice(pos, flushEnd));
				pos = flushEnd;
				hint = 0;
				continue;
			}
			// ESC + SGR mouse is never a meta chord: alt-modified mouse
			// reports carry the modifier in the button bits, not an ESC
			// prefix. Deliver the bare ESC and the report separately.
			if (third === 0x5b && buffer.charCodeAt(pos + 3) === 0x3c) {
				sequences.push(ESC);
				sequences.push(buffer.slice(pos + 1, innerEnd));
				pos = innerEnd;
				hint = 0;
				continue;
			}
			sequences.push(buffer.slice(pos, innerEnd));
			pos = innerEnd;
			hint = 0;
			continue;
		}

		// Single ESC — resolve directly. Hint carries over from the previous
		// call only when we are still on the buffered escape (pos === 0).
		const end = resolveEscapeEnd(buffer, pos, length, pos === 0 ? hint : 0);
		if (end === -1) {
			// Buffer for more. When this is the leading OSC/DCS/APC,
			// remember how far we scanned so the next `process()` call
			// resumes from there instead of rescanning the whole buffer.
			const next = pos + 1 < length ? buffer.charCodeAt(pos + 1) : -1;
			const nextHint = pos === 0 && (next === 0x5d || next === 0x50 || next === 0x5f) ? length : 0;
			return { sequences, remainder: buffer.slice(pos), resumeSearchFrom: nextHint };
		}
		if (end === -2) {
			const next = buffer.charCodeAt(pos + 1);
			if (next === 0x5d || next === 0x50 || next === 0x5f) {
				// Unterminated string sequence at the cap (issue #4073 case A):
				// the head is junk and everything after it is mid-string
				// payload. Hand both to the caller's discard mode instead of
				// flushing the payload as per-character typed text.
				return { sequences, remainder: "", resumeSearchFrom: 0, discardFrom: pos };
			}
			const flushEnd = Math.min(length, pos + MAX_CSI_BYTES);
			sequences.push(buffer.slice(pos, flushEnd));
			pos = flushEnd;
			hint = 0;
			continue;
		}
		sequences.push(buffer.slice(pos, end));
		pos = end;
		hint = 0;
	}

	return { sequences, remainder: "", resumeSearchFrom: 0 };
}
