//! Shared collab wire constants.
//!
//! These values are the Rust-owned source of truth for the collab protocol.
//! Transitional TypeScript consumers continue to define their own copies until
//! their switch-over is complete.

pub const COLLAB_PROMPT_MESSAGE_TYPE: &str = "collab-prompt";
pub const COLLAB_PROTO: u8 = 3;
pub const ENVELOPE_HEADER_LENGTH: usize = 4;
pub const ROOM_ID_BYTES: usize = 16;
pub const ROOM_KEY_BYTES: usize = 32;
pub const WRITE_TOKEN_BYTES: usize = 16;
pub const DEFAULT_RELAY_URL: &str = "wss://my.omp.sh";
pub const DEFAULT_SHARE_URL: &str = "https://my.omp.sh/s";
pub const INTENT_FIELD: &str = "i";

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn collab_constants_match_legacy_wire_values() {
		assert_eq!(COLLAB_PROTO, 3);
		assert_eq!(COLLAB_PROMPT_MESSAGE_TYPE, "collab-prompt");
		assert_eq!(ENVELOPE_HEADER_LENGTH, 4);
		assert_eq!(ROOM_ID_BYTES, 16);
		assert_eq!(DEFAULT_RELAY_URL, "wss://my.omp.sh");
	}
}
