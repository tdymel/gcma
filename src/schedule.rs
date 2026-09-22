//! The time model (`Window`) and the pure scheduler. Nothing here knows about git.

use std::collections::HashSet;
use std::hash::Hasher;

use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, Offset, TimeZone, Weekday};
use chrono_tz::Tz;
use fnv::FnvHasher;
use rand::{RngExt, SeedableRng};
use rand_chacha::ChaCha8Rng;

use crate::config::{Distribution, weekday_index};
use crate::error::{Error, Result};

/// The generator used by the scheduler (ChaCha8: value-stable across platforms and versions).
pub type ScheduleRng = ChaCha8Rng;

pub fn rng_from_seed(seed: u64) -> ScheduleRng {
    ScheduleRng::seed_from_u64(seed)
}

/// FNV-1a over a list of byte strings (stable seed derivation); parts cannot run together.
pub fn derive_seed(parts: &[&[u8]]) -> u64 {
    let mut h = FnvHasher::default();
    for p in parts {
        h.write(p);
        h.write_u8(0xff);
    }
    h.finish()
}

#[derive(Debug, Clone, Copy)]
pub struct Interval {
    pub start: i64,
    pub end: i64, // exclusive
    pub weekday: Weekday,
}

impl Interval {
    fn len(&self) -> u64 {
        (self.end - self.start) as u64
    }
}

/// The set of allowed UTC instants: configured weekdays and hours in the configured timezone,
/// inside `[from, to)`. Half-open intervals. This is the single predicate used by both the scheduler
/// and the conformance check.
#[derive(Debug, Clone)]
pub struct Window {
    pub tz: Tz,
    pub from: i64,
    pub to: i64,
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
    pub fn build(tz: Tz, days: &[Weekday], hours: (u32, u32), from: i64, to: i64) -> Window {
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
                    let s = local_to_utc(&tz, midnight + Duration::minutes(hours.0 as i64));
                    let e = local_to_utc(&tz, midnight + Duration::minutes(hours.1 as i64));
                    let (s, e) = (s.max(from), e.min(to));
                    if e > s {
                        intervals.push(Interval {
                            start: s,
                            end: e,
                            weekday: d.weekday(),
                        });
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
        Window {
            tz,
            from,
            to,
            intervals,
        }
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
    fn clipped(&self, lo: i64) -> Vec<Interval> {
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

    /// Allowed seconds inside `[lo, to)`.
    pub fn capacity(&self, lo: i64) -> u64 {
        self.clipped(lo).iter().map(|i| i.len()).sum()
    }
}

/// Weekday weights for the weekday-weighted distribution (Mon..Sun): busier midweek.
const WEEKDAY_WEIGHTS: [f64; 7] = [0.85, 1.1, 1.25, 1.1, 0.8, 0.25, 0.15];

/// Maps a position in "capacity space" (the concatenation of all allowed seconds) to an instant.
struct Space {
    ivs: Vec<Interval>,
    cum: Vec<u64>,
    total: u64,
}

impl Space {
    fn new(ivs: Vec<Interval>) -> Space {
        let mut cum = Vec::with_capacity(ivs.len());
        let mut total = 0;
        for i in &ivs {
            cum.push(total);
            total += i.len();
        }
        Space { ivs, cum, total }
    }

    fn at(&self, p: u64) -> i64 {
        let p = p.min(self.total.saturating_sub(1));
        let idx = self.cum.partition_point(|&c| c <= p) - 1;
        self.ivs[idx].start + (p - self.cum[idx]) as i64
    }
}

/// Produces `n` sorted instants, every one inside the window and >= `floor`.
/// Equal timestamps are allowed. Deterministic for a given seed.
pub fn schedule<R: RngExt>(
    n: usize,
    window: &Window,
    floor: i64,
    dist: Distribution,
    rng: &mut R,
) -> Result<Vec<i64>> {
    if n == 0 {
        return Ok(Vec::new());
    }
    let space = Space::new(window.clipped(floor));
    if space.total == 0 {
        return Err(Error::Precondition(format!(
            "no allowed time left between the floor and `to` (capacity 0) for {n} commit(s); \
             widen the window, change `to`, or wait"
        )));
    }
    let mut times: Vec<i64> = Vec::with_capacity(n);
    match dist {
        Distribution::Uniform => {
            for _ in 0..n {
                times.push(space.at(rng.random_range(0..space.total)));
            }
        }
        Distribution::WeekdayWeighted => {
            let weights: Vec<f64> = space
                .ivs
                .iter()
                .map(|i| i.len() as f64 * WEEKDAY_WEIGHTS[weekday_index(i.weekday)])
                .collect();
            let total: f64 = weights.iter().sum();
            for _ in 0..n {
                let mut x = rng.random::<f64>() * total;
                let mut idx = weights.len() - 1;
                for (k, w) in weights.iter().enumerate() {
                    if x < *w {
                        idx = k;
                        break;
                    }
                    x -= w;
                }
                let iv = &space.ivs[idx];
                times.push(iv.start + rng.random_range(0..iv.len()) as i64);
            }
        }
        Distribution::Bursty => {
            // Sessions of ~1-8 commits (mean ~4) spread over up to 90 minutes of allowed time.
            // The session always fits inside the window, so nothing is clamped onto its last second.
            let span: u64 = (90 * 60).min(space.total);
            let mut left = n;
            while left > 0 {
                let mut size = 1;
                while size < 8 && rng.random::<f64>() < 0.75 {
                    size += 1;
                }
                let size = size.min(left);
                left -= size;
                let start = rng.random_range(0..space.total - span + 1);
                for _ in 0..size {
                    times.push(space.at(start + rng.random_range(0..span)));
                }
            }
        }
    }
    times.sort_unstable();
    Ok(times)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn weekdays() -> Vec<Weekday> {
        vec![
            Weekday::Mon,
            Weekday::Tue,
            Weekday::Wed,
            Weekday::Thu,
            Weekday::Fri,
        ]
    }

    fn ts(tz: Tz, y: i32, m: u32, d: u32, h: u32, mi: u32) -> i64 {
        tz.with_ymd_and_hms(y, m, d, h, mi, 0).unwrap().timestamp()
    }

    #[test]
    fn rng_is_deterministic_and_bounded() {
        let (mut a, mut b) = (rng_from_seed(42), rng_from_seed(42));
        for _ in 0..100 {
            assert_eq!(a.random_range(0..u64::MAX), b.random_range(0..u64::MAX));
        }
        let mut r = rng_from_seed(1);
        for _ in 0..1000 {
            assert!(r.random_range(0..7u64) < 7);
            assert!((0.0..1.0).contains(&r.random::<f64>()));
        }
    }

    #[test]
    fn window_membership_is_half_open_and_weekday_limited() {
        let tz = chrono_tz::UTC;
        // 2026-04-06 is a Monday, 04-04 a Saturday.
        let from = ts(tz, 2026, 4, 1, 0, 0);
        let to = ts(tz, 2026, 4, 30, 0, 0);
        let w = Window::build(tz, &weekdays(), (9 * 60, 18 * 60), from, to);
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
            (9 * 60, 18 * 60),
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
        let w = Window::build(tz, &all, (60, 4 * 60), from, to); // 01:00-04:00 local
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
    fn dst_overlap_counts_both_passes_and_is_consistent() {
        // Europe/Berlin fall back: 2026-10-25 03:00 -> 02:00 (a Sunday): local 02:00-03:00 occurs twice.
        let tz: Tz = "Europe/Berlin".parse().unwrap();
        let from = ts(tz, 2026, 10, 25, 0, 0);
        let to = ts(tz, 2026, 10, 26, 0, 0);
        let w = Window::build(tz, &[Weekday::Sun], (0, 4 * 60), from, to);
        assert_eq!(w.capacity(from), 5 * 3600);
        let mut r = rng_from_seed(3);
        let times = schedule(200, &w, from, Distribution::Uniform, &mut r).unwrap();
        assert!(times.iter().all(|t| w.contains(*t)));
    }

    #[test]
    fn schedule_outputs_conform_for_every_distribution() {
        let tz: Tz = "America/New_York".parse().unwrap();
        let from = ts(tz, 2026, 1, 1, 0, 0);
        let to = ts(tz, 2026, 7, 1, 0, 0);
        let w = Window::build(tz, &weekdays(), (9 * 60 + 30, 17 * 60 + 45), from, to);
        for dist in [
            Distribution::Uniform,
            Distribution::WeekdayWeighted,
            Distribution::Bursty,
        ] {
            let mut r = rng_from_seed(99);
            let times = schedule(500, &w, from, dist, &mut r).unwrap();
            assert_eq!(times.len(), 500);
            assert!(times.windows(2).all(|p| p[0] <= p[1]), "sorted");
            for t in &times {
                assert!(
                    w.contains(*t),
                    "{dist:?} produced a time outside the window"
                );
                assert!(*t >= from);
            }
        }
    }

    #[test]
    fn schedule_is_deterministic_and_seed_sensitive() {
        let tz = chrono_tz::UTC;
        let from = ts(tz, 2026, 1, 1, 0, 0);
        let to = ts(tz, 2026, 3, 1, 0, 0);
        let w = Window::build(tz, &weekdays(), (9 * 60, 18 * 60), from, to);
        let a = schedule(50, &w, from, Distribution::Bursty, &mut rng_from_seed(5)).unwrap();
        let b = schedule(50, &w, from, Distribution::Bursty, &mut rng_from_seed(5)).unwrap();
        let c = schedule(50, &w, from, Distribution::Bursty, &mut rng_from_seed(6)).unwrap();
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn floor_is_respected() {
        let tz = chrono_tz::UTC;
        let from = ts(tz, 2026, 1, 1, 0, 0);
        let to = ts(tz, 2026, 3, 1, 0, 0);
        let w = Window::build(tz, &weekdays(), (9 * 60, 18 * 60), from, to);
        let floor = ts(tz, 2026, 2, 10, 12, 0);
        let times = schedule(100, &w, floor, Distribution::Uniform, &mut rng_from_seed(1)).unwrap();
        assert!(times.iter().all(|t| *t >= floor));
    }

    #[test]
    fn zero_capacity_is_an_error() {
        let tz = chrono_tz::UTC;
        // Friday 18:00 floor, `to` Monday 08:00: nothing allowed in between.
        let from = ts(tz, 2026, 4, 1, 0, 0);
        let floor = ts(tz, 2026, 4, 3, 18, 0); // Friday
        let to = ts(tz, 2026, 4, 6, 8, 0); // Monday
        let w = Window::build(tz, &weekdays(), (9 * 60, 18 * 60), from, to);
        assert_eq!(w.capacity(floor), 0);
        let e = schedule(1, &w, floor, Distribution::Uniform, &mut rng_from_seed(1)).unwrap_err();
        assert_eq!(e.exit_code(), 3);
        assert!(
            schedule(0, &w, floor, Distribution::Uniform, &mut rng_from_seed(1))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn empty_window_is_an_error() {
        let tz = chrono_tz::UTC;
        let w = Window::build(tz, &weekdays(), (540, 1080), 100, 50);
        assert!(!w.contains(75));
        assert!(schedule(1, &w, 0, Distribution::Uniform, &mut rng_from_seed(1)).is_err());
    }

    #[test]
    fn weekday_weighted_prefers_midweek() {
        let tz = chrono_tz::UTC;
        let from = ts(tz, 2026, 1, 1, 0, 0);
        let to = ts(tz, 2026, 12, 31, 0, 0);
        let all = [
            Weekday::Mon,
            Weekday::Tue,
            Weekday::Wed,
            Weekday::Thu,
            Weekday::Fri,
            Weekday::Sat,
            Weekday::Sun,
        ];
        let w = Window::build(tz, &all, (9 * 60, 18 * 60), from, to);
        let times = schedule(
            7000,
            &w,
            from,
            Distribution::WeekdayWeighted,
            &mut rng_from_seed(11),
        )
        .unwrap();
        let mut counts = [0usize; 7];
        for t in times {
            counts[weekday_index(tz.timestamp_opt(t, 0).unwrap().weekday())] += 1;
        }
        assert!(
            counts[2] > counts[5] * 3,
            "Wednesday should dominate Saturday: {counts:?}"
        );
    }

    #[test]
    fn no_distribution_piles_commits_onto_one_instant() {
        let tz = chrono_tz::UTC;
        let from = ts(tz, 2026, 1, 5, 0, 0);
        let to = ts(tz, 2026, 1, 12, 0, 0); // one working week
        let w = Window::build(tz, &weekdays(), (9 * 60, 18 * 60), from, to);
        for dist in [
            Distribution::Uniform,
            Distribution::WeekdayWeighted,
            Distribution::Bursty,
        ] {
            for seed in 0..20 {
                let times = schedule(300, &w, from, dist, &mut rng_from_seed(seed)).unwrap();
                let last = *times.last().unwrap();
                let at_last = times.iter().filter(|t| **t == last).count();
                assert!(
                    at_last <= 3,
                    "{dist:?} seed {seed}: {at_last} commits share the final instant"
                );
                let first = times[0];
                assert!(
                    times.iter().filter(|t| **t == first).count() <= 3,
                    "{dist:?}: pile-up at the start"
                );
            }
        }
    }

    #[test]
    fn bursty_in_a_window_shorter_than_a_session_still_works() {
        let tz = chrono_tz::UTC;
        let from = ts(tz, 2026, 1, 5, 9, 0);
        let to = ts(tz, 2026, 1, 5, 9, 30); // 30 minutes only
        let w = Window::build(tz, &weekdays(), (9 * 60, 18 * 60), from, to);
        let times = schedule(40, &w, from, Distribution::Bursty, &mut rng_from_seed(2)).unwrap();
        assert!(times.iter().all(|t| w.contains(*t)));
    }

    #[test]
    fn seed_derivation_is_stable_and_unambiguous() {
        assert_eq!(derive_seed(&[b"a"]), derive_seed(&[b"a"]));
        assert_ne!(derive_seed(&[b"a", b"b"]), derive_seed(&[b"ab"]));
    }
}
