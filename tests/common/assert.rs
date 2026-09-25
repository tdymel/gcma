//! Assertions about what a run did to a history: the graph, the trees and the schedule.

use std::collections::{HashMap, HashSet};

use chrono::{Datelike, TimeZone, Timelike};

use super::repo::Row;

/// Pairs every old commit with the new commit that replaced it: same tree and subject, and the
/// parents are the replacements of the old parents, in the same order. Panics when a commit has
/// no counterpart or two commits would share one.
pub fn map_commits(old: &[Row], new: &[Row]) -> HashMap<String, String> {
    map_commits_by(old, new, true)
}

/// `map_commits`, optionally ignoring the subjects (for runs that rewrite messages).
fn map_commits_by(old: &[Row], new: &[Row], same_subject: bool) -> HashMap<String, String> {
    let mut map: HashMap<String, String> = Default::default();
    let mut taken: HashSet<&str> = Default::default();
    for o in old {
        let parents: Vec<String> = o.parents.iter().map(|p| map[p].clone()).collect();
        let n = new
            .iter()
            .find(|n| {
                !taken.contains(n.oid.as_str())
                    && n.tree == o.tree
                    && (!same_subject || n.subject == o.subject)
                    && n.parents == parents
            })
            .unwrap_or_else(|| panic!("no replacement for {} ({:?})", o.oid, o.subject));
        taken.insert(&n.oid);
        map.insert(o.oid.clone(), n.oid.clone());
    }
    map
}

/// Asserts that the two histories are the same shape and every tree and subject is untouched.
pub fn assert_same_content(old: &[Row], new: &[Row]) {
    assert_eq!(old.len(), new.len(), "commit count changed");
    map_commits(old, new);
}

/// Same shape and trees, but the messages may differ.
pub fn assert_same_shape(old: &[Row], new: &[Row]) {
    assert_eq!(old.len(), new.len(), "commit count changed");
    map_commits_by(old, new, false);
}

/// A config that schedules weekdays 09:30-18:00 in Berlin between 2026-01-05 and 2026-03-01,
/// followed by `extra` (more top-level keys).
pub fn berlin_cfg(extra: &str) -> String {
    format!(
        "version: 1\nfrom: 2026-01-05\nto: 2026-03-01\ntimezone: Europe/Berlin\nschedule:\n  days: [mon, tue, wed, thu, fri]\n  hours: \"09:30-18:00\"\n  distribution: bursty\n  seed: 7\n{extra}"
    )
}

/// Every row follows `berlin_cfg`: inside the range and the hours, author time equal to committer
/// time, and no commit older than its parents.
pub fn assert_scheduled(rows: &[Row]) {
    let tz: chrono_tz::Tz = "Europe/Berlin".parse().unwrap();
    let from = tz
        .with_ymd_and_hms(2026, 1, 5, 0, 0, 0)
        .unwrap()
        .timestamp();
    let to = tz
        .with_ymd_and_hms(2026, 3, 2, 0, 0, 0)
        .unwrap()
        .timestamp();
    for r in rows {
        assert_eq!(r.at, r.ct, "author time equals committer time");
        assert!(r.ct >= from && r.ct < to, "inside the range");
        let l = tz.timestamp_opt(r.ct, 0).unwrap();
        assert!(l.weekday().number_from_monday() <= 5, "weekday: {l}");
        let mins = l.hour() * 60 + l.minute();
        assert!((570..1080).contains(&mins), "working hours: {l}");
    }
    let by: HashMap<&str, i64> = rows.iter().map(|r| (r.oid.as_str(), r.ct)).collect();
    for r in rows {
        for p in &r.parents {
            assert!(by[p.as_str()] <= r.ct, "monotone along parents");
        }
    }
}
