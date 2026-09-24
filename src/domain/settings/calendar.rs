//! Parsing of the calendar-ish settings: instants, weekdays and working hours.

use chrono::{NaiveDate, TimeZone, Weekday};
use chrono_tz::Tz;

use crate::domain::error::{Error, Result};

pub(super) fn parse_instant(s: &str, tz: &Tz, end_of_day: bool) -> Result<i64> {
    if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        let d = if end_of_day {
            d.succ_opt().unwrap_or(d)
        } else {
            d
        };
        return Ok(local_midnight(d, tz));
    }
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|t| t.timestamp())
        .map_err(|_| {
            Error::Usage(format!(
                "config: cannot parse date {s:?} (use YYYY-MM-DD or RFC 3339)"
            ))
        })
}

/// The first instant at or after local midnight of `d`. When a DST gap swallows midnight (Sao Paulo
/// before 2019, Havana, Beirut) that is the moment the gap ends; an overlap takes the earlier offset.
fn local_midnight(d: NaiveDate, tz: &Tz) -> i64 {
    let midnight = d.and_hms_opt(0, 0, 0).unwrap();
    // Gaps are whole multiples of 15 minutes, and none spans more than a day.
    (0..=24 * 4)
        .find_map(|quarter| {
            let naive = midnight + chrono::Duration::minutes(15 * quarter);
            tz.from_local_datetime(&naive).earliest()
        })
        .map_or_else(|| tz.from_utc_datetime(&midnight).timestamp(), |t| t.timestamp())
}

pub fn parse_days(days: &[String]) -> Result<Vec<Weekday>> {
    if days.is_empty() {
        return Err(Error::Usage("config: schedule.days is empty".into()));
    }
    days.iter()
        .map(|d| {
            d.parse::<Weekday>()
                .map_err(|_| Error::Usage(format!("config: unknown weekday {d:?}")))
        })
        .collect()
}

/// Ranges as (start minute, end minute) of the day they start on. A range that ends at or before
/// its start runs past midnight, so its end lies beyond 24:00 (`18:00-06:00` is `(1080, 1800)`).
pub fn parse_hours(ranges: &[String]) -> Result<Vec<(u32, u32)>> {
    if ranges.is_empty() {
        return Err(Error::Usage("config: schedule.hours is empty".into()));
    }
    ranges.iter().map(|r| parse_range(r)).collect()
}

fn parse_range(s: &str) -> Result<(u32, u32)> {
    let bad = || Error::Usage(format!("config: bad hours {s:?} (expected HH:MM-HH:MM)"));
    let (a, b) = s.split_once('-').ok_or_else(bad)?;
    let p = |x: &str| -> Option<u32> {
        let (h, m) = x.trim().split_once(':')?;
        let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
        (m < 60 && (h < 24 || (h == 24 && m == 0))).then_some(h * 60 + m)
    };
    let (start, end) = (p(a).ok_or_else(bad)?, p(b).ok_or_else(bad)?);
    if start == end {
        return Err(Error::Usage(format!(
            "config: hours {s:?}: start and end are the same (use 00:00-24:00 for the whole day)"
        )));
    }
    if start >= 24 * 60 {
        return Err(bad());
    }
    Ok((start, if end > start { end } else { end + 24 * 60 }))
}

pub fn weekday_index(w: Weekday) -> usize {
    w.num_days_from_monday() as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(s: &str) -> Result<Vec<(u32, u32)>> {
        parse_hours(&[s.to_string()])
    }

    #[test]
    fn hours_parse() {
        assert_eq!(one("09:30-18:00").unwrap(), [(570, 1080)]);
        assert_eq!(one("00:00-24:00").unwrap(), [(0, 1440)]);
        assert!(one("9-18").is_err());
        assert!(one("09:00-25:00").is_err());
        assert!(one("24:00-06:00").is_err());
        assert!(one("09:00-09:00").is_err());
        assert!(parse_hours(&[]).is_err());
    }

    #[test]
    fn a_range_ending_before_its_start_runs_into_the_next_day() {
        assert_eq!(one("18:00-06:00").unwrap(), [(1080, 1800)]);
        assert_eq!(one("22:30-00:00").unwrap(), [(1350, 1440)]);
        assert_eq!(one("23:00-00:30").unwrap(), [(1380, 1470)]);
    }

    fn at(s: &str, tz: &str, end_of_day: bool) -> i64 {
        parse_instant(s, &tz.parse().unwrap(), end_of_day).unwrap()
    }

    #[test]
    fn midnight_swallowed_by_a_dst_gap_starts_when_the_gap_ends() {
        // America/Havana: 2026-03-08 00:00 does not exist, clocks jump to 01:00 (-04:00 = 05:00Z).
        let from = at("2026-03-08", "America/Havana", false);
        assert_eq!(from, 1_772_946_000); // 2026-03-08T05:00:00Z
        // The end of the previous day is that same instant.
        assert_eq!(at("2026-03-07", "America/Havana", true), from);
    }

    #[test]
    fn an_ordinary_midnight_and_an_overlap_are_unchanged() {
        assert_eq!(at("2026-01-01", "UTC", false), 1_767_225_600);
        assert_eq!(at("2026-01-01", "Europe/Berlin", false), 1_767_225_600 - 3600);
        // America/Havana 2026-11-01 00:00 occurs twice; the earlier (CDT, -04:00) is used.
        assert_eq!(at("2026-11-01", "America/Havana", false), 1_793_505_600);
    }

    #[test]
    fn several_ranges_keep_their_order() {
        let r = parse_hours(&["18:00-24:00".into(), "06:00-07:00".into()]).unwrap();
        assert_eq!(r, [(1080, 1440), (360, 420)]);
    }

    #[test]
    fn all_seven_days_parse_by_short_or_full_name_in_any_case() {
        let days =
            parse_days(&["mon", "Tuesday", "WED", "thu", "fri", "Sat", "sunday"].map(String::from))
                .unwrap();
        assert_eq!(days.len(), 7);
        assert_eq!(days[5], Weekday::Sat);
        assert_eq!(days[6], Weekday::Sun);
        assert!(parse_days(&["funday".to_string()]).is_err());
        assert!(parse_days(&[]).is_err());
    }
}
