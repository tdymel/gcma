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
        let naive = d.and_hms_opt(0, 0, 0).unwrap();
        return match tz.from_local_datetime(&naive) {
            chrono::LocalResult::Single(t) => Ok(t.timestamp()),
            chrono::LocalResult::Ambiguous(a, _) => Ok(a.timestamp()),
            chrono::LocalResult::None => Ok(tz.from_utc_datetime(&naive).timestamp()),
        };
    }
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|t| t.timestamp())
        .map_err(|_| {
            Error::Usage(format!(
                "config: cannot parse date {s:?} (use YYYY-MM-DD or RFC 3339)"
            ))
        })
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

/// "HH:MM-HH:MM" -> (start_minute, end_minute); end may be 24:00.
pub fn parse_hours(s: &str) -> Result<(u32, u32)> {
    let bad = || Error::Usage(format!("config: bad hours {s:?} (expected HH:MM-HH:MM)"));
    let (a, b) = s.split_once('-').ok_or_else(bad)?;
    let p = |x: &str| -> Option<u32> {
        let (h, m) = x.trim().split_once(':')?;
        let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
        (m < 60 && (h < 24 || (h == 24 && m == 0))).then_some(h * 60 + m)
    };
    let (start, end) = (p(a).ok_or_else(bad)?, p(b).ok_or_else(bad)?);
    if start >= end {
        return Err(Error::Usage(format!(
            "config: hours {s:?}: start must be before end"
        )));
    }
    Ok((start, end))
}

pub fn weekday_index(w: Weekday) -> usize {
    w.num_days_from_monday() as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hours_parse() {
        assert_eq!(parse_hours("09:30-18:00").unwrap(), (570, 1080));
        assert_eq!(parse_hours("00:00-24:00").unwrap(), (0, 1440));
        assert!(parse_hours("18:00-09:00").is_err());
        assert!(parse_hours("9-18").is_err());
        assert!(parse_hours("09:00-25:00").is_err());
    }
}
