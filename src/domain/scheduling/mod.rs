//! Scheduling: the allowed-time `Window`, the seeded generator and the distributions.

mod scheduler;
mod seed;
mod window;

pub use scheduler::schedule;
pub use seed::{ScheduleRng, derive_seed, rng_from_seed};
pub use window::{Interval, Window};

#[cfg(test)]
mod testkit;
