//! The parts of the command line adapter that every command uses: the grammar, the session, the
//! backend choice and the output helpers.

pub mod args;
mod backend;
mod render;
pub mod report;
pub mod session;
