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

// ---------- safety and preconditions ----------

#[test]
fn tip_moved_after_planning_is_refused() {
    let r = Repo::new();
    r.linear(3, 1_600_000_000);
    r.config(IDENTITY_CFG);
    let plan = r.path().join("plan.json");
    r.ghma_ok(&["plan", "--from", "root", "--out", plan.to_str().unwrap()]);
    r.commit_at("extra.txt", "extra", 1_600_900_000);
    let tip = r.git(&["rev-parse", "HEAD"]);
    let o = r.ghma(&["apply", "--plan", plan.to_str().unwrap()]);
    assert_eq!(Repo::code(&o), 4, "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(r.git(&["rev-parse", "HEAD"]), tip);
}

#[test]
fn saved_plan_applies_later() {
    let r = Repo::new();
    r.linear(3, 1_600_000_000);
    r.config(IDENTITY_CFG);
    let plan = r.path().join("plan.json");
    r.ghma_ok(&["plan", "--from", "root", "--out", plan.to_str().unwrap()]);
    r.ghma_ok(&["apply", "--plan", plan.to_str().unwrap()]);
    assert!(r.log().iter().all(|x| x.an == "Jane Doe"));
}

#[test]
fn tampered_plan_is_rejected_before_anything_is_written() {
    let r = Repo::new();
    r.linear(3, 1_600_000_000);
    r.config(IDENTITY_CFG);
    let tip = r.git(&["rev-parse", "HEAD"]);
    let path = r.path().join("plan.json");
    r.ghma_ok(&["plan", "--from", "root", "--out", path.to_str().unwrap()]);
    let mut v: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    // Point entry 2's parent at entry 0 instead of entry 1.
    v["entries"][2]["parents"] = serde_json::json!([{"in": 0}]);
    std::fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
    let o = r.ghma(&["apply", "--plan", path.to_str().unwrap()]);
    assert!(!o.status.success());
    assert_eq!(r.git(&["rev-parse", "HEAD"]), tip);
    assert!(r.git(&["for-each-ref", "refs/ghma/backup/"]).is_empty());
}

#[test]
fn preconditions_detached_head_and_dirty_index_and_shallow() {
    let r = Repo::new();
    r.linear(3, 1_600_000_000);
    r.config(IDENTITY_CFG);
    // Dirty index.
    r.write("staged.txt", "s\n");
    r.git(&["add", "staged.txt"]);
    // The read-only dry run still works; writing is refused.
    r.ghma_ok(&["plan", "--from", "root"]);
    let tip = r.git(&["rev-parse", "HEAD"]);
    let o = r.ghma(&["apply", "--from", "root"]);
    assert_eq!(Repo::code(&o), 3, "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(r.git(&["rev-parse", "HEAD"]), tip);
    r.git(&[
        "commit",
        "-q",
        "-m",
        "staged",
        "--author",
        "Jane Doe <jane@work.com>",
    ]);
    // Detached HEAD.
    r.git(&["checkout", "-q", "--detach"]);
    let o = r.ghma(&["plan", "--from", "root"]);
    assert_eq!(Repo::code(&o), 3);
    r.git(&["checkout", "-q", "main"]);
    // Shallow clone.
    let clone = r.home.path().join("shallow");
    let o = r
        .cmd("git")
        .args(["clone", "-q", "--depth", "1"])
        .arg(format!("file://{}", r.path().display()))
        .arg(&clone)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let o = base_cmd(bin(), &clone, r.home.path())
        .args(["plan", "--from", "root"])
        .output()
        .unwrap();
    assert_eq!(Repo::code(&o), 3, "{}", String::from_utf8_lossy(&o.stderr));
}

#[test]
fn no_upstream_requires_from() {
    let r = Repo::new();
    r.linear(2, 1_600_000_000);
    r.config(IDENTITY_CFG);
    let o = r.ghma(&["plan"]);
    assert_eq!(Repo::code(&o), 2, "{}", String::from_utf8_lossy(&o.stderr));
    let o = r.ghma(&["plan", "--from", "HEAD~5"]);
    assert_eq!(Repo::code(&o), 2);
}

#[test]
fn from_rev_is_exclusive() {
    let r = Repo::new();
    let c = r.linear(4, 1_600_000_000);
    r.config(IDENTITY_CFG);
    r.ghma_ok(&["apply", "--from", &c[1]]);
    let rows = r.log();
    assert_eq!(rows[0].an, "Old Me");
    assert_eq!(rows[1].an, "Old Me");
    assert_eq!(rows[1].oid, c[1], "the --from commit itself is untouched");
    assert_eq!(rows[2].an, "Jane Doe");
    assert_eq!(rows[3].an, "Jane Doe");
}

#[test]
fn pushed_commits_need_the_flag() {
    let r = Repo::new();
    r.linear(3, 1_600_000_000);
    r.bare_remote();
    r.git(&["push", "-q", "-u", "origin", "main"]);
    r.commit_at("local.txt", "local only", 1_600_900_000);
    r.config(IDENTITY_CFG);

    // Default range is upstream..HEAD: only the unpushed commit changes.
    let plan = r.ghma_ok(&["plan"]);
    assert!(plan.contains("1 to rewrite"), "{plan}");

    // Forcing the whole branch hits pushed commits.
    let o = r.ghma(&["plan", "--from", "root"]);
    assert_eq!(Repo::code(&o), 5, "{}", String::from_utf8_lossy(&o.stderr));
    let o = r.ghma(&["apply", "--from", "root"]);
    assert_eq!(Repo::code(&o), 5);
    assert_eq!(r.log().iter().filter(|x| x.an == "Jane Doe").count(), 0);

    r.ghma_ok(&["apply", "--from", "root", "--rewrite-pushed"]);
    assert_eq!(r.log().iter().filter(|x| x.an == "Jane Doe").count(), 4);
}

#[test]
fn default_range_rewrites_only_unpushed_commits() {
    let r = Repo::new();
    let pushed = r.linear(3, 1_600_000_000);
    r.bare_remote();
    r.git(&["push", "-q", "-u", "origin", "main"]);
    r.commit_at("l1.txt", "l1", 1_600_900_000);
    r.commit_at("l2.txt", "l2", 1_600_950_000);
    r.config(IDENTITY_CFG);
    r.ghma_ok(&["apply"]);
    let rows = r.log();
    assert_eq!(rows[2].oid, pushed[2], "pushed commits are untouched");
    assert_eq!(rows[3].an, "Jane Doe");
    assert_eq!(rows[4].an, "Jane Doe");
    r.fsck();
}

#[test]
fn config_errors_exit_2() {
    let r = Repo::new();
    r.linear(1, 1_600_000_000);
    for bad in [
        "version: 1\nlast: 6mo\n",
        "version: 3\n",
        "version: 1\nschedule: {}\n",
        "version: 1\nidentity:\n  - match: {email: a@x}\n    set: {name: B, email: b@x}\n  - match: {email: b@x}\n    set: {name: C, email: c@x}\n",
        "version: 1\ntimezone: Mars/Base\n",
    ] {
        r.config(bad);
        let o = r.ghma(&["plan", "--from", "root"]);
        assert_eq!(
            Repo::code(&o),
            2,
            "{bad}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
    }
}

#[test]
fn trailer_stripping_is_applied_and_idempotent() {
    let r = Repo::new();
    r.commit_at(
        "a.txt",
        "first\n\nbody\n\nSigned-off-by: Old Me <me@home.org>",
        1_600_000_000,
    );
    r.commit_at("b.txt", "second", 1_600_100_000);
    r.config("version: 1\nmessages:\n  strip_trailers: [Signed-off-by]\n");
    r.ghma_ok(&["apply", "--from", "root"]);
    let msg = r.git(&["log", "-1", "--format=%B", "HEAD~1"]);
    assert!(!msg.contains("Signed-off-by"), "{msg}");
    assert!(msg.contains("body"));
    assert!(
        r.ghma_ok(&["apply", "--from", "root"])
            .contains("Nothing to do")
    );
}

#[test]
fn trailers_can_be_swapped_and_the_result_is_stable() {
    let r = Repo::new();
    r.commit_at(
        "a.txt",
        "first\n\nbody\n\nCo-Authored-By: Bot <bot@x>",
        1_600_000_000,
    );
    r.commit_at("b.txt", "second", 1_600_100_000);
    r.config(
        "version: 1\nmessages:\n  strip_trailers: [Co-Authored-By]\n  add_trailers: [\"Assisted-By: Bot <bot@x>\"]\n",
    );
    r.ghma_ok(&["apply", "--from", "root"]);
    let first = r.git(&["log", "-1", "--format=%B", "HEAD~1"]);
    assert!(!first.contains("Co-Authored-By"), "{first}");
    assert!(
        first.contains("body\n\nAssisted-By: Bot <bot@x>"),
        "{first}"
    );
    let second = r.git(&["log", "-1", "--format=%B", "HEAD"]);
    assert!(
        second.trim_end().ends_with("Assisted-By: Bot <bot@x>"),
        "{second}"
    );
    assert!(
        r.ghma_ok(&["apply", "--from", "root"])
            .contains("Nothing to do")
    );
    r.fsck();
}

#[test]
fn a_trailer_both_stripped_and_added_is_a_config_error() {
    let r = Repo::new();
    r.linear(1, 1_600_000_000);
    r.config("version: 1\nmessages:\n  strip_trailers: [Assisted-By]\n  add_trailers: [\"assisted-by: x\"]\n");
    assert_eq!(Repo::code(&r.ghma(&["plan", "--from", "root"])), 2);
    r.config("version: 1\nmessages:\n  add_trailers: [\"not a trailer\"]\n");
    assert_eq!(Repo::code(&r.ghma(&["plan", "--from", "root"])), 2);
}

#[test]
fn all_flag_with_nothing_to_change_is_a_clean_noop() {
    let r = Repo::new();
    r.linear(3, 1_600_000_000);
    r.config("version: 1\n");
    let tip = r.git(&["rev-parse", "HEAD"]);
    let o = r.ghma_ok(&["apply", "--from", "root", "--all"]);
    assert!(o.contains("Nothing to do"), "{o}");
    assert_eq!(r.git(&["rev-parse", "HEAD"]), tip);
    assert!(r.git(&["for-each-ref", "refs/ghma/backup/"]).is_empty());
}

#[test]
fn tags_pointing_into_the_rewrite_are_warned_about() {
    let r = Repo::new();
    r.linear(3, 1_600_000_000);
    r.git(&["tag", "v1", "HEAD~1"]);
    r.git(&["tag", "-a", "-m", "annotated", "v2", "HEAD"]);
    r.config(IDENTITY_CFG);
    let out = r.ghma_ok(&["plan", "--from", "root"]);
    assert!(
        out.contains("refs/tags/v1") && out.contains("refs/tags/v2"),
        "{out}"
    );
}

#[test]
fn init_writes_a_valid_starter_config() {
    let r = Repo::new();
    r.linear(2, 1_600_000_000);
    r.ghma_ok(&["init"]);
    assert_eq!(Repo::code(&r.ghma(&["init"])), 3, "refuses to overwrite");
    r.ghma_ok(&["init", "--force"]);
    let o = r.ghma(&["plan", "--from", "root"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
}

// ---------- LLM workflow (milestone 4) ----------

#[test]
fn llm_export_import_apply_roundtrip() {
    let r = Repo::new();
    r.commit_at(
        "a.txt",
        "wip\n\nSigned-off-by: Old Me <me@home.org>",
        1_600_000_000,
    );
    r.commit_at("b.txt", "fix stuff", 1_600_100_000);
    r.commit_at("c.txt", "more", 1_600_200_000);
    r.config("version: 1\n");
    let old = r.log();
    let plan = r.path().join("plan.json");
    // Everything already conforms, so use --all to make all commits editable.
    r.ghma_ok(&[
        "plan",
        "--from",
        "root",
        "--all",
        "--out",
        plan.to_str().unwrap(),
    ]);

    let exported = r.ghma_ok(&["export", "--plan", plan.to_str().unwrap()]);
    let rows: Vec<serde_json::Value> = exported
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0]["i"], 0);
    assert!(rows[0]["s"].as_str().unwrap().ends_with("1f"));
    assert!(rows[0].get("a").is_none(), "common author is not repeated");
    let batch = r.ghma_ok(&[
        "export",
        "--plan",
        plan.to_str().unwrap(),
        "--batch",
        "2",
        "--offset",
        "1",
    ]);
    assert_eq!(batch.lines().count(), 2);

    // A bad reply changes nothing and exits 7.
    let bad = r.path().join("bad.jsonl");
    std::fs::write(&bad, "Sure, here you go\n{\"i\":0,\"t\":\"x\"}\n").unwrap();
    let o = r.ghma(&[
        "import",
        "--plan",
        plan.to_str().unwrap(),
        bad.to_str().unwrap(),
    ]);
    assert_eq!(Repo::code(&o), 7);

    let reply = r.path().join("reply.jsonl");
    std::fs::write(
        &reply,
        "{\"i\":0,\"t\":\"Add first file\",\"b\":\"Introduces a.txt.\\n\\nSigned-off-by: Old Me <me@home.org>\"}\n{\"i\":1,\"t\":\"Add second file\"}\n",
    )
    .unwrap();
    r.ghma_ok(&[
        "import",
        "--plan",
        plan.to_str().unwrap(),
        reply.to_str().unwrap(),
    ]);
    r.ghma_ok(&["apply", "--plan", plan.to_str().unwrap()]);

    let new = r.log();
    assert_same_shape(&old, &new);
    assert_eq!(new[0].subject, "Add first file");
    assert_eq!(new[1].subject, "Add second file");
    assert_eq!(new[2].subject, "more");
    let full = r.git(&["log", "-1", "--format=%B", "HEAD~2"]);
    assert!(
        full.contains("Introduces a.txt.") && full.contains("Signed-off-by: Old Me"),
        "{full}"
    );
    r.fsck();
    // The edited messages are the new baseline: nothing left to do.
    assert!(
        r.ghma(&["plan", "--check", "--from", "root"])
            .status
            .success()
    );
}

// ---------- signing ----------

fn have_ssh_keygen() -> bool {
    std::process::Command::new("ssh-keygen")
        .arg("-?")
        .output()
        .is_ok()
}

fn setup_ssh_signing(r: &Repo) {
    let key = r.home.path().join("sign_key");
    let o = std::process::Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-f"])
        .arg(&key)
        .output()
        .unwrap();
    assert!(o.status.success());
    r.git(&["config", "gpg.format", "ssh"]);
    r.git(&[
        "config",
        "user.signingkey",
        &format!("{}.pub", key.display()),
    ]);
}

#[test]
fn signing_strip_and_resign() {
    if !have_ssh_keygen() {
        // A silently skipped signing test would be a false green on CI.
        assert!(
            std::env::var_os("CI").is_none(),
            "ssh-keygen is required for the signing test on CI"
        );
        eprintln!("skipping: ssh-keygen not available");
        return;
    }
    let r = Repo::new();
    setup_ssh_signing(&r);
    r.git(&["config", "commit.gpgsign", "true"]);
    r.linear(3, 1_600_000_000);
    r.git(&["config", "commit.gpgsign", "false"]);
    assert!(r.git(&["cat-file", "-p", "HEAD"]).contains("gpgsig"));

    // strip (default): the signed commits are nonconforming and lose the signature.
    r.config("version: 1\n");
    r.ghma_ok(&["apply", "--from", "root"]);
    for row in r.log() {
        assert!(
            !String::from_utf8_lossy(&r.cat(&row.oid)).contains("gpgsig"),
            "signature stripped"
        );
    }
    r.fsck();
    assert!(
        r.ghma_ok(&["apply", "--from", "root"])
            .contains("Nothing to do")
    );

    // resign: unsigned commits are nonconforming and get signed; trees stay.
    let old = r.log();
    r.config("version: 1\nsigning: resign\n");
    r.ghma_ok(&["apply", "--from", "root"]);
    let new = r.log();
    assert_same_content(&old, &new);
    for row in &new {
        assert!(
            String::from_utf8_lossy(&r.cat(&row.oid)).contains("gpgsig"),
            "re-signed"
        );
    }
    r.fsck();
    assert!(
        r.ghma_ok(&["apply", "--from", "root"])
            .contains("Nothing to do"),
        "resign is idempotent"
    );
}

// ---------- hook (milestones 2 and 5) ----------

#[test]
fn hook_verify_blocks_nonconforming_pushes_and_allows_conforming_ones() {
    let r = Repo::new();
    r.bare_remote();
    r.config(IDENTITY_CFG);
    r.linear(3, 1_600_000_000);
    r.ghma_ok(&["hook", "install"]);

    // First push of a new branch: nonconforming -> blocked.
    let o = r.git_out(&["push", "-q", "-u", "origin", "main"]);
    assert!(!o.status.success());
    assert!(
        String::from_utf8_lossy(&o.stderr).contains("ghma apply"),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(
        r.git(&["ls-remote", "origin"]).is_empty(),
        "nothing was pushed"
    );

    r.ghma_ok(&["apply", "--from", "root"]);
    let o = r.git_out(&["push", "-q", "-u", "origin", "main"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));

    // Later: a new commit by the old identity blocks; fixing it (range = upstream..HEAD) unblocks.
    r.commit_at("later.txt", "later", 1_700_000_000);
    let o = r.git_out(&["push", "-q"]);
    assert!(!o.status.success());
    r.ghma_ok(&["apply"]);
    let o = r.git_out(&["push", "-q"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(
        r.git(&["rev-parse", "HEAD"]),
        r.git(&["rev-parse", "origin/main"])
    );
}

#[test]
fn hook_rewrite_mode_rewrites_then_aborts_and_the_retry_succeeds() {
    let r = Repo::new();
    let remote = r.bare_remote();
    r.config(&format!("{IDENTITY_CFG}hook:\n  mode: rewrite\n"));
    r.linear(3, 1_600_000_000);
    r.ghma_ok(&["hook", "install"]);
    let old_tip = r.git(&["rev-parse", "HEAD"]);

    let o = r.git_out(&["push", "-q", "-u", "origin", "main"]);
    assert!(!o.status.success());
    assert!(
        String::from_utf8_lossy(&o.stderr).contains("run `git push` again"),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    let new_tip = r.git(&["rev-parse", "HEAD"]);
    assert_ne!(old_tip, new_tip, "branch was rewritten");
    assert!(r.log().iter().all(|x| x.an == "Jane Doe"));
    assert!(r.git(&["ls-remote", "origin"]).is_empty());

    let o = r.git_out(&["push", "-q", "-u", "origin", "main"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let remote_tip = std::process::Command::new("git")
        .args(["--git-dir"])
        .arg(&remote)
        .args(["rev-parse", "main"])
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&remote_tip.stdout).trim(), new_tip);
    r.fsck();
}

#[test]
fn hook_ignores_deletes_and_other_branches_and_install_is_safe() {
    let r = Repo::new();
    r.bare_remote();
    r.config(IDENTITY_CFG);
    r.commit_as("ok.txt", "ok", 1_600_000_000, "Jane Doe", "jane@work.com");
    r.ghma_ok(&["hook", "install"]);
    r.git(&["push", "-q", "-u", "origin", "main"]);
    // A nonconforming commit on another branch is not judged when pushing main... and a delete is a no-op.
    r.git(&["checkout", "-q", "-b", "other"]);
    r.commit_at("bad.txt", "bad", 1_600_100_000);
    r.git(&["checkout", "-q", "main"]);
    r.git(&["push", "-q", "origin", "other"]);
    let o = r.git_out(&["push", "-q", "origin", "--delete", "other"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));

    // Install refuses to clobber a foreign hook, uninstall refuses to remove one.
    r.ghma_ok(&["hook", "uninstall"]);
    let hook = r.path().join(".git/hooks/pre-push");
    std::fs::write(&hook, "#!/bin/sh\nexit 0\n").unwrap();
    assert_eq!(Repo::code(&r.ghma(&["hook", "install"])), 3);
    assert_eq!(Repo::code(&r.ghma(&["hook", "uninstall"])), 3);
    r.ghma_ok(&["hook", "install", "--force"]);
    r.ghma_ok(&["hook", "uninstall"]);
    assert!(!hook.exists());
}

#[test]
fn hook_ignores_pushes_of_non_tip_commits_and_judges_the_branch() {
    let r = Repo::new();
    r.bare_remote();
    r.config(IDENTITY_CFG);
    r.commit_as("a.txt", "a", 1_600_000_000, "Jane Doe", "jane@work.com");
    r.commit_as("b.txt", "b", 1_600_100_000, "Jane Doe", "jane@work.com");
    r.commit_at("bad.txt", "bad", 1_600_200_000); // nonconforming tip
    r.ghma_ok(&["hook", "install"]);
    // Pushing HEAD~1 to main only sends conforming commits, but the hook only judges when the
    // pushed local ref is the checked-out branch, so `HEAD~1:main` is not judged at all.
    let o = r.git_out(&["push", "-q", "origin", "HEAD~1:refs/heads/main"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    // Pushing the branch itself includes the bad tip and is blocked.
    let o = r.git_out(&["push", "-q", "origin", "main"]);
    assert!(!o.status.success());
}

#[test]
fn hook_judges_pushes_spelled_as_head_or_sha() {
    let r = Repo::new();
    r.bare_remote();
    r.config(IDENTITY_CFG);
    r.commit_at("bad.txt", "bad", 1_600_200_000); // nonconforming tip
    r.ghma_ok(&["hook", "install"]);
    let tip = r.git(&["rev-parse", "HEAD"]);
    let remote_spec = format!("{tip}:refs/heads/o2");
    for spec in [
        "HEAD",
        "HEAD:refs/heads/main",
        "HEAD:refs/heads/other",
        &remote_spec,
    ] {
        let o = r.git_out(&["push", "-q", "origin", spec]);
        assert!(!o.status.success(), "push {spec} must be blocked");
        let err = String::from_utf8_lossy(&o.stderr);
        assert!(
            err.contains("do not follow the ghma rules"),
            "{spec}: {err}"
        );
        assert!(err.contains("--from"), "no-upstream hint missing: {err}");
    }
    assert!(
        r.git_out(&["ls-remote", "origin"]).stdout.is_empty(),
        "nothing may have been pushed"
    );
}

#[test]
fn hook_rewrite_mode_fixes_head_pushes() {
    let r = Repo::new();
    r.bare_remote();
    r.config(&format!("{IDENTITY_CFG}hook: {{mode: rewrite}}\n"));
    r.commit_at("bad.txt", "bad", 1_600_200_000);
    r.ghma_ok(&["hook", "install"]);
    let old_tip = r.git(&["rev-parse", "HEAD"]);
    let o = r.git_out(&["push", "-q", "origin", "HEAD"]);
    assert!(!o.status.success(), "the push is aborted after rewriting");
    assert_ne!(
        r.git(&["rev-parse", "HEAD"]),
        old_tip,
        "branch was rewritten"
    );
    assert_eq!(r.log().last().unwrap().an, "Jane Doe");
    let o = r.git_out(&["push", "-q", "origin", "HEAD"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
}

// ---------- backends ----------

#[test]
fn backend_selection_flag_env_and_config() {
    let r = Repo::new();
    r.linear(3, 1_600_000_000);
    r.config(IDENTITY_CFG);
    // Unknown values are usage errors, from the flag, the env var and the config alike.
    assert_eq!(
        Repo::code(&r.ghma(&["--backend", "bogus", "plan", "--from", "root"])),
        2
    );
    let o = r
        .cmd(bin())
        .args(["plan", "--from", "root"])
        .env("GHMA_BACKEND", "bogus")
        .output()
        .unwrap();
    assert_eq!(Repo::code(&o), 2);
    r.config(&format!("{IDENTITY_CFG}backend: bogus\n"));
    let plain = || {
        r.cmd(bin())
            .args(["plan", "--from", "root"])
            .env_remove("GHMA_BACKEND")
            .output()
            .unwrap()
    };
    assert_eq!(Repo::code(&plain()), 2);
    // `git` always works; `gix` works exactly when it was compiled in.
    r.config(IDENTITY_CFG);
    r.ghma_ok(&["--backend", "git", "plan", "--from", "root"]);
    let gix = r.ghma(&["--backend", "gix", "plan", "--from", "root"]);
    let expected = if cfg!(feature = "gix") { 0 } else { 2 };
    assert_eq!(
        Repo::code(&gix),
        expected,
        "{}",
        String::from_utf8_lossy(&gix.stderr)
    );
    // The flag beats the environment, which beats the config.
    r.config(&format!("{IDENTITY_CFG}backend: gix\n"));
    if !cfg!(feature = "gix") {
        assert_eq!(Repo::code(&plain()), 2);
        r.ghma_ok(&["--backend", "git", "plan", "--from", "root"]);
        let o = r
            .cmd(bin())
            .args(["plan", "--from", "root"])
            .env("GHMA_BACKEND", "git")
            .output()
            .unwrap();
        assert_eq!(Repo::code(&o), 0);
    }
}

#[cfg(feature = "gix")]
#[test]
fn gix_and_git_backends_write_byte_identical_commits() {
    let mk = || {
        let r = Repo::new();
        r.linear(8, 1_600_000_000);
        r.config(&format!(
            "{IDENTITY_CFG}from: 2025-01-01\nto: 2026-01-31\nschedule: {{days: [mon, tue, wed], hours: \"10:00-16:00\", seed: 5}}\n"
        ));
        r
    };
    let (a, b) = (mk(), mk());
    assert_eq!(a.git(&["rev-parse", "HEAD"]), b.git(&["rev-parse", "HEAD"]));
    a.ghma_ok(&["--backend", "git", "apply", "--from", "root"]);
    b.ghma_ok(&["--backend", "gix", "apply", "--from", "root"]);
    let (la, lb) = (a.log(), b.log());
    assert_eq!(la.len(), 8);
    for (x, y) in la.iter().zip(&lb) {
        assert_eq!(x.oid, y.oid, "commit ids must match across backends");
        assert_eq!(a.cat(&x.oid), b.cat(&y.oid));
    }
    b.fsck();
    // A rerun with the other backend is a no-op too.
    assert!(
        b.ghma_ok(&["--backend", "git", "apply", "--from", "root"])
            .contains("Nothing to do")
    );
}
