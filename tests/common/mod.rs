//! Shared helpers of the integration tests: a scratch git repository with a builder for histories
//! (`repo`), assertions about what a run did to the schedule and the graph (`assert`), and the
//! outside world of ssh signing and bare remotes (`signing`).
//!
//! Every test binary pulls in the whole module and uses a part of it.
#![allow(dead_code, unused_imports)]

mod assert;
mod repo;
mod signing;

pub use assert::*;
pub use repo::*;
pub use signing::*;
