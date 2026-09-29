//! The schedule: commits are spread over the configured days and hours, deterministically and
//! monotone along parents (every distribution, merges, a window without capacity), and a commit
//! that is inside the hours but carries the wrong UTC offset is fixed.

mod common;

use chrono::TimeZone;
use common::*;

#[test]
fn schedule_distributes_commits_into_working_hours() {
    let r = Repo::new();
    // Old commits at night / weekends, years earlier.
    r.linear(25, 1_500_000_000);
    r.config(&berlin_cfg(""));
    let old = r.log();
    r.gcma_ok(&["apply", "--from", "root"]);
    let new = r.log();
    assert_same_content(&old, &new);
    assert_scheduled(&new);
    assert!(
        new.iter()
            .map(|x| x.subject.clone())
            .eq(old.iter().map(|x| x.subject.clone())),
        "messages untouched"
    );
    r.fsck();
    // Idempotent: rerun is a no-op and `--check` passes.
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
fn every_distribution_conforms_and_is_deterministic() {
    for dist in ["uniform", "weekday-weighted", "bursty"] {
        let mut tips = Vec::new();
        for _ in 0..2 {
            let r = Repo::new();
            r.linear(15, 1_500_000_000);
            r.config(&berlin_cfg("").replace("bursty", dist));
            r.gcma_ok(&["apply", "--from", "root"]);
            assert_scheduled(&r.log());
            tips.push(r.log().iter().map(|x| x.ct).collect::<Vec<_>>());
            assert!(
                r.gcma(&["plan", "--check", "--from", "root"])
                    .status
                    .success(),
                "{dist}"
            );
        }
        assert_eq!(tips[0], tips[1], "{dist}: same input, same schedule");
    }
}

#[test]
fn schedule_with_merge_is_monotone_and_complete() {
    let r = Repo::new();
    r.commit_at("base.txt", "base", 1_500_000_000);
    r.git(&["checkout", "-q", "-b", "side"]);
    r.commit_at("s1.txt", "s1", 1_500_100_000);
    r.commit_at("s2.txt", "s2", 1_500_200_000);
    r.git(&["checkout", "-q", "main"]);
    r.commit_at("m1.txt", "m1", 1_500_300_000);
    r.commit_at("m2.txt", "m2", 1_500_400_000);
    r.git(&["merge", "-q", "--no-ff", "-m", "merge", "side"]);
    r.commit_at("z.txt", "z", 1_500_500_000);
    r.config(&berlin_cfg(""));
    let old = r.log();
    r.gcma_ok(&["apply", "--from", "root"]);
    let new = r.log();
    assert_same_content(&old, &new);
    assert_scheduled(&new);
    r.fsck();
}

#[test]
fn zero_capacity_window_is_refused_with_exit_3() {
    let r = Repo::new();
    r.linear(3, 1_500_000_000);
    // Only Mondays 09:00-10:00 in a window that contains no Monday.
    r.config("version: 1\nfrom: 2026-01-06\nto: 2026-01-09\nschedule:\n  days: [mon]\n  hours: \"09:00-10:00\"\n");
    let o = r.gcma(&["apply", "--from", "root"]);
    assert_eq!(Repo::code(&o), 3, "{}", stderr(&o));
    assert_eq!(r.log().len(), 3);
}

#[test]
fn a_commit_inside_the_hours_but_with_the_wrong_offset_is_fixed_and_its_neighbour_is_not() {
    let r = Repo::new();
    let berlin = |h| {
        let tz: chrono_tz::Tz = "Europe/Berlin".parse().unwrap();
        tz.with_ymd_and_hms(2026, 1, 12, h, 0, 0)
            .unwrap()
            .timestamp()
    };
    let good = r.commit_at_offset("good.txt", "right offset", berlin(10), "+0100");
    r.commit_at_offset("bad.txt", "wrong offset", berlin(11), "+0000");
    r.config(&berlin_cfg(""));
    let o = r.gcma(&["plan", "--check", "--from", "root"]);
    assert_eq!(Repo::code(&o), 6, "{}", stderr(&o));
    assert!(stderr(&o).contains("1 commit(s)"), "{}", stderr(&o));
    r.gcma_ok(&["apply", "--from", "root"]);
    assert_eq!(r.log()[0].oid, good, "the conforming commit keeps its id");
    assert!(
        r.committer_offsets().iter().all(|(_, off)| *off == 60),
        "{:?}",
        r.committer_offsets()
    );
    assert_scheduled(&r.log());
    assert!(
        r.gcma(&["plan", "--check", "--from", "root"])
            .status
            .success()
    );
}
