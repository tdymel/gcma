//! The pure scheduler: turns a `Window` into sorted commit instants.

use rand::RngExt;

use super::window::{Interval, Window};
use crate::domain::error::{Error, Result};
use crate::domain::settings::{Distribution, weekday_index};

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
    use super::super::seed::rng_from_seed;
    use super::super::testkit::{ts, weekdays};
    use super::*;
    use chrono::{Datelike, TimeZone, Weekday};
    use chrono_tz::Tz;

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
        assert!(matches!(e, crate::domain::error::Error::Precondition(_)));
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
}
