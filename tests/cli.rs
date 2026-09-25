mod common;

use common::*;

const IDENTITY_CFG: &str = "version: 1\nidentity:\n  - match: {email: me@home.org}\n    set: {name: Jane Doe, email: jane@work.com}\n";

// ---------- identity rewriting (milestone 1) ----------

#[test]
fn identity_rewrite_preserves_everything_else() {
    let r = Repo::new();
    r.linear(6, 1_600_000_000);
    r.config(IDENTITY_CFG);
    let old = r.log();
    let old_tip = old.last().unwrap().oid.clone();

    let plan = r.ghma_ok(&["plan", "--from", "root"]);
    assert!(plan.contains("6 to rewrite"), "{plan}");
    assert_eq!(
        r.git(&["rev-parse", "HEAD"]),
        old_tip,
        "plan must not write"
    );

    let out = r.ghma_ok(&["apply", "--from", "root"]);
    assert!(out.contains("Rewrote 6"), "{out}");
    let new = r.log();
    assert_same_content(&old, &new);
    for (o, n) in old.iter().zip(&new) {
        assert_eq!(n.an, "Jane Doe");
        assert_eq!(n.ae, "jane@work.com");
        assert_eq!(n.cn, "Jane Doe");
        assert_eq!(o.at, n.at, "no schedule: times are kept");
        assert_eq!(o.ct, n.ct);
        assert_eq!(o.subject, n.subject);
        assert_eq!(o.tree, n.tree);
    }
    r.fsck();
    // Backup refs exist and keep the old history reachable.
    let refs = r.git(&["for-each-ref", "refs/ghma/backup/"]);
    assert!(refs.contains("/old") && refs.contains("/new"), "{refs}");
    assert_eq!(r.git(&["rev-list", "--count", &old_tip]), "6");
}

#[test]
fn second_apply_is_a_noop_and_restore_returns_the_original_tip() {
    let r = Repo::new();
    r.linear(4, 1_600_000_000);
    r.config(IDENTITY_CFG);
    let old_tip = r.git(&["rev-parse", "HEAD"]);
    r.ghma_ok(&["apply", "--from", "root"]);
    let new_tip = r.git(&["rev-parse", "HEAD"]);
    assert_ne!(old_tip, new_tip);

    let again = r.ghma_ok(&["apply", "--from", "root"]);
    assert!(again.contains("Nothing to do"), "{again}");
    assert_eq!(r.git(&["rev-parse", "HEAD"]), new_tip);
    assert!(
        r.ghma(&["plan", "--check", "--from", "root"])
            .status
            .success()
    );

    let list = r.ghma_ok(&["restore"]);
    let id = list.split_whitespace().next().unwrap().to_string();
    // The branch is still at the recorded new tip: restore works without --force.
    r.ghma_ok(&["restore", &id]);
    assert_eq!(r.git(&["rev-parse", "HEAD"]), old_tip);
    r.fsck();
}

#[test]
fn restore_refuses_when_the_branch_moved_unless_forced() {
    let r = Repo::new();
    r.linear(3, 1_600_000_000);
    r.config(IDENTITY_CFG);
    let old_tip = r.git(&["rev-parse", "HEAD"]);
    r.ghma_ok(&["apply", "--from", "root"]);
    let id = r
        .ghma_ok(&["restore"])
        .split_whitespace()
        .next()
        .unwrap()
        .to_string();
    // New work after the rewrite (made as the new identity so it conforms).
    r.commit_as(
        "later.txt",
        "later",
        1_700_000_000,
        "Jane Doe",
        "jane@work.com",
    );
    let moved = r.git(&["rev-parse", "HEAD"]);
    let o = r.ghma(&["restore", &id]);
    assert_eq!(Repo::code(&o), 4, "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(
        r.git(&["rev-parse", "HEAD"]),
        moved,
        "refused restore must not move the branch"
    );
    r.ghma_ok(&["restore", &id, "--force"]);
    assert_eq!(r.git(&["rev-parse", "HEAD"]), old_tip);
    // The discarded tip keeps a ref, so nothing is lost even after gc.
    let holders = r.git(&["for-each-ref", "--contains", &moved, "refs/ghma/discarded/"]);
    assert!(holders.contains(&moved), "{holders}");
    let o = r.ghma(&["restore", "--prune"]);
    assert_eq!(Repo::code(&o), 2);
}

#[test]
fn prune_is_explicit_and_removes_both_refs() {
    let r = Repo::new();
    r.linear(2, 1_600_000_000);
    r.config(IDENTITY_CFG);
    r.ghma_ok(&["apply", "--from", "root"]);
    let id = r
        .ghma_ok(&["restore"])
        .split_whitespace()
        .next()
        .unwrap()
        .to_string();
    assert!(!r.git(&["for-each-ref", "refs/ghma/backup/"]).is_empty());
    r.ghma_ok(&["restore", &id, "--prune"]);
    assert!(r.git(&["for-each-ref", "refs/ghma/backup/"]).is_empty());
}

#[test]
fn merge_history_keeps_parent_order_and_trees() {
    let r = Repo::new();
    r.commit_at("base.txt", "base", 1_600_000_000);
    r.git(&["checkout", "-q", "-b", "side"]);
    r.commit_at("side1.txt", "side 1", 1_600_100_000);
    r.commit_at("side2.txt", "side 2", 1_600_200_000);
    r.git(&["checkout", "-q", "main"]);
    r.commit_at("main1.txt", "main 1", 1_600_300_000);
    r.git(&["merge", "-q", "--no-ff", "-m", "merge side", "side"]);
    r.commit_at("after.txt", "after merge", 1_600_400_000);
    r.config(IDENTITY_CFG);
    let old = r.log();
    let old_merge = old.iter().find(|x| x.parents.len() == 2).unwrap().clone();
    let old_first_parent_tree = old
        .iter()
        .find(|x| x.oid == old_merge.parents[0])
        .unwrap()
        .tree
        .clone();
    let old_second_parent_tree = old
        .iter()
        .find(|x| x.oid == old_merge.parents[1])
        .unwrap()
        .tree
        .clone();

    r.ghma_ok(&["apply", "--from", "root"]);
    let new = r.log();
    assert_same_content(&old, &new);
    let merge = new.iter().find(|x| x.parents.len() == 2).unwrap();
    let p0 = new.iter().find(|x| x.oid == merge.parents[0]).unwrap();
    let p1 = new.iter().find(|x| x.oid == merge.parents[1]).unwrap();
    assert_eq!(p0.tree, old_first_parent_tree, "first parent stays first");
    assert_eq!(p1.tree, old_second_parent_tree);
    assert!(new.iter().all(|x| x.an == "Jane Doe"));
    r.fsck();
    // First-parent history is intact.
    let fp = r.git(&["rev-list", "--first-parent", "--count", "HEAD"]);
    assert_eq!(fp, "4");
}

#[test]
fn frozen_commits_keep_their_oids_and_only_the_rest_changes() {
    let r = Repo::new();
    // Three commits already by the target identity, then two by the old identity.
    let a = r.commit_as("a.txt", "a", 1_600_000_000, "Jane Doe", "jane@work.com");
    let b = r.commit_as("b.txt", "b", 1_600_100_000, "Jane Doe", "jane@work.com");
    r.commit_at("c.txt", "c", 1_600_200_000);
    r.commit_at("d.txt", "d", 1_600_300_000);
    r.config(IDENTITY_CFG);
    let plan = r.ghma_ok(&["plan", "--from", "root"]);
    assert!(plan.contains("2 kept as-is, 2 to rewrite"), "{plan}");
    r.ghma_ok(&["apply", "--from", "root"]);
    let rows = r.log();
    assert_eq!(rows[0].oid, a);
    assert_eq!(rows[1].oid, b);
    assert_eq!(rows[2].an, "Jane Doe");
    assert_eq!(rows[2].parents, vec![b]);
}

#[test]
fn raw_headers_and_non_utf8_messages_survive() {
    let r = Repo::new();
    r.git(&["config", "i18n.commitEncoding", "ISO-8859-1"]);
    r.write("x.txt", "x\n");
    r.git(&["add", "x.txt"]);
    // A latin-1 message: "caf\xe9".
    let msg_file = r.path().join("msg.bin");
    std::fs::write(&msg_file, b"caf\xe9 au lait\n\nbody \xe9\n").unwrap();
    r.git(&["commit", "-q", "-F", msg_file.to_str().unwrap()]);
    r.config(IDENTITY_CFG);
    let old = r.git(&["rev-parse", "HEAD"]);
    let old_raw = r.cat(&old);
    assert!(
        old_raw.windows(18).any(|w| w == b"encoding ISO-8859-"),
        "fixture has an encoding header"
    );

    r.ghma_ok(&["apply", "--from", "root"]);
    let new = r.git(&["rev-parse", "HEAD"]);
    let new_raw = r.cat(&new);
    assert!(
        new_raw.windows(18).any(|w| w == b"encoding ISO-8859-"),
        "encoding header kept"
    );
    let body_of = |raw: &[u8]| {
        raw.windows(2)
            .position(|w| w == b"\n\n")
            .map(|p| raw[p + 2..].to_vec())
            .unwrap()
    };
    assert_eq!(
        body_of(&old_raw),
        body_of(&new_raw),
        "message bytes are identical"
    );
    r.fsck();
}

// ---------- scheduling (milestone 3) ----------

#[test]
fn schedule_distributes_commits_into_working_hours() {
    let r = Repo::new();
    // Old commits at night / weekends, years earlier.
    r.linear(25, 1_500_000_000);
    r.config(&berlin_cfg(""));
    let old = r.log();
    r.ghma_ok(&["apply", "--from", "root"]);
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
fn every_distribution_conforms_and_is_deterministic() {
    for dist in ["uniform", "weekday-weighted", "bursty"] {
        let mut tips = Vec::new();
        for _ in 0..2 {
            let r = Repo::new();
            r.linear(15, 1_500_000_000);
            r.config(&berlin_cfg("").replace("bursty", dist));
            r.ghma_ok(&["apply", "--from", "root"]);
            assert_scheduled(&r.log());
            tips.push(r.log().iter().map(|x| x.ct).collect::<Vec<_>>());
            assert!(
                r.ghma(&["plan", "--check", "--from", "root"])
                    .status
                    .success(),
                "{dist}"
            );
        }
        assert_eq!(tips[0], tips[1], "{dist}: same input, same schedule");
    }
}

#[test]
fn hook_case_only_new_commits_are_rescheduled() {
    let r = Repo::new();
    r.linear(10, 1_500_000_000);
    r.config(&berlin_cfg(""));
    r.ghma_ok(&["apply", "--from", "root"]);
    let settled = r.log();
    // Three new commits made "now-ish" (outside the allowed window).
    let t = settled.last().unwrap().ct + 3600 * 24 * 3 + 7 * 3600; // a night, a few days later
    for i in 0..3 {
        r.commit_at(&format!("new{i}.txt"), &format!("new {i}"), t + i * 60);
    }
    let plan = r.ghma_ok(&["plan", "--from", "root"]);
    assert!(plan.contains("10 kept as-is, 3 to rewrite"), "{plan}");
    r.ghma_ok(&["apply", "--from", "root"]);
    let after = r.log();
    for (a, b) in settled.iter().zip(&after) {
        assert_eq!(a.oid, b.oid, "settled commits keep their OIDs");
    }
    assert_eq!(after.len(), 13);
    assert_scheduled(&after);
    assert!(after[10].ct >= settled.last().unwrap().ct);
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
    r.ghma_ok(&["apply", "--from", "root"]);
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
    let o = r.ghma(&["apply", "--from", "root"]);
    assert_eq!(Repo::code(&o), 3, "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(r.log().len(), 3);
}
