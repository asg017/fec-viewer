//! A developer's view of a filing's raw bytes: every line with its byte
//! offset, what the parser makes of it, and its fields named from the
//! mappings, as text or as a hex dump.

mod data;
pub mod index;
mod rows;
mod view;

pub use view::{DebugView, Mode, Target, init};
