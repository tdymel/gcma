//! Helpers shared by the scheduling tests.

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
