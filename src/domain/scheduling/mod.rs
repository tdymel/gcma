//! Scheduling: the allowed-time `Window`, the seeded generator and the distributions.

mod scheduler;
mod seed;
mod window;

pub use scheduler::schedule;
pub use seed::{derive_seed, rng_from_seed};
pub use window::Window;

#[cfg(test)]
mod testkit;
