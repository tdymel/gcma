//! Histories that are not the plain linear case: unrelated roots, and the warnings `plan` and
//! `apply` print about tags that a rewrite cannot carry along.

mod common;

use common::*;

#[test]
fn apply_warns_about_tags_that_will_keep_pointing_at_the_old_commits() {
    let r = Repo::new();
    r.linear(3, T0);
    let old_middle = r.git(&["rev-parse", "HEAD~1"]);
    r.git(&["tag", "v1", "HEAD~1"]);
    r.config(IDENTITY_CFG);
    let o = r.gcma(&["apply", "--from", "root"]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(
        stderr(&o).contains("warning: tags/notes point at"),
        "{}",
        stderr(&o)
    );
    assert!(stderr(&o).contains("refs/tags/v1"), "{}", stderr(&o));
    assert_eq!(
        r.git(&["rev-parse", "v1"]),
        old_middle,
        "the tag stays where it was"
    );
}

#[test]
fn tags_pointing_into_the_rewrite_are_warned_about() {
    let r = Repo::new();
    r.linear(3, T0);
    r.git(&["tag", "v1", "HEAD~1"]);
    r.git(&["tag", "-a", "-m", "annotated", "v2", "HEAD"]);
    r.config(IDENTITY_CFG);
    let out = r.gcma_ok(&["plan", "--from", "root"]);
    assert!(
        out.contains("refs/tags/v1") && out.contains("refs/tags/v2"),
        "{out}"
    );
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
