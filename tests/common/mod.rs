//! Shared helpers of the integration tests: a scratch git repository (`repo`) with a builder for
//! histories (`commits`) and for a random one (`dag`), readers for what a run left behind (`read`),
//! assertions about what a run did to the schedule and the graph (`assert`), and the outside world
//! of ssh signing and bare remotes (`signing`). This file holds the small things all of them share:
//! the configs and the first commit time most tests use, a seeded random number generator, and the
//! text of a command's output.
//!
//! Every test binary pulls in the whole module and uses a part of it.
#![allow(dead_code, unused_imports)]

use std::process::Output;

use rand::{RngExt, SeedableRng};
use rand_chacha::ChaCha8Rng;

mod assert;
mod commits;
mod dag;
mod read;
mod repo;
mod signing;

pub use assert::*;
pub use read::*;
pub use repo::*;
pub use signing::*;

/// Rewrites `me@home.org` (the default identity of `Repo`) to Jane Doe.
pub const IDENTITY_CFG: &str = "version: 1\nidentity:\n  - match: {email: me@home.org}\n    set: {name: Jane Doe, email: jane@work.com}\n";

/// The identity rule of `IDENTITY_CFG` without the version line, to put into a larger config.
pub const IDENTITY_RULE: &str =
    "identity:\n  - match: {email: me@home.org}\n    set: {name: Jane Doe, email: jane@work.com}\n";

/// Takes everything under `secrets/` out of the history.
pub const SECRETS_CFG: &str = "version: 1\npaths:\n  exclude: [\"secrets/\"]\n";

/// `SECRETS_CFG` with the `.gitignore` entries for the excluded paths turned off.
pub const SECRETS_NO_GITIGNORE_CFG: &str =
    "version: 1\npaths:\n  exclude: [\"secrets/\"]\n  gitignore: false\n";

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

/// A seeded generator: the same seed always builds the same history.
pub struct Rand(ChaCha8Rng);

impl Rand {
    pub fn new(seed: u64) -> Rand {
        Rand(ChaCha8Rng::seed_from_u64(seed))
    }

    /// A number in `0..n`.
    pub fn below(&mut self, n: u64) -> u64 {
        self.0.random_range(0..n)
    }
}
