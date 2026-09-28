//! Shared helpers of the integration tests: a scratch git repository with a builder for histories
//! (`repo`), assertions about what a run did to the schedule and the graph (`assert`), and the
//! outside world of ssh signing and bare remotes (`signing`). This file holds the small things all
//! of them share: the configs and the first commit time most tests use, a seeded random number
//! generator, and the text of a command's output.
//!
//! Every test binary pulls in the whole module and uses a part of it.
#![allow(dead_code, unused_imports)]

use std::process::Output;

mod assert;
mod repo;
mod signing;

pub use assert::*;
pub use repo::*;
pub use signing::*;

/// Rewrites `me@home.org` (the default identity of `Repo`) to Jane Doe.
pub const IDENTITY_CFG: &str = "version: 1\nidentity:\n  - match: {email: me@home.org}\n    set: {name: Jane Doe, email: jane@work.com}\n";

/// Takes everything under `secrets/` out of the history.
pub const SECRETS_CFG: &str = "version: 1\npaths:\n  exclude: [\"secrets/\"]\n";

/// A commit time (2020-09-13) for histories that need no particular schedule.
pub const T0: i64 = 1_600_000_000;

/// What a command wrote to stdout.
pub fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).to_string()
}

/// What a command wrote to stderr.
pub fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).to_string()
}

/// A xorshift generator: the same seed always builds the same history.
pub struct Rand(pub u64);

impl Rand {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}
