/**
 * Pure JavaScript fallback implementation of terminal keyboard matching and parsing.
 *
 * Implements the exact same contracts and behavior as crates/pi-natives/src/keys.rs
 * so that interactive TUI keyboard input works flawlessly on platforms or environments
 * where the native .node binary is unavailable.
 */

const LOCK_MASK = 64 + 128; // capsLock (64) + numLock (128)

// Modifiers bitmask
export const MOD_SHIFT = 1;
export const MOD_ALT = 2;
export const MOD_CTRL = 4;
export const MOD_SUPER = 8;
export const MOD_NUM_LOCK = 128;

// Internal sentinel codes for functional/arrow keys
const ARROW_UP = -1;
const ARROW_DOWN = -2;
const ARROW_RIGHT = -3;
const ARROW_LEFT = -4;

const FUNC_DELETE = -10;
const FUNC_INSERT = -11;
const FUNC_PAGE_UP = -12;
const FUNC_PAGE_DOWN = -13;
const FUNC_HOME = -14;
const FUNC_END = -15;
const FUNC_CLEAR = -16;

const FUNC_F1 = -20;
const FUNC_F2 = -21;
const FUNC_F3 = -22;
const FUNC_F4 = -23;
const FUNC_F5 = -24;
const FUNC_F6 = -25;
const FUNC_F7 = -26;
const FUNC_F8 = -27;
const FUNC_F9 = -28;
const FUNC_F10 = -29;
const FUNC_F11 = -30;
const FUNC_F12 = -31;

const CP_ESCAPE = 27;
const CP_TAB = 9;
const CP_ENTER = 13;
const CP_SPACE = 32;
const CP_BACKSPACE = 127;

const CP_KP_0 = 57399;
const CP_KP_1 = 57400;
const CP_KP_2 = 57401;
const CP_KP_3 = 57402;
const CP_KP_4 = 57403;
const CP_KP_5 = 57404;
const CP_KP_6 = 57405;
const CP_KP_7 = 57406;
const CP_KP_8 = 57407;
const CP_KP_9 = 57408;
const CP_KP_DECIMAL = 57409;
const CP_KP_DIVIDE = 57410;
const CP_KP_MULTIPLY = 57411;
const CP_KP_SUBTRACT = 57412;
const CP_KP_ADD = 57413;
const CP_KP_ENTER = 57414;
const CP_KP_EQUALS = 57415;

const LEGACY_SEQUENCES = {
	// Arrow keys (SS3 and CSI)
	"\x1bOA": "up", "\x1bOB": "down", "\x1bOC": "right", "\x1bOD": "left",
	"\x1b[A": "up", "\x1b[B": "down", "\x1b[C": "right", "\x1b[D": "left",
	// Home/End (multiple terminal variants)
	"\x1bOH": "home", "\x1bOF": "end",
	"\x1b[H": "home", "\x1b[F": "end",
	"\x1b[1~": "home", "\x1b[7~": "home",
	"\x1b[4~": "end", "\x1b[8~": "end",
	// Clear
	"\x1b[E": "clear", "\x1bOE": "clear", "\x1bOe": "ctrl+clear", "\x1b[e": "shift+clear",
	// Insert/Delete
	"\x1b[2~": "insert", "\x1b[2$": "shift+insert", "\x1b[2^": "ctrl+insert",
	"\x1b[3~": "delete", "\x1b[3$": "shift+delete", "\x1b[3^": "ctrl+delete",
	// Page Up/Down
	"\x1b[5~": "pageUp", "\x1b[6~": "pageDown",
	"\x1b[[5~": "pageUp", "\x1b[[6~": "pageDown",
	// Shift+arrow
	"\x1b[a": "shift+up", "\x1b[b": "shift+down", "\x1b[c": "shift+right", "\x1b[d": "shift+left",
	// Ctrl+arrow
	"\x1bOa": "ctrl+up", "\x1bOb": "ctrl+down", "\x1bOc": "ctrl+right", "\x1bOd": "ctrl+left",
	// Shift+page/home/end
	"\x1b[5$": "shift+pageUp", "\x1b[6$": "shift+pageDown",
	"\x1b[7$": "shift+home", "\x1b[8$": "shift+end",
	// Ctrl+page/home/end
	"\x1b[5^": "ctrl+pageUp", "\x1b[6^": "ctrl+pageDown",
	"\x1b[7^": "ctrl+home", "\x1b[8^": "ctrl+end",
	// Function keys (SS3, CSI tilde, Linux console)
	"\x1bOP": "f1", "\x1bOQ": "f2", "\x1bOR": "f3", "\x1bOS": "f4",
	"\x1b[11~": "f1", "\x1b[12~": "f2", "\x1b[13~": "f3", "\x1b[14~": "f4",
	"\x1b[[A": "f1", "\x1b[[B": "f2", "\x1b[[C": "f3", "\x1b[[D": "f4", "\x1b[[E": "f5",
	"\x1b[15~": "f5", "\x1b[17~": "f6", "\x1b[18~": "f7", "\x1b[19~": "f8",
	"\x1b[20~": "f9", "\x1b[21~": "f10", "\x1b[23~": "f11", "\x1b[24~": "f12",
};

function mapKeypadNav(codepoint) {
	switch (codepoint) {
		case CP_KP_0: return FUNC_INSERT;
		case CP_KP_1: return FUNC_END;
		case CP_KP_2: return ARROW_DOWN;
		case CP_KP_3: return FUNC_PAGE_DOWN;
		case CP_KP_4: return ARROW_LEFT;
		case CP_KP_5: return FUNC_CLEAR;
		case CP_KP_6: return ARROW_RIGHT;
		case CP_KP_7: return FUNC_HOME;
		case CP_KP_8: return ARROW_UP;
		case CP_KP_9: return FUNC_PAGE_UP;
		case CP_KP_DECIMAL: return FUNC_DELETE;
		default: return null;
	}
}

function keypadNumLockTextCodepoint(codepoint) {
	if (codepoint >= CP_KP_0 && codepoint <= CP_KP_9) {
		return codepoint - CP_KP_0 + 48;
	}
	if (codepoint === CP_KP_DECIMAL) return 46;
	return null;
}

function keypadOperatorTextCodepoint(codepoint) {
	switch (codepoint) {
		case CP_KP_DIVIDE: return 47; // '/'
		case CP_KP_MULTIPLY: return 42; // '*'
		case CP_KP_SUBTRACT: return 45; // '-'
		case CP_KP_ADD: return 43; // '+'
		case CP_KP_EQUALS: return 61; // '='
		default: return null;
	}
}

const SYMBOL_CODEPOINTS = new Set([
	96, 34, 45, 61, 91, 93, 92, 59, 39, 44, 46, 47, 33, 64, 35, 36, 37,
	94, 38, 42, 40, 41, 95, 43, 124, 126, 123, 125, 58, 60, 62, 63
]);

function isSymbolKey(cp) {
	return SYMBOL_CODEPOINTS.has(cp);
}

function rawCtrlChar(letter) {
	return letter.toLowerCase().charCodeAt(0) - 96;
}

function isNamedKeyLegacyByte(b) {
	return b === 0x08 || b === 0x09 || b === 0x0a || b === 0x0d || b === 0x1b || b === 0x7f;
}

function ctrlSymbolToByte(ch) {
	const code = ch.charCodeAt(0);
	if (code === 64 || (code >= 91 && code <= 95)) {
		return code - 0x40;
	}
	if (code === 45) return 0x1f;
	return null;
}

function parseModifyOtherKeys(data) {
	if (!data.startsWith("\x1b[27;")) return null;
	const match = data.match(/^\x1b\[27;(\d+);(\d+)~?$/);
	if (!match) return null;
	const modValue = parseInt(match[1], 10);
	const keycode = parseInt(match[2], 10);
	if (modValue === 0) return null;
	return { modifier: modValue - 1, keycode };
}

/**
 * Parse a Kitty keyboard protocol sequence.
 * @param {string} data
 * @returns {import("./index.d.ts").ParsedKittyResult | null}
 */
export function parseKittySequence(data) {
	if (!data || data.length < 4 || data.charCodeAt(0) !== 0x1b || data.charCodeAt(1) !== 0x5b) {
		return null;
	}

	const lastChar = data[data.length - 1];
	if (lastChar === "u") {
		return parseCsiU(data);
	}
	if (lastChar === "~") {
		return parseFunctional(data);
	}
	if (/^[ABCDEFHPQRS]$/.test(lastChar)) {
		return parseCsi1Letter(data);
	}
	return null;
}

function parseCsiU(data) {
	// Format: \x1b[<codepoint>(:<shifted>(:<base>)?)?(;<mod>(:<event>)?)?(;<text>)?u
	const match = data.match(/^\x1b\[(\d+)(?::(\d*))?(?::(\d+))?(?:;(\d*))?(?::(\d+))?(?:;([^u]*))?u$/);
	if (!match) return null;

	const codepoint = parseInt(match[1], 10);
	const shiftedKey = match[2] && match[2].length > 0 ? parseInt(match[2], 10) : undefined;
	const baseLayoutKey = match[3] ? parseInt(match[3], 10) : undefined;
	const modValue = match[4] && match[4].length > 0 ? parseInt(match[4], 10) : 1;
	const eventType = match[5] ? parseInt(match[5], 10) : undefined;

	let textCodepoint = undefined;
	if (match[6] && match[6].length > 0) {
		const textParts = match[6].split(":").filter(Boolean);
		if (textParts.length === 1) {
			const cp = parseInt(textParts[0], 10);
			if (cp >= 32) {
				textCodepoint = cp;
			}
		}
	}

	if (modValue === 0) return null;

	return {
		codepoint,
		shiftedKey,
		baseLayoutKey,
		textCodepoint,
		modifier: modValue - 1,
		eventType,
	};
}

function parseCsi1Letter(data) {
	if (!data.startsWith("\x1b[1;")) return null;
	const match = data.match(/^\x1b\[1;(\d+)(?::(\d+))?([ABCDEFHPQRS])$/);
	if (!match) return null;

	const modValue = parseInt(match[1], 10);
	const eventType = match[2] ? parseInt(match[2], 10) : undefined;
	const letter = match[3];

	if (modValue === 0) return null;

	let codepoint;
	switch (letter) {
		case "A": codepoint = ARROW_UP; break;
		case "B": codepoint = ARROW_DOWN; break;
		case "C": codepoint = ARROW_RIGHT; break;
		case "D": codepoint = ARROW_LEFT; break;
		case "H": codepoint = FUNC_HOME; break;
		case "F": codepoint = FUNC_END; break;
		case "E": codepoint = FUNC_CLEAR; break;
		case "P": codepoint = FUNC_F1; break;
		case "Q": codepoint = FUNC_F2; break;
		case "R": codepoint = FUNC_F3; break;
		case "S": codepoint = FUNC_F4; break;
		default: return null;
	}

	return {
		codepoint,
		modifier: modValue - 1,
		eventType,
	};
}

function parseFunctional(data) {
	const match = data.match(/^\x1b\[(\d+)(?:;(\d+))?(?::(\d+))?~$/);
	if (!match) return null;

	const keyNum = parseInt(match[1], 10);
	const modValue = match[2] ? parseInt(match[2], 10) : 1;
	const eventType = match[3] ? parseInt(match[3], 10) : undefined;

	if (modValue === 0) return null;

	let codepoint;
	switch (keyNum) {
		case 2: codepoint = FUNC_INSERT; break;
		case 3: codepoint = FUNC_DELETE; break;
		case 5: codepoint = FUNC_PAGE_UP; break;
		case 6: codepoint = FUNC_PAGE_DOWN; break;
		case 1:
		case 7: codepoint = FUNC_HOME; break;
		case 4:
		case 8: codepoint = FUNC_END; break;
		case 11: codepoint = FUNC_F1; break;
		case 12: codepoint = FUNC_F2; break;
		case 13: codepoint = FUNC_F3; break;
		case 14: codepoint = FUNC_F4; break;
		case 15: codepoint = FUNC_F5; break;
		case 17: codepoint = FUNC_F6; break;
		case 18: codepoint = FUNC_F7; break;
		case 19: codepoint = FUNC_F8; break;
		case 20: codepoint = FUNC_F9; break;
		case 21: codepoint = FUNC_F10; break;
		case 23: codepoint = FUNC_F11; break;
		case 24: codepoint = FUNC_F12; break;
		default: return null;
	}

	return {
		codepoint,
		modifier: modValue - 1,
		eventType,
	};
}

/**
 * Match Kitty protocol input against a codepoint and modifier mask.
 * @param {string} data
 * @param {number} expectedCodepoint
 * @param {number} expectedModifier
 * @returns {boolean}
 */
export function matchesKittySequence(data, expectedCodepoint, expectedModifier) {
	const parsed = parseKittySequence(data);
	if (!parsed) return false;

	const actualMod = parsed.modifier & ~LOCK_MASK;
	const expectedMod = expectedModifier & ~LOCK_MASK;
	if (actualMod !== expectedMod) return false;

	if (parsed.codepoint === expectedCodepoint) return true;

	if (actualMod !== 0 && parsed.baseLayoutKey != null && parsed.baseLayoutKey === expectedCodepoint) {
		const cp = parsed.codepoint;
		const isAsciiLetter = (cp >= 65 && cp <= 90) || (cp >= 97 && cp <= 122);
		const isKnownSymbol = isSymbolKey(cp);
		if (!isAsciiLetter && !isKnownSymbol) return true;
	}

	return false;
}

/**
 * Check if input matches a legacy escape sequence for the given key name.
 * @param {string} data
 * @param {string} keyName
 * @returns {boolean}
 */
export function matchesLegacySequence(data, keyName) {
	return LEGACY_SEQUENCES[data] === keyName;
}

function formatKeyName(codepoint) {
	switch (codepoint) {
		case CP_ESCAPE: return "escape";
		case CP_TAB: return "tab";
		case CP_ENTER:
		case CP_KP_ENTER: return "enter";
		case CP_SPACE: return "space";
		case CP_BACKSPACE: return "backspace";
		case CP_KP_0: return "insert";
		case CP_KP_1: return "end";
		case CP_KP_2: return "down";
		case CP_KP_3: return "pageDown";
		case CP_KP_4: return "left";
		case CP_KP_5: return "clear";
		case CP_KP_6: return "right";
		case CP_KP_7: return "home";
		case CP_KP_8: return "up";
		case CP_KP_9: return "pageUp";
		case CP_KP_DECIMAL: return "delete";

		case FUNC_DELETE: return "delete";
		case FUNC_INSERT: return "insert";
		case FUNC_HOME: return "home";
		case FUNC_END: return "end";
		case FUNC_PAGE_UP: return "pageUp";
		case FUNC_PAGE_DOWN: return "pageDown";
		case FUNC_CLEAR: return "clear";

		case ARROW_UP: return "up";
		case ARROW_DOWN: return "down";
		case ARROW_LEFT: return "left";
		case ARROW_RIGHT: return "right";

		case FUNC_F1: return "f1";
		case FUNC_F2: return "f2";
		case FUNC_F3: return "f3";
		case FUNC_F4: return "f4";
		case FUNC_F5: return "f5";
		case FUNC_F6: return "f6";
		case FUNC_F7: return "f7";
		case FUNC_F8: return "f8";
		case FUNC_F9: return "f9";
		case FUNC_F10: return "f10";
		case FUNC_F11: return "f11";
		case FUNC_F12: return "f12";

		default:
			if (codepoint >= 33 && codepoint <= 126) {
				return String.fromCharCode(codepoint);
			}
			return null;
	}
}

function formatWithMods(mods, keyName) {
	let result = "";
	if (mods & MOD_SHIFT) result += "shift+";
	if (mods & MOD_CTRL) result += "ctrl+";
	if (mods & MOD_ALT) result += "alt+";
	if (mods & MOD_SUPER) result += "super+";
	result += keyName;
	return result;
}

function formatKittyKey(parsed) {
	const effectiveMod = parsed.modifier & ~LOCK_MASK;
	if ((effectiveMod & ~(MOD_SHIFT | MOD_CTRL | MOD_ALT | MOD_SUPER)) !== 0) {
		return null;
	}

	let effectiveCodepoint;
	const opText = keypadOperatorTextCodepoint(parsed.codepoint);
	if (opText !== null) {
		effectiveCodepoint = opText;
	} else {
		const cp = parsed.codepoint;
		const isAsciiLetter = (cp >= 65 && cp <= 90) || (cp >= 97 && cp <= 122);
		const isKnownSymbol = isSymbolKey(cp);
		if (effectiveMod === 0 || isAsciiLetter || isKnownSymbol) {
			effectiveCodepoint = cp;
		} else {
			effectiveCodepoint = parsed.baseLayoutKey ?? cp;
		}
	}

	if (effectiveMod === 0) {
		if (parsed.textCodepoint != null) {
			const keyName = formatKeyName(parsed.textCodepoint);
			if (keyName != null) return keyName;
		}
		const numLockText = keypadNumLockTextCodepoint(parsed.codepoint);
		if (numLockText != null) {
			const keyName = formatKeyName(numLockText);
			if (keyName != null) return keyName;
		}
		return formatKeyName(effectiveCodepoint);
	}

	const keyName = formatKeyName(effectiveCodepoint);
	if (!keyName) return null;
	return formatWithMods(effectiveMod, keyName);
}

function parseSingleByte(code) {
	switch (code) {
		case 0x1b: return "escape";
		case 0x09: return "tab";
		case 0x0d:
		case 0x0a: return "enter";
		case 0x00: return "ctrl+space";
		case 0x20: return "space";
		case 0x7f:
		case 0x08: return "backspace";
		case 28: return "ctrl+\\";
		case 29: return "ctrl+]";
		case 30: return "ctrl+^";
		case 31: return "ctrl+_";
		default:
			if (code >= 1 && code <= 26) {
				return `ctrl+${String.fromCharCode(code + 96)}`;
			}
			if (code >= 33 && code <= 126) {
				return String.fromCharCode(code);
			}
			return null;
	}
}

function parseEscPair(code, kittyProtocolActive) {
	switch (code) {
		case 0x7f:
		case 0x08: return "alt+backspace";
		case 0x0d:
		case 0x0a: return "alt+enter";
		case 0x09: return "alt+tab";
		default: break;
	}

	if (!kittyProtocolActive) {
		switch (code) {
			case 0x20: return "alt+space";
			case 0x42: return "alt+left"; // 'B'
			case 0x46: return "alt+right"; // 'F'
			default: break;
		}
	}

	if (code >= 1 && code <= 26) {
		return `ctrl+alt+${String.fromCharCode(code + 96)}`;
	}
	if (code >= 97 && code <= 122) { // 'a'..'z'
		return `alt+${String.fromCharCode(code)}`;
	}
	if (code >= 65 && code <= 90) { // 'A'..'Z'
		return `alt+shift+${String.fromCharCode(code + 32)}`;
	}
	return null;
}

/**
 * Parse terminal input and return a normalized key identifier.
 * @param {string} data
 * @param {boolean} [kittyProtocolActive=false]
 * @returns {string | null}
 */
export function parseKey(data, kittyProtocolActive = false) {
	if (!data || data.length === 0) return null;

	if (data.length === 1) {
		return parseSingleByte(data.charCodeAt(0));
	}

	if (data.charCodeAt(0) !== 0x1b) {
		return null;
	}

	if (data.length === 2) {
		const key = parseEscPair(data.charCodeAt(1), kittyProtocolActive);
		if (key != null) return key;
	}

	const legacy = LEGACY_SEQUENCES[data];
	if (legacy) return legacy;

	const mok = parseModifyOtherKeys(data);
	if (mok) {
		const keyName = formatKeyName(mok.keycode);
		if (keyName) {
			if (mok.modifier === 0) return keyName;
			return formatWithMods(mok.modifier & ~LOCK_MASK, keyName);
		}
	}

	const kitty = parseKittySequence(data);
	if (kitty) {
		if (kitty.eventType === 3) return null; // release event
		return formatKittyKey(kitty);
	}

	if (data.length > 2 && data[0] === "\x1b" && data[1] === "\x1b" && (data[2] === "[" || data[2] === "O")) {
		const innerKey = parseKey(data.slice(1), true);
		if (innerKey) return `alt+${innerKey}`;
	}

	if (data === "\x1b[Z") return "shift+tab";
	if (data === "\x1bOM") return "enter";

	return null;
}

const PARSED_KEY_ID_CACHE = new Map();

function parseKeyId(keyId) {
	const s = keyId.toLowerCase();
	const cached = PARSED_KEY_ID_CACHE.get(s);
	if (cached) return cached;

	const forcedKeyPlus = s === "+" || s.endsWith("++");
	const prefix = s === "+" ? "" : (s.endsWith("++") ? s.slice(0, -2) : s);

	let modifier = 0;
	let key = forcedKeyPlus ? "+" : null;

	for (const part of prefix.split("+")) {
		const p = part.trim();
		if (!p) continue;
		if (p === "ctrl") {
			modifier |= MOD_CTRL;
			continue;
		}
		if (p === "shift") {
			modifier |= MOD_SHIFT;
			continue;
		}
		if (p === "super") {
			modifier |= MOD_SUPER;
			continue;
		}
		if (p === "alt") {
			modifier |= MOD_ALT;
			continue;
		}
		key = p;
	}

	if (!key) return null;
	if (key === "plus") key = "+";
	else if (key === "esc") key = "escape";

	const result = { key, modifier };
	PARSED_KEY_ID_CACHE.set(s, result);
	return result;
}

function matchesLegacyKey(data, key) {
	return LEGACY_SEQUENCES[data] === key;
}

function matchesLegacyModifierSequence(data, key, modifier) {
	let expected = null;
	if (modifier === MOD_SHIFT) {
		switch (key) {
			case "up": expected = "shift+up"; break;
			case "down": expected = "shift+down"; break;
			case "right": expected = "shift+right"; break;
			case "left": expected = "shift+left"; break;
			case "clear": expected = "shift+clear"; break;
			case "insert": expected = "shift+insert"; break;
			case "delete": expected = "shift+delete"; break;
			case "pageUp": expected = "shift+pageUp"; break;
			case "pageDown": expected = "shift+pageDown"; break;
			case "home": expected = "shift+home"; break;
			case "end": expected = "shift+end"; break;
			default: break;
		}
	} else if (modifier === MOD_CTRL) {
		switch (key) {
			case "up": expected = "ctrl+up"; break;
			case "down": expected = "ctrl+down"; break;
			case "right": expected = "ctrl+right"; break;
			case "left": expected = "ctrl+left"; break;
			case "clear": expected = "ctrl+clear"; break;
			case "insert": expected = "ctrl+insert"; break;
			case "delete": expected = "ctrl+delete"; break;
			case "pageUp": expected = "ctrl+pageUp"; break;
			case "pageDown": expected = "ctrl+pageDown"; break;
			case "home": expected = "ctrl+home"; break;
			case "end": expected = "ctrl+end"; break;
			default: break;
		}
	}
	return expected != null && LEGACY_SEQUENCES[data] === expected;
}

/**
 * Match input data against a key identifier string.
 * @param {string} data
 * @param {string} keyId
 * @param {boolean} [kittyProtocolActive=false]
 * @returns {boolean}
 */
export function matchesKey(data, keyId, kittyProtocolActive = false) {
	if (!data || !keyId) return false;

	const parsed = parseKeyId(keyId);
	if (!parsed) return false;

	const { key, modifier } = parsed;

	// ESC-prefixed sequences (\x1b\x1b[...] = Alt + inner-key)
	if (
		(modifier & MOD_ALT) !== 0 &&
		data.length > 2 &&
		data.charCodeAt(0) === 0x1b &&
		data.charCodeAt(1) === 0x1b &&
		(data.charCodeAt(2) === 0x5b || data.charCodeAt(2) === 0x4f)
	) {
		const innerModifier = modifier & ~MOD_ALT;
		let innerKeyId = "";
		if (innerModifier & MOD_SHIFT) innerKeyId += "shift+";
		if (innerModifier & MOD_CTRL) innerKeyId += "ctrl+";
		if (innerModifier & MOD_SUPER) innerKeyId += "super+";
		innerKeyId += key;
		return matchesKey(data.slice(1), innerKeyId, true);
	}

	const kittyParsed = parseKittySequence(data);
	const kittyMatches = (codepoint, m) => {
		if (!kittyParsed) return false;
		if (kittyParsed.eventType === 3) return false; // Ignore release events
		const actualMod = kittyParsed.modifier & ~LOCK_MASK;
		const expectedMod = m & ~LOCK_MASK;
		if (actualMod !== expectedMod) return false;

		let parsedCodepoint = kittyParsed.codepoint;
		let parsedBase = kittyParsed.baseLayoutKey;

		if (kittyParsed.textCodepoint == null) {
			const opText = keypadOperatorTextCodepoint(parsedCodepoint);
			if (opText !== null) {
				parsedCodepoint = opText;
				parsedBase = undefined;
			} else if (actualMod === 0) {
				const numLockText = keypadNumLockTextCodepoint(parsedCodepoint);
				if (numLockText !== null) {
					parsedCodepoint = numLockText;
					parsedBase = undefined;
				} else if ((kittyParsed.modifier & MOD_NUM_LOCK) !== 0) {
					const mapped = mapKeypadNav(parsedCodepoint);
					if (mapped !== null) parsedCodepoint = mapped;
					if (parsedBase != null) {
						const mappedBase = mapKeypadNav(parsedBase);
						if (mappedBase !== null) parsedBase = mappedBase;
					}
				}
			} else {
				const mapped = mapKeypadNav(parsedCodepoint);
				if (mapped !== null) parsedCodepoint = mapped;
				if (parsedBase != null) {
					const mappedBase = mapKeypadNav(parsedBase);
					if (mappedBase !== null) parsedBase = mappedBase;
				}
			}
		}

		if (parsedCodepoint === codepoint) return true;

		if (actualMod !== 0 && parsedBase != null && parsedBase === codepoint) {
			const isAsciiLetter = (parsedCodepoint >= 65 && parsedCodepoint <= 90) || (parsedCodepoint >= 97 && parsedCodepoint <= 122);
			const isKnownSymbol = isSymbolKey(parsedCodepoint);
			if (!isAsciiLetter && !isKnownSymbol) return true;
		}

		return false;
	};

	const mok = parseModifyOtherKeys(data);
	const mokMatches = (keycode, m) => mok !== null && mok.keycode === keycode && mok.modifier === m;

	// Named keys
	if (key === "escape") {
		if (modifier !== 0) return false;
		return data === "\x1b" || kittyMatches(CP_ESCAPE, 0);
	}

	if (key === "space") {
		if (modifier === MOD_CTRL && data === "\x00") return true;
		if (modifier === MOD_ALT && !kittyProtocolActive && data === "\x1b ") return true;
		if (modifier === 0) return data === " " || kittyMatches(CP_SPACE, 0);
		return kittyMatches(CP_SPACE, modifier) || mokMatches(CP_SPACE, modifier);
	}

	if (key === "tab") {
		if (modifier === MOD_SHIFT) {
			return data === "\x1b[Z" || kittyMatches(CP_TAB, MOD_SHIFT) || mokMatches(CP_TAB, MOD_SHIFT);
		}
		if (modifier === MOD_ALT && data === "\x1b\t") return true;
		if (modifier === 0) return data === "\t" || kittyMatches(CP_TAB, 0);
		return kittyMatches(CP_TAB, modifier) || mokMatches(CP_TAB, modifier);
	}

	if (key === "enter" || key === "return") {
		if (modifier === MOD_ALT && (data === "\x1b\r" || data === "\x1b\n")) return true;
		if (modifier === 0) {
			return data === "\r" || data === "\n" || data === "\x1bOM" ||
				kittyMatches(CP_ENTER, 0) || kittyMatches(CP_KP_ENTER, 0);
		}
		return kittyMatches(CP_ENTER, modifier) || kittyMatches(CP_KP_ENTER, modifier) ||
			mokMatches(CP_ENTER, modifier) || mokMatches(CP_KP_ENTER, modifier);
	}

	if (key === "backspace") {
		if (modifier === MOD_ALT) {
			return data === "\x1b\x7f" || data === "\x1b\x08" ||
				kittyMatches(CP_BACKSPACE, MOD_ALT) || mokMatches(CP_BACKSPACE, MOD_ALT);
		}
		if (modifier === 0) {
			return data === "\x7f" || data === "\x08" || kittyMatches(CP_BACKSPACE, 0);
		}
		return kittyMatches(CP_BACKSPACE, modifier) || mokMatches(CP_BACKSPACE, modifier);
	}

	if (key === "insert") {
		if (modifier === 0) return matchesLegacyKey(data, "insert") || kittyMatches(FUNC_INSERT, 0);
		return matchesLegacyModifierSequence(data, "insert", modifier) || kittyMatches(FUNC_INSERT, modifier);
	}

	if (key === "delete") {
		if (modifier === 0) return matchesLegacyKey(data, "delete") || kittyMatches(FUNC_DELETE, 0);
		return matchesLegacyModifierSequence(data, "delete", modifier) || kittyMatches(FUNC_DELETE, modifier);
	}

	if (key === "clear") {
		if (modifier === 0) return matchesLegacyKey(data, "clear") || kittyMatches(FUNC_CLEAR, 0);
		return matchesLegacyModifierSequence(data, "clear", modifier) || kittyMatches(FUNC_CLEAR, modifier);
	}

	if (key === "home") {
		if (modifier === 0) return matchesLegacyKey(data, "home") || kittyMatches(FUNC_HOME, 0);
		return matchesLegacyModifierSequence(data, "home", modifier) || kittyMatches(FUNC_HOME, modifier);
	}

	if (key === "end") {
		if (modifier === 0) return matchesLegacyKey(data, "end") || kittyMatches(FUNC_END, 0);
		return matchesLegacyModifierSequence(data, "end", modifier) || kittyMatches(FUNC_END, modifier);
	}

	if (key === "pageup") {
		if (modifier === 0) return matchesLegacyKey(data, "pageUp") || kittyMatches(FUNC_PAGE_UP, 0);
		return matchesLegacyModifierSequence(data, "pageUp", modifier) || kittyMatches(FUNC_PAGE_UP, modifier);
	}

	if (key === "pagedown") {
		if (modifier === 0) return matchesLegacyKey(data, "pageDown") || kittyMatches(FUNC_PAGE_DOWN, 0);
		return matchesLegacyModifierSequence(data, "pageDown", modifier) || kittyMatches(FUNC_PAGE_DOWN, modifier);
	}

	if (key === "up") {
		if (modifier === MOD_ALT) return kittyMatches(ARROW_UP, MOD_ALT);
		if (modifier === 0) return matchesLegacyKey(data, "up") || kittyMatches(ARROW_UP, 0);
		return matchesLegacyModifierSequence(data, "up", modifier) || kittyMatches(ARROW_UP, modifier);
	}

	if (key === "down") {
		if (modifier === MOD_ALT) return kittyMatches(ARROW_DOWN, MOD_ALT);
		if (modifier === 0) return matchesLegacyKey(data, "down") || kittyMatches(ARROW_DOWN, 0);
		return matchesLegacyModifierSequence(data, "down", modifier) || kittyMatches(ARROW_DOWN, modifier);
	}

	if (key === "left") {
		if (modifier === MOD_ALT) {
			return data === "\x1b[1;3D" || (!kittyProtocolActive && data === "\x1bB") || kittyMatches(ARROW_LEFT, MOD_ALT);
		}
		if (modifier === MOD_CTRL) {
			return data === "\x1b[1;5D" || matchesLegacyModifierSequence(data, "left", MOD_CTRL) || kittyMatches(ARROW_LEFT, MOD_CTRL);
		}
		if (modifier === 0) return matchesLegacyKey(data, "left") || kittyMatches(ARROW_LEFT, 0);
		return matchesLegacyModifierSequence(data, "left", modifier) || kittyMatches(ARROW_LEFT, modifier);
	}

	if (key === "right") {
		if (modifier === MOD_ALT) {
			return data === "\x1b[1;3C" || (!kittyProtocolActive && data === "\x1bF") || kittyMatches(ARROW_RIGHT, MOD_ALT);
		}
		if (modifier === MOD_CTRL) {
			return data === "\x1b[1;5C" || matchesLegacyModifierSequence(data, "right", MOD_CTRL) || kittyMatches(ARROW_RIGHT, MOD_CTRL);
		}
		if (modifier === 0) return matchesLegacyKey(data, "right") || kittyMatches(ARROW_RIGHT, 0);
		return matchesLegacyModifierSequence(data, "right", modifier) || kittyMatches(ARROW_RIGHT, modifier);
	}

	// Function keys f1-f12
	const fMatch = key.match(/^f([1-9]|1[0-2])$/);
	if (fMatch) {
		const num = parseInt(fMatch[1], 10);
		const cp = FUNC_F1 - (num - 1);
		if (modifier === 0) return matchesLegacyKey(data, key);
		return kittyMatches(cp, modifier);
	}

	// Single-character graphic keys (33-126)
	if (key.length === 1) {
		const ch = key;
		const code = ch.charCodeAt(0);
		if (code < 33 || code > 126) return false;

		const isLetter = code >= 97 && code <= 122;
		const codepoint = code;

		// Legacy ctrl+alt+letter
		if (modifier === (MOD_CTRL | MOD_ALT) && isLetter) {
			const ctrlChar = rawCtrlChar(ch);
			if (data.length === 2 && data.charCodeAt(0) === 0x1b && data.charCodeAt(1) === ctrlChar && !isNamedKeyLegacyByte(ctrlChar)) {
				return true;
			}
		}

		// Legacy alt+letter
		if (modifier === MOD_ALT && isLetter && data.length === 2 && data.charCodeAt(0) === 0x1b && data[1] === ch) {
			return true;
		}

		// Legacy alt+shift+letter
		if (modifier === (MOD_ALT | MOD_SHIFT) && isLetter && data.length === 2 && data.charCodeAt(0) === 0x1b && data[1] === ch.toUpperCase()) {
			return true;
		}

		// ctrl+key
		if (modifier === MOD_CTRL) {
			if (isLetter) {
				const raw = rawCtrlChar(ch);
				if (data.length === 1 && data.charCodeAt(0) === raw && !isNamedKeyLegacyByte(raw)) {
					return true;
				}
				return mokMatches(codepoint, MOD_CTRL) || kittyMatches(codepoint, MOD_CTRL);
			}

			const legacyCtrl = ctrlSymbolToByte(ch);
			if (legacyCtrl !== null && data.length === 1 && data.charCodeAt(0) === legacyCtrl && !isNamedKeyLegacyByte(legacyCtrl)) {
				return true;
			}
			return mokMatches(codepoint, MOD_CTRL) || kittyMatches(codepoint, MOD_CTRL);
		}

		// ctrl+shift
		if (modifier === (MOD_CTRL | MOD_SHIFT)) {
			return kittyMatches(codepoint, MOD_SHIFT | MOD_CTRL) || mokMatches(codepoint, MOD_SHIFT | MOD_CTRL);
		}

		// shift+key
		if (modifier === MOD_SHIFT) {
			if (isLetter && data.length === 1 && data === ch.toUpperCase()) {
				return true;
			}
			return kittyMatches(codepoint, MOD_SHIFT) || mokMatches(codepoint, MOD_SHIFT);
		}

		// Other modifiers
		if (modifier !== 0) {
			return kittyMatches(codepoint, modifier) || mokMatches(codepoint, modifier);
		}

		// Plain key
		return (data.length === 1 && data === ch) || kittyMatches(codepoint, 0);
	}

	return false;
}
