//! Working hours beyond the single daytime range: several ranges per day, ranges that run past
//! midnight, and commits whose original time is outside every range.

mod common;

use chrono::{Datelike, Duration, TimeZone, Timelike};
use common::*;

const TZ: &str = "Europe/Berlin";

fn cfg(days: &str, hours: &str, dist: &str) -> String {
    format!(
        "version: 1\nfrom: 2026-01-05\nto: 2026-03-01\ntimezone: {TZ}\nschedule:\n  days: {days}\n  hours: {hours}\n  distribution: {dist}\n  seed: 11\n"
    )
}

fn berlin(y: i32, m: u32, d: u32, h: u32, mi: u32) -> i64 {
    let tz: chrono_tz::Tz = TZ.parse().unwrap();
    tz.with_ymd_and_hms(y, m, d, h, mi, 0).unwrap().timestamp()
}

/// Independent oracle: is `t` inside one of `ranges` (minutes of the day they start on, the end
/// may exceed 24:00) of an allowed weekday? A range belongs to the day it starts on.
fn allowed(t: i64, days: &[chrono::Weekday], ranges: &[(i64, i64)]) -> bool {
    let tz: chrono_tz::Tz = TZ.parse().unwrap();
    let local = tz.timestamp_opt(t, 0).unwrap();
    (0..=1).any(|back| {
        let day = local.date_naive() - Duration::days(back);
        let minute = i64::from(local.hour() * 60 + local.minute()) + 1440 * back;
        days.contains(&day.weekday()) && ranges.iter().any(|&(s, e)| s <= minute && minute < e)
    })
}

fn assert_all_allowed(rows: &[Row], days: &[chrono::Weekday], ranges: &[(i64, i64)]) {
    let tz: chrono_tz::Tz = TZ.parse().unwrap();
    for r in rows {
        assert!(
            allowed(r.ct, days, ranges),
            "{} is outside the working hours: {}",
            r.subject,
            tz.timestamp_opt(r.ct, 0).unwrap()
        );
        assert_eq!(r.at, r.ct, "author time follows the committer time");
    }
    let by: std::collections::HashMap<&str, i64> =
        rows.iter().map(|r| (r.oid.as_str(), r.ct)).collect();
    for r in rows {
        for p in &r.parents {
            assert!(
                by[p.as_str()] <= r.ct,
                "a child is never older than its parent"
            );
        }
    }
}

const WEEKDAYS: [chrono::Weekday; 5] = [
    chrono::Weekday::Mon,
    chrono::Weekday::Tue,
    chrono::Weekday::Wed,
    chrono::Weekday::Thu,
    chrono::Weekday::Fri,
];

/// Commits made at noon of consecutive days, outside every range used below.
fn noon_history(r: &Repo, n: usize) {
    for i in 0..n {
        let day = berlin(2025, 6, 2, 12, 0) + i as i64 * 86_400;
        r.commit_at(&format!("f{i}.txt"), &format!("noon {i}"), day);
    }
}

fn local_hour(t: i64) -> u32 {
    let tz: chrono_tz::Tz = TZ.parse().unwrap();
    tz.timestamp_opt(t, 0).unwrap().hour()
}

fn local_weekday(t: i64) -> chrono::Weekday {
    let tz: chrono_tz::Tz = TZ.parse().unwrap();
    tz.timestamp_opt(t, 0).unwrap().weekday()
}

#[test]
fn two_ranges_in_one_day_evening_and_early_morning() {
    let r = Repo::new();
    noon_history(&r, 40);
    r.config(&cfg(
        "[mon, tue, wed, thu, fri]",
        "[\"18:00-24:00\", \"06:00-07:00\"]",
        "uniform",
    ));
    let old = r.log();
    r.gcma_ok(&["apply", "--from", "root"]);
    let new = r.log();
    assert_same_content(&old, &new);
    assert_all_allowed(&new, &WEEKDAYS, &[(1080, 1440), (360, 420)]);
    assert!(
        new.iter().any(|x| local_hour(x.ct) == 6),
        "the morning range is used"
    );
    assert!(
        new.iter().any(|x| local_hour(x.ct) >= 18),
        "the evening range is used"
    );
    assert!(
        new.iter().all(|x| local_hour(x.ct) != 12),
        "no commit stays at noon, which is in neither range"
    );
    r.fsck();
    assert!(
        r.gcma_ok(&["apply", "--from", "root"])
            .contains("Nothing to do")
    );
    assert!(
        r.gcma(&["plan", "--check", "--from", "root"])
            .status
            .success()
    );
}

#[test]
fn a_range_running_past_midnight_spills_into_the_next_weekday() {
    let r = Repo::new();
    noon_history(&r, 60);
    // Only Fridays 18:00 until 06:00 on Saturday.
    r.config(&cfg("[fri]", "\"18:00-06:00\"", "uniform"));
    r.gcma_ok(&["apply", "--from", "root"]);
    let new = r.log();
    assert_all_allowed(&new, &[chrono::Weekday::Fri], &[(1080, 1800)]);
    let saturday_morning = new
        .iter()
        .filter(|x| local_weekday(x.ct) == chrono::Weekday::Sat)
        .count();
    assert!(
        saturday_morning > 0,
        "some commits land after midnight, on a Saturday"
    );
    assert!(
        new.iter()
            .filter(|x| local_weekday(x.ct) == chrono::Weekday::Sat)
            .all(|x| local_hour(x.ct) < 6),
        "but only before 06:00"
    );
    assert!(
        new.iter()
            .all(|x| local_weekday(x.ct) != chrono::Weekday::Sun)
    );
    r.fsck();
}

#[test]
fn overnight_range_and_a_second_range_together() {
    let r = Repo::new();
    noon_history(&r, 50);
    r.config(&cfg(
        "[mon, tue, wed, thu, fri, sat, sun]",
        "[\"22:00-02:00\", \"12:30-13:30\"]",
        "bursty",
    ));
    r.gcma_ok(&["apply", "--from", "root"]);
    let new = r.log();
    let all_days = [
        chrono::Weekday::Mon,
        chrono::Weekday::Tue,
        chrono::Weekday::Wed,
        chrono::Weekday::Thu,
        chrono::Weekday::Fri,
        chrono::Weekday::Sat,
        chrono::Weekday::Sun,
    ];
    assert_all_allowed(&new, &all_days, &[(1320, 1560), (750, 810)]);
    assert!(
        r.gcma(&["plan", "--check", "--from", "root"])
            .status
            .success()
    );
}

#[test]
fn a_commit_at_noon_moves_to_the_next_valid_slot_and_one_inside_stays() {
    let r = Repo::new();
    // Monday 2026-01-12: 19:00 is inside 18:00-24:00; the noon commits after it are not.
    let inside = r.commit_at_offset(
        "a.txt",
        "monday evening",
        berlin(2026, 1, 12, 19, 0),
        "+0100",
    );
    r.commit_at(
        "b.txt",
        "monday noon",
        berlin(2026, 1, 12, 20, 0) - 8 * 3600 + 24 * 3600,
    );
    r.commit_at("c.txt", "saturday noon", berlin(2026, 1, 17, 12, 0));
    r.commit_at("d.txt", "sunday noon", berlin(2026, 1, 18, 12, 0));
    r.config(&cfg(
        "[mon, tue, wed, thu, fri]",
        "\"18:00-24:00\"",
        "uniform",
    ));
    let old = r.log();
    r.gcma_ok(&["apply", "--from", "root"]);
    let new = r.log();
    assert_eq!(
        new[0].oid, inside,
        "a commit already in the window keeps its identity"
    );
    assert_all_allowed(&new, &WEEKDAYS, &[(1080, 1440)]);
    for (o, n) in old.iter().zip(&new).skip(1) {
        assert_ne!(o.ct, n.ct, "{} had to move", n.subject);
        assert_ne!(local_hour(n.ct), 12);
    }
    r.fsck();
}

#[test]
fn a_commit_just_outside_the_window_edges_is_moved_and_one_on_the_edge_is_not() {
    let r = Repo::new();
    // Window 09:00-17:00 on weekdays. 09:00 is in, 17:00 is out (half-open).
    let first = r.commit_at_offset("a.txt", "on the start", berlin(2026, 1, 12, 9, 0), "+0100");
    let second = r.commit_at_offset("b.txt", "on the end", berlin(2026, 1, 12, 17, 0), "+0100");
    r.config(&cfg(
        "[mon, tue, wed, thu, fri]",
        "\"09:00-17:00\"",
        "uniform",
    ));
    r.gcma_ok(&["apply", "--from", "root"]);
    let new = r.log();
    assert_eq!(new[0].oid, first, "09:00 is inside");
    assert_ne!(new[1].oid, second, "17:00 is outside and moves");
    assert_all_allowed(&new, &WEEKDAYS, &[(540, 1020)]);
}

#[test]
fn hours_as_a_single_string_and_as_a_list_mean_the_same() {
    let build = |hours: &str| {
        let r = Repo::new();
        noon_history(&r, 12);
        r.config(&cfg("[mon, tue, wed]", hours, "uniform"));
        r.gcma_ok(&["apply", "--from", "root"]);
        r.log().iter().map(|x| x.ct).collect::<Vec<_>>()
    };
    assert_eq!(build("\"18:00-22:00\""), build("[\"18:00-22:00\"]"));
}

#[test]
fn invalid_hours_are_usage_errors() {
    for hours in [
        "[]",
        "\"09:00-09:00\"",
        "\"24:00-06:00\"",
        "\"9-18\"",
        "[\"18:00-24:00\", \"nonsense\"]",
        "5",
    ] {
        let r = Repo::new();
        r.linear(2, 1_600_000_000);
        r.config(&cfg("[mon]", hours, "uniform"));
        let o = r.gcma(&["plan", "--from", "root"]);
        assert_eq!(
            Repo::code(&o),
            2,
            "{hours}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
    }
}

#[test]
fn a_window_without_any_matching_day_is_refused_even_with_overnight_hours() {
    let r = Repo::new();
    r.linear(3, 1_600_000_000);
    // The window starts on Tuesday 06:00, just when Monday's 18:00-06:00 range ends.
    r.config("version: 1\nfrom: \"2026-01-06T06:00:00+01:00\"\nto: \"2026-01-06T23:00:00+01:00\"\ntimezone: Europe/Berlin\nschedule:\n  days: [mon]\n  hours: \"18:00-06:00\"\n");
    let o = r.gcma(&["apply", "--from", "root"]);
    assert_eq!(Repo::code(&o), 3, "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(r.log().len(), 3, "nothing was rewritten");
}

#[test]
fn a_window_starting_after_midnight_still_gets_the_tail_of_the_previous_days_range() {
    let r = Repo::new();
    r.linear(4, 1_600_000_000);
    // Tuesday 2026-01-06 only, but Monday's range reaches into it until 06:00.
    r.config("version: 1\nfrom: 2026-01-06\nto: 2026-01-06\ntimezone: Europe/Berlin\nschedule:\n  days: [mon]\n  hours: \"18:00-06:00\"\n");
    r.gcma_ok(&["apply", "--from", "root"]);
    for x in r.log() {
        assert_eq!(local_weekday(x.ct), chrono::Weekday::Tue);
        assert!(local_hour(x.ct) < 6, "{}", x.ct);
    }
}
