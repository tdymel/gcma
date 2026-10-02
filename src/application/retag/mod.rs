//! `--retag`: tags and notes follow the commits a rewrite replaces.

mod classify;
mod moves;
mod notes;
mod preview;

pub use moves::{Moves, TagMove, prepare};
pub use notes::copy_notes;
pub use preview::{Preview, preview};
