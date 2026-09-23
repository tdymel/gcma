//! The time model: the set of instants at which a commit may be made.
//! One predicate (`Window::contains`) is shared by the scheduler and the conformance check.

use std::collections::HashSet;

use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, Offset, TimeZone, Weekday};
use chrono_tz::Tz;

#[derive(Debug, Clone, Copy)]
pub struct Interval {
    pub start: i64,
    pub end: i64, // exclusive
    pub weekday: Weekday,
}

impl Interval {
    pub(super) fn len(&self) -> u64 {
        (self.end - self.start) as u64
    }
}

/// The set of allowed UTC instants: configured weekdays and hours in the configured timezone,
/// inside `[from, to)`. Half-open intervals. This is the single predicate used by both the scheduler
/// and the conformance check.
#[derive(Debug, Clone)]
pub struct Window {
    pub tz: Tz,
    intervals: Vec<Interval>,
}

fn local_to_utc(tz: &Tz, naive: NaiveDateTime) -> i64 {
    match tz.from_local_datetime(&naive) {
        chrono::LocalResult::Single(t) => t.timestamp(),
        chrono::LocalResult::Ambiguous(a, _) => a.timestamp(),
        chrono::LocalResult::None => {
            // DST gap: move forward to the first instant that exists.
            let mut n = naive;
            for _ in 0..24 * 60 {
                n += Duration::minutes(1);
                if let chrono::LocalResult::Single(t) | chrono::LocalResult::Ambiguous(t, _) =
                    tz.from_local_datetime(&n)
                {
                    return t.timestamp();
                }
            }
            tz.from_utc_datetime(&naive).timestamp()
        }
    }
}

impl Window {
    /// `hours` are (start, end) minutes of the day an interval starts on; an end beyond 24:00 runs
    /// into the next day, which keeps the weekday of the day it started.
    pub fn build(tz: Tz, days: &[Weekday], hours: &[(u32, u32)], from: i64, to: i64) -> Window {
        let allowed: HashSet<Weekday> = days.iter().copied().collect();
        let mut intervals: Vec<Interval> = Vec::new();
        if to > from {
            let first = tz
                .timestamp_opt(from, 0)
                .unwrap()
                .date_naive()
                .pred_opt()
                .unwrap();
            let last = tz
                .timestamp_opt(to, 0)
                .unwrap()
                .date_naive()
                .succ_opt()
                .unwrap();
            let mut d: NaiveDate = first;
            while d <= last {
                if allowed.contains(&d.weekday()) {
                    let midnight = d.and_hms_opt(0, 0, 0).unwrap();
                    for &(from_min, to_min) in hours {
                        let s = local_to_utc(&tz, midnight + Duration::minutes(from_min as i64));
                        let e = local_to_utc(&tz, midnight + Duration::minutes(to_min as i64));
                        let (s, e) = (s.max(from), e.min(to));
                        if e > s {
                            intervals.push(Interval {
                                start: s,
                                end: e,
                                weekday: d.weekday(),
                            });
                        }
                    }
                }
                d = d.succ_opt().unwrap();
            }
            intervals.sort_by_key(|i| i.start);
            // Make disjoint (DST oddities could make neighbours touch or overlap).
            let mut disjoint: Vec<Interval> = Vec::new();
            for mut i in intervals {
                if let Some(prev) = disjoint.last()
                    && i.start < prev.end
                {
                    i.start = prev.end;
                }
                if i.end > i.start {
                    disjoint.push(i);
                }
            }
            intervals = disjoint;
        }
        Window { tz, intervals }
    }

    /// Allowed seconds inside `[lo, to)`.
    #[cfg(test)]
    pub fn capacity(&self, lo: i64) -> u64 {
        self.clipped(lo).iter().map(|i| i.len()).sum()
    }

    pub fn contains(&self, t: i64) -> bool {
        let idx = self.intervals.partition_point(|i| i.end <= t);
        self.intervals.get(idx).is_some_and(|i| i.start <= t)
    }

    pub fn tz_offset_minutes(&self, t: i64) -> i32 {
        let off = self
            .tz
            .timestamp_opt(t, 0)
            .unwrap()
            .offset()
            .fix()
            .local_minus_utc();
        off / 60
    }

    /// Window intervals clipped to `[lo, to)`.
    pub(super) fn clipped(&self, lo: i64) -> Vec<Interval> {
        self.intervals
            .iter()
            .filter_map(|i| {
                let s = i.start.max(lo);
                (i.end > s).then_some(Interval {
                    start: s,
                    end: i.end,
                    weekday: i.weekday,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::super::testkit::{ts, weekdays};
    use super::*;

    #[test]
    fn window_membership_is_half_open_and_weekday_limited() {
        let tz = chrono_tz::UTC;
        // 2026-04-06 is a Monday, 04-04 a Saturday.
        let from = ts(tz, 2026, 4, 1, 0, 0);
        let to = ts(tz, 2026, 4, 30, 0, 0);
        let w = Window::build(tz, &weekdays(), &[(9 * 60, 18 * 60)], from, to);
        assert!(w.contains(ts(tz, 2026, 4, 6, 9, 0)));
        assert!(w.contains(ts(tz, 2026, 4, 6, 17, 59)));
        assert!(!w.contains(ts(tz, 2026, 4, 6, 18, 0)), "end is exclusive");
        assert!(!w.contains(ts(tz, 2026, 4, 6, 8, 59)));
        assert!(!w.contains(ts(tz, 2026, 4, 4, 12, 0)), "Saturday");
        assert!(!w.contains(from - 1) && !w.contains(to));
    }

    #[test]
    fn window_honours_timezone() {
        let tz: Tz = "Europe/Berlin".parse().unwrap();
        let w = Window::build(
            tz,
            &weekdays(),
            &[(9 * 60, 18 * 60)],
            ts(tz, 2026, 4, 1, 0, 0),
            ts(tz, 2026, 5, 1, 0, 0),
        );
        assert!(w.contains(ts(tz, 2026, 4, 7, 9, 0)));
        assert!(!w.contains(ts(tz, 2026, 4, 7, 8, 59)));
        assert_eq!(w.tz_offset_minutes(ts(tz, 2026, 4, 7, 12, 0)), 120); // CEST
        assert_eq!(w.tz_offset_minutes(ts(tz, 2026, 1, 7, 12, 0)), 60); // CET
    }

    #[test]
    fn dst_gap_shrinks_the_interval() {
        // Europe/Berlin spring forward: 2026-03-29 02:00 -> 03:00 (a Sunday).
        let tz: Tz = "Europe/Berlin".parse().unwrap();
        let all = [Weekday::Sun];
        let from = ts(tz, 2026, 3, 29, 0, 0);
        let to = ts(tz, 2026, 3, 30, 0, 0);
        let w = Window::build(tz, &all, &[(60, 4 * 60)], from, to); // 01:00-04:00 local
        // Real duration is 2h (01:00-02:00 and 03:00-04:00), not 3h.
        assert_eq!(w.capacity(from), 2 * 3600);
        // Every instant the window contains has a local time inside 01:00-04:00.
        let mut t = from;
        while t < to {
            if w.contains(t) {
                let l = tz.timestamp_opt(t, 0).unwrap();
                let mins = chrono::Timelike::hour(&l) * 60 + chrono::Timelike::minute(&l);
                assert!((60..240).contains(&mins));
            }
            t += 60;
        }
    }

    #[test]
    fn an_overnight_range_belongs_to_the_weekday_it_starts_on() {
        let tz = chrono_tz::UTC;
        // Fridays 18:00 until 06:00 the next morning. 2026-04-10 is a Friday.
        let from = ts(tz, 2026, 4, 6, 0, 0);
        let to = ts(tz, 2026, 4, 20, 0, 0);
        let w = Window::build(tz, &[Weekday::Fri], &[(18 * 60, 30 * 60)], from, to);
        assert!(w.contains(ts(tz, 2026, 4, 10, 18, 0)));
        assert!(w.contains(ts(tz, 2026, 4, 10, 23, 59)));
        assert!(
            w.contains(ts(tz, 2026, 4, 11, 0, 0)),
            "after midnight, a Saturday"
        );
        assert!(w.contains(ts(tz, 2026, 4, 11, 5, 59)));
        assert!(!w.contains(ts(tz, 2026, 4, 11, 6, 0)), "end is exclusive");
        assert!(
            !w.contains(ts(tz, 2026, 4, 11, 18, 0)),
            "Saturday evening is not allowed"
        );
        assert!(
            !w.contains(ts(tz, 2026, 4, 10, 5, 0)),
            "Friday morning belongs to Thursday"
        );
        assert!(!w.contains(ts(tz, 2026, 4, 10, 17, 59)));
        assert_eq!(
            w.capacity(from),
            2 * 12 * 3600,
            "two Fridays of twelve hours"
        );
    }

    #[test]
    fn the_tail_of_an_overnight_range_before_from_still_counts_after_from() {
        let tz = chrono_tz::UTC;
        // `from` lies in the small hours of Saturday, inside Friday's range.
        let from = ts(tz, 2026, 4, 11, 3, 0);
        let to = ts(tz, 2026, 4, 13, 0, 0);
        let w = Window::build(tz, &[Weekday::Fri], &[(18 * 60, 30 * 60)], from, to);
        assert!(!w.contains(ts(tz, 2026, 4, 11, 2, 59)), "before from");
        assert!(w.contains(from));
        assert!(w.contains(ts(tz, 2026, 4, 11, 5, 59)));
        assert_eq!(w.capacity(from), 3 * 3600);
    }

    #[test]
    fn the_head_of_an_overnight_range_is_cut_at_to() {
        let tz = chrono_tz::UTC;
        let from = ts(tz, 2026, 4, 6, 0, 0);
        let to = ts(tz, 2026, 4, 11, 1, 0);
        let w = Window::build(tz, &[Weekday::Fri], &[(18 * 60, 30 * 60)], from, to);
        assert!(w.contains(ts(tz, 2026, 4, 11, 0, 59)));
        assert!(!w.contains(to));
        assert_eq!(w.capacity(from), 7 * 3600);
    }

    #[test]
    fn several_ranges_per_day_are_all_allowed() {
        let tz = chrono_tz::UTC;
        let from = ts(tz, 2026, 4, 6, 0, 0);
        let to = ts(tz, 2026, 4, 13, 0, 0);
        let w = Window::build(
            tz,
            &weekdays(),
            &[(18 * 60, 24 * 60), (6 * 60, 7 * 60)],
            from,
            to,
        );
        assert!(w.contains(ts(tz, 2026, 4, 7, 6, 30)));
        assert!(w.contains(ts(tz, 2026, 4, 7, 20, 0)));
        assert!(!w.contains(ts(tz, 2026, 4, 7, 7, 0)));
        assert!(
            !w.contains(ts(tz, 2026, 4, 7, 12, 0)),
            "noon is in neither range"
        );
        assert!(!w.contains(ts(tz, 2026, 4, 11, 20, 0)), "Saturday");
        assert_eq!(w.capacity(from), 5 * 7 * 3600);
    }

    #[test]
    fn overlapping_and_repeated_ranges_count_once() {
        let tz = chrono_tz::UTC;
        let from = ts(tz, 2026, 4, 6, 0, 0);
        let to = ts(tz, 2026, 4, 7, 0, 0);
        let w = Window::build(
            tz,
            &[Weekday::Mon],
            &[
                (9 * 60, 12 * 60),
                (10 * 60, 15 * 60),
                (10 * 60, 15 * 60),
                (22 * 60, 26 * 60),
            ],
            from,
            to,
        );
        assert_eq!(
            w.capacity(from),
            (6 + 2) * 3600,
            "09:00-15:00 and 22:00-24:00 (cut at to)"
        );
        assert!(w.contains(ts(tz, 2026, 4, 6, 14, 59)));
        assert!(!w.contains(ts(tz, 2026, 4, 6, 15, 0)));
    }
}
