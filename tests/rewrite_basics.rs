//! The basics of a rewrite: identities change and everything else stays, merges keep their parent
//! order and trees, unrelated roots all stay roots, commits that already conform keep their ids, a
//! second run is a no-op, and `plan --check` reports what is left to do.

mod common;

use common::*;

#[test]
fn identity_rewrite_preserves_everything_else() {
    let r = Repo::new();
    r.linear(6, T0);
    r.config(IDENTITY_CFG);
    let old = r.log();
    let old_tip = old.last().unwrap().oid.clone();

    let plan = r.gcma_ok(&["plan", "--from", "root"]);
    assert!(plan.contains("6 to rewrite"), "{plan}");
    assert_eq!(
        r.git(&["rev-parse", "HEAD"]),
        old_tip,
        "plan must not write"
    );

    let out = r.gcma_ok(&["apply", "--from", "root"]);
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
    let refs = r.git(&["for-each-ref", "refs/gcma/backup/"]);
    assert!(refs.contains("/old") && refs.contains("/new"), "{refs}");
    assert_eq!(r.git(&["rev-list", "--count", &old_tip]), "6");
}

#[test]
fn second_apply_is_a_noop_and_restore_returns_the_original_tip() {
    let r = Repo::new();
    r.linear(4, T0);
    r.config(IDENTITY_CFG);
    let old_tip = r.git(&["rev-parse", "HEAD"]);
    r.gcma_ok(&["apply", "--from", "root"]);
    let new_tip = r.git(&["rev-parse", "HEAD"]);
    assert_ne!(old_tip, new_tip);

    let again = r.gcma_ok(&["apply", "--from", "root"]);
    assert!(again.contains("Nothing to do"), "{again}");
    assert_eq!(r.git(&["rev-parse", "HEAD"]), new_tip);
    assert!(
        r.gcma(&["plan", "--check", "--from", "root"])
            .status
            .success()
    );

    let id = r.backup_id();
    // The branch is still at the recorded new tip: restore works without --force.
    r.gcma_ok(&["restore", &id]);
    assert_eq!(r.git(&["rev-parse", "HEAD"]), old_tip);
    r.fsck();
}

#[test]
fn merge_history_keeps_parent_order_and_trees() {
    let r = Repo::new();
    r.commit_at("base.txt", "base", T0);
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

    r.gcma_ok(&["apply", "--from", "root"]);
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
    let a = r.commit_as("a.txt", "a", T0, "Jane Doe", "jane@work.com");
    let b = r.commit_as("b.txt", "b", 1_600_100_000, "Jane Doe", "jane@work.com");
    r.commit_at("c.txt", "c", 1_600_200_000);
    r.commit_at("d.txt", "d", 1_600_300_000);
    r.config(IDENTITY_CFG);
    let plan = r.gcma_ok(&["plan", "--from", "root"]);
    assert!(plan.contains("2 kept as-is, 2 to rewrite"), "{plan}");
    r.gcma_ok(&["apply", "--from", "root"]);
    let rows = r.log();
    assert_eq!(rows[0].oid, a);
    assert_eq!(rows[1].oid, b);
    assert_eq!(rows[2].an, "Jane Doe");
    assert_eq!(rows[2].parents, vec![b]);
}

// ---------- scheduling (milestone 3) ----------

#[test]
fn plan_check_exits_6_and_changes_nothing() {
    let r = Repo::new();
    r.linear(3, T0);
    r.config(IDENTITY_CFG);
    let tip = r.git(&["rev-parse", "HEAD"]);
    let o = r.gcma(&["plan", "--check", "--from", "root"]);
    assert_eq!(Repo::code(&o), 6, "{}", stderr(&o));
    assert!(stderr(&o).contains("3 commit(s) do not follow"));
    assert_eq!(r.git(&["rev-parse", "HEAD"]), tip);
    assert!(r.git(&["for-each-ref", "refs/gcma/"]).is_empty());
}

#[test]
fn all_flag_with_nothing_to_change_is_a_clean_noop() {
    let r = Repo::new();
    r.linear(3, T0);
    r.config("version: 1\n");
    let tip = r.git(&["rev-parse", "HEAD"]);
    let o = r.gcma_ok(&["apply", "--from", "root", "--all"]);
    assert!(o.contains("Nothing to do"), "{o}");
    assert_eq!(r.git(&["rev-parse", "HEAD"]), tip);
    assert_eq!(r.backup_count(), 0);
}

#[test]
fn unrelated_roots_are_all_rewritten_and_stay_roots() {
    let r = Repo::new();
    r.commit_at("a.txt", "first root", T0);
    r.commit_at("b.txt", "after the first root", T0 + 1000);
    r.git(&["checkout", "-q", "--orphan", "other"]);
    r.git(&["rm", "-rfq", "."]);
    r.commit_at("o.txt", "second root", T0 + 2000);
    r.git(&["checkout", "-q", "main"]);
    r.git(&[
        "merge",
        "-q",
        "--allow-unrelated-histories",
        "-m",
        "join",
        "other",
    ]);
    r.commit_at("z.txt", "after the join", T0 + 3000);
    let roots = |rev: &str| r.git(&["rev-list", "--max-parents=0", rev]).lines().count();
    assert_eq!(roots("HEAD"), 2);
    r.config(&berlin_cfg(IDENTITY_RULE));
    let old = r.log();
    r.gcma_ok(&["apply", "--from", "root"]);
    let new = r.log();
    assert_same_content(&old, &new);
    assert_scheduled(&new);
    assert_eq!(roots("HEAD"), 2, "both roots survive");
    assert!(new.iter().all(|x| x.an == "Jane Doe"));
    r.fsck();
    assert!(
        r.gcma_ok(&["apply", "--from", "root"])
            .contains("Nothing to do")
    );
    let id = r.backup_id();
    r.gcma_ok(&["restore", &id]);
    assert_same_content(&old, &r.log());
}
