//! Daylight-saving changes end to end: a schedule whose allowed night spans the clock change keeps
//! every commit inside the hours, writes the offset that is valid at that instant, and a rerun
//! finds nothing to do.

mod common;

use chrono::{Datelike, Duration, Offset, TimeZone, Timelike};
use common::*;

const TZ: &str = "Europe/Berlin";

/// Saturday and Sunday nights from 22:00 to 04:00, which contain the 02:00 clock change.
fn cfg(from: &str, to: &str) -> String {
    format!(
        "version: 1\nfrom: {from}\nto: {to}\ntimezone: {TZ}\nschedule:\n  days: [sat, sun]\n  hours: \"22:00-04:00\"\n  distribution: uniform\n  seed: 5\n"
    )
}

/// (committer time, committer UTC offset in minutes) of every commit, oldest first.
fn committer_offsets(r: &Repo) -> Vec<(i64, i32)> {
    r.git(&[
        "log",
        "--reverse",
        "--topo-order",
        "--format=%cd",
        "--date=raw",
    ])
    .lines()
    .map(|l| {
        let (t, off) = l.split_once(' ').unwrap();
        let sign = if off.starts_with('-') { -1 } else { 1 };
        let (h, m): (i32, i32) = (off[1..3].parse().unwrap(), off[3..5].parse().unwrap());
        (t.parse().unwrap(), sign * (h * 60 + m))
    })
    .collect()
}

/// The night of a day belongs to the day it starts on: 22:00-24:00 of a Saturday or Sunday, or the
/// first four hours of the day after one.
fn in_a_weekend_night(t: i64) -> bool {
    let tz: chrono_tz::Tz = TZ.parse().unwrap();
    let local = tz.timestamp_opt(t, 0).unwrap();
    let weekend =
        |d: chrono::NaiveDate| matches!(d.weekday(), chrono::Weekday::Sat | chrono::Weekday::Sun);
    (local.hour() >= 22 && weekend(local.date_naive()))
        || (local.hour() < 4 && weekend(local.date_naive() - Duration::days(1)))
}

fn check_change(from: &str, to: &str) {
    let r = Repo::new();
    // A year before the schedule starts, so every commit has to be moved.
    for i in 0..60 {
        r.commit_at(
            &format!("f{i}.txt"),
            &format!("c{i}"),
            1_750_000_000 + i * 3_600,
        );
    }
    r.config(&cfg(from, to));
    let old = r.log();
    r.ghma_ok(&["apply", "--from", "root"]);
    r.fsck();
    let new = r.log();
    assert_same_content(&old, &new);

    let tz: chrono_tz::Tz = TZ.parse().unwrap();
    let offsets = committer_offsets(&r);
    for (t, off) in &offsets {
        assert!(
            in_a_weekend_night(*t),
            "{} is outside the allowed nights",
            tz.timestamp_opt(*t, 0).unwrap()
        );
        let valid = tz
            .timestamp_opt(*t, 0)
            .unwrap()
            .offset()
            .fix()
            .local_minus_utc()
            / 60;
        assert_eq!(*off, valid, "offset written for {t}");
    }
    let seen: std::collections::BTreeSet<i32> = offsets.iter().map(|(_, o)| *o).collect();
    assert_eq!(
        seen,
        [60, 120].into(),
        "commits land on both sides of the change"
    );
    assert!(
        new.iter().all(|x| x.at == x.ct),
        "author time follows the committer time"
    );

    assert!(
        r.ghma_ok(&["apply", "--from", "root"])
            .contains("Nothing to do")
    );
    assert!(
        r.ghma(&["plan", "--check", "--from", "root"])
            .status
            .success()
    );
}

#[test]
fn spring_forward_night_keeps_commits_in_the_hours_with_the_right_offsets() {
    // 2026-03-29: 02:00 CET jumps to 03:00 CEST; 02:xx does not exist.
    check_change("2026-03-27", "2026-03-31");
}

#[test]
fn fall_back_night_keeps_commits_in_the_hours_with_the_right_offsets() {
    // 2026-10-25: 03:00 CEST falls back to 02:00 CET; 02:xx happens twice.
    check_change("2026-10-23", "2026-10-27");
}
