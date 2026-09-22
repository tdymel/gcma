//! Scheduling: the allowed-time `Window`, the seeded generator and the distributions.

mod scheduler;
mod seed;
mod window;

pub use scheduler::schedule;
pub use seed::{ScheduleRng, derive_seed, rng_from_seed};
pub use window::{Interval, Window};

#[cfg(test)]
pub(crate) mod testkit {
    use chrono::{TimeZone, Weekday};
    use chrono_tz::Tz;

    pub fn weekdays() -> Vec<Weekday> {
        vec![
            Weekday::Mon,
            Weekday::Tue,
            Weekday::Wed,
            Weekday::Thu,
            Weekday::Fri,
        ]
    }

    pub fn ts(tz: Tz, y: i32, m: u32, d: u32, h: u32, mi: u32) -> i64 {
        tz.with_ymd_and_hms(y, m, d, h, mi, 0).unwrap().timestamp()
    }
}
