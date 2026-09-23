//! ghma: safely reshape the history of the current branch.
//!
//! Layers (hexagonal): `domain` is pure; `application` holds the use cases and the ports they
//! need; `adapters` implement the ports and drive the use cases from the command line.

mod adapters;
mod application;
mod domain;

/// Runs the command line and returns the process exit code.
pub use adapters::cli::main;
