//! Shared ECMAScript whitespace pattern for native source-data heuristics.
pub(crate) const JS_SPACE: &str = r"[\x09-\x0D\x20\x{00A0}\x{1680}\x{2000}-\x{200A}\x{2028}\x{2029}\x{202F}\x{205F}\x{3000}\x{FEFF}]";
