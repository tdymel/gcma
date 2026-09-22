//! The domain: pure rules about time windows, settings and text. Nothing here performs I/O,
//! spawns processes or knows about git's storage.

pub mod error;
pub mod history;
pub mod paths;
pub mod scheduling;
pub mod settings;
pub mod text;
