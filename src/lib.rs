//! ghma: safely reshape the history of the current branch.
//!
//! Layers (hexagonal): `domain` is pure; `application` holds the use cases and the ports they
//! need; `adapters` implement the ports and drive the use cases from the command line.

pub mod adapters;
pub mod application;
pub mod domain;
