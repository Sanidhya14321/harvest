//! Terminal protocol primitives shared by the native application and N-API
//! bridge.
/// Shared safety bound for one terminal CSI sequence.
pub const MAX_CSI_BYTES: usize = 4096;

pub mod deccara;
pub mod highlight;
pub mod input;
pub mod keys;
pub mod mouse;
pub mod output;
pub mod paste;
pub mod text;
pub mod writer;
