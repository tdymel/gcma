//! The backup commands: `restore` lists the backups `apply` leaves behind, restores one by id or
//! prefix, refuses when the branch moved or belongs to another branch, and prunes.

mod common;

use common::*;

#[test]
fn two_rewrites_in_one_second_keep_two_distinct_backups() {
    let r = Repo::new();
    r.linear(3, T0);
    r.config(IDENTITY_CFG);
    r.gcma_ok(&["apply", "--from", "root"]);
    r.config("version: 1\nmessages:\n  add_trailers: [\"Assisted-By: A <a@x>\"]\n");
    r.gcma_ok(&["apply", "--from", "root"]);
    let listing = r.gcma_ok(&["restore"]);
    let ids: Vec<&str> = listing
        .lines()
        .map(|l| l.split_whitespace().next().unwrap())
        .collect();
    assert_eq!(ids.len(), 2, "{listing}");
    assert_ne!(ids[0], ids[1]);
    for l in listing.lines() {
        assert!(
            l.contains("branch main") && l.contains("old ") && l.contains("new "),
            "{l}"
        );
    }
    // Both backups hold their own old tip; the chain restores step by step.
    let tip = r.git(&["rev-parse", "HEAD"]);
    let newest = listing
        .lines()
        .find(|l| l.contains(&format!("new {tip}")))
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_string();
    r.gcma_ok(&["restore", &newest]);
    assert!(r.log().iter().all(|x| x.an == "Jane Doe"));
    assert!(
        !r.messages("HEAD")
            .values()
            .any(|m| m.contains("Assisted-By"))
    );
}

#[test]
fn pruning_a_backup_forgets_it_and_listing_shows_none() {
    let r = Repo::new();
    r.linear(2, T0);
    r.config(IDENTITY_CFG);
    r.gcma_ok(&["apply", "--from", "root"]);
    let id = r.backup_id();
    let o = r.gcma(&["restore", &id, "--prune"]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(stdout(&o).contains("pruned backup"), "{}", stdout(&o));
    assert_eq!(r.gcma_ok(&["restore"]).trim(), "No backups.");
    assert_eq!(r.backup_count(), 0);
    let o = r.gcma(&["restore", &id]);
    assert_eq!(Repo::code(&o), 2, "{}", stderr(&o));
}

#[test]
fn restoring_by_an_unambiguous_prefix_works_and_an_ambiguous_one_is_refused() {
    let r = Repo::new();
    r.linear(2, T0);
    r.config(IDENTITY_CFG);
    r.gcma_ok(&["apply", "--from", "root"]);
    let id = r.backup_id();
    let o = r.gcma(&["restore", &id[..4]]);
    assert!(o.status.success(), "a prefix is enough: {}", stderr(&o));
    let o = r.gcma(&["restore", ""]);
    assert!(
        !o.status.success(),
        "the empty prefix must not match everything silently"
    );
}

#[test]
fn restore_refuses_when_the_branch_moved_unless_forced() {
    let r = Repo::new();
    r.linear(3, T0);
    r.config(IDENTITY_CFG);
    let old_tip = r.git(&["rev-parse", "HEAD"]);
    r.gcma_ok(&["apply", "--from", "root"]);
    let id = r.backup_id();
    // New work after the rewrite (made as the new identity so it conforms).
    r.commit_as(
        "later.txt",
        "later",
        1_700_000_000,
        "Jane Doe",
        "jane@work.com",
    );
    let moved = r.git(&["rev-parse", "HEAD"]);
    let o = r.gcma(&["restore", &id]);
    assert_eq!(Repo::code(&o), 4, "{}", stderr(&o));
    assert_eq!(
        r.git(&["rev-parse", "HEAD"]),
        moved,
        "refused restore must not move the branch"
    );
    r.gcma_ok(&["restore", &id, "--force"]);
    assert_eq!(r.git(&["rev-parse", "HEAD"]), old_tip);
    // The discarded tip keeps a ref, so nothing is lost even after gc.
    let holders = r.git(&["for-each-ref", "--contains", &moved, "refs/gcma/discarded/"]);
    assert!(holders.contains(&moved), "{holders}");
    let o = r.gcma(&["restore", "--prune"]);
    assert_eq!(Repo::code(&o), 2);
}

#[test]
fn restore_after_path_rules_leaves_index_and_gitignore_as_before() {
    let r = Repo::new();
    r.commit_files(&[("a.txt", "a\n")], "add a", T0);
    r.commit_files(
        &[("secrets/k", "k\n"), ("b.txt", "b\n")],
        "add b",
        1_600_100_000,
    );
    r.config(SECRETS_CFG);
    let tip = r.git(&["rev-parse", "HEAD"]);
    r.gcma_ok(&["apply", "--from", "root"]);
    assert!(r.path().join(".gitignore").exists());
    let id = r.backup_id_from_refs();
    r.gcma_ok(&["restore", &id]);
    assert_eq!(r.git(&["rev-parse", "HEAD"]), tip);
    assert_eq!(r.status_without_config(), "", "{}", r.git(&["status"]));
    assert!(
        !r.path().join(".gitignore").exists(),
        "gcma's own .gitignore is gone again"
    );
    assert!(r.path().join("secrets/k").exists());
}

#[test]
fn a_backup_of_another_branch_is_refused_and_leaves_both_branches_alone() {
    let r = Repo::new();
    r.linear(3, T0);
    r.config(IDENTITY_CFG);
    r.gcma_ok(&["apply", "--from", "root"]);
    let id = r.backup_id();
    r.git(&["checkout", "-q", "-b", "other"]);
    let (main, other) = (
        r.git(&["rev-parse", "main"]),
        r.git(&["rev-parse", "other"]),
    );
    let o = r.gcma(&["restore", &id]);
    assert_eq!(Repo::code(&o), 3, "{}", stderr(&o));
    assert!(
        stderr(&o).contains("belongs to branch main"),
        "{}",
        stderr(&o)
    );
    assert_eq!(r.git(&["rev-parse", "main"]), main);
    assert_eq!(r.git(&["rev-parse", "other"]), other);
    // Once the right branch is checked out the same backup restores fine.
    r.git(&["checkout", "-q", "main"]);
    r.gcma_ok(&["restore", &id]);
}
