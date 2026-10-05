//! The pre-push hook and pushes of tags (and other refs that are not branches): a ref whose commit
//! is part of the checked-out branch is judged like the branch, one whose commit is not is ignored.

mod common;

use common::*;

#[test]
fn a_lightweight_tag_below_the_tip_is_judged() {
    let r = Repo::hooked(SECRETS_CFG);
    r.commit_files(&[("a.txt", "a\n")], "a", T0);
    r.commit_files(&[("secrets/k2", "k\n")], "add key", T0 + 100);
    r.git(&["tag", "v2"]);
    r.commit_files(&[("b.txt", "b\n")], "b", T0 + 200);
    r.push_blocked(&["origin", "v2"], NONCONFORMING);
    r.push_blocked(&["origin", "v2:refs/heads/main"], NONCONFORMING);
    r.push_blocked(&["origin", "refs/tags/v2:refs/tags/v2"], NONCONFORMING);
}

#[test]
fn an_annotated_tag_at_the_tip_is_judged() {
    let r = Repo::hooked(IDENTITY_CFG);
    r.commit_at("bad.txt", "bad", T0);
    r.git(&["tag", "-a", "v1", "-m", "release"]);
    // Only `--retag` takes the tag along to the rewritten commit.
    r.push_blocked(&["origin", "v1"], "run `gcma apply --retag --from <rev>`");
    r.push_blocked(&["origin", "v1:refs/heads/main"], NONCONFORMING);
    let branch = r.push_blocked(&["origin", "main"], NONCONFORMING);
    assert!(!branch.contains("--retag"), "{branch}");
}

#[test]
fn rewrite_mode_blocks_a_tag_instead_of_rewriting_the_branch_under_it() {
    let r = Repo::hooked(&format!("{IDENTITY_CFG}hook: {{mode: rewrite}}\n"));
    r.commit_at("bad.txt", "bad", T0);
    r.git(&["tag", "-a", "v1", "-m", "release"]);
    let tip = r.git(&["rev-parse", "HEAD"]);
    r.push_blocked(&["origin", "v1"], NONCONFORMING);
    assert_eq!(
        r.git(&["rev-parse", "HEAD"]),
        tip,
        "rewriting the branch would not fix the tag"
    );
}

#[test]
fn a_tag_of_a_conforming_history_is_pushed() {
    let r = Repo::hooked(IDENTITY_CFG);
    r.commit_as("a.txt", "a", T0, "Jane Doe", "jane@work.com");
    r.git(&["tag", "-a", "v1", "-m", "release"]);
    r.git(&["tag", "light"]);
    r.commit_at("bad.txt", "bad", T0 + 100); // nonconforming, but above the tags
    r.push_ok(&["origin", "v1"]);
    r.push_ok(&["origin", "light"]);
    assert!(r.git(&["ls-remote", "origin"]).contains("refs/tags/v1"));
}

#[test]
fn a_tag_unrelated_to_the_branch_or_not_on_a_commit_is_ignored() {
    let r = Repo::hooked(IDENTITY_CFG);
    r.commit_as("a.txt", "a", T0, "Jane Doe", "jane@work.com");
    r.git(&["checkout", "-q", "-b", "other"]);
    r.commit_at("bad.txt", "bad", T0 + 100);
    r.git(&["tag", "elsewhere"]);
    r.git(&["checkout", "-q", "main"]);
    r.git(&["tag", "tree", "HEAD^{tree}"]);
    r.push_ok(&["origin", "elsewhere"]);
    r.push_ok(&["origin", "tree"]);
}

#[test]
fn another_branch_or_ref_at_the_tip_is_judged_and_has_to_be_moved_by_hand() {
    let r = Repo::hooked(IDENTITY_CFG);
    r.commit_at("bad.txt", "bad", T0);
    r.git(&["branch", "feature"]);
    r.git(&["update-ref", "refs/keep/x", "HEAD"]);
    for (spec, how) in [
        ("feature", "`git branch -f feature <new commit>`"),
        (
            "refs/keep/x:refs/keep/x",
            "`git update-ref refs/keep/x <new commit>`",
        ),
    ] {
        let err = r.push_blocked(&["origin", spec], NONCONFORMING);
        assert!(!err.contains("--retag"), "--retag moves only tags: {err}");
        assert!(err.contains("run `gcma apply --from <rev>`"), "{err}");
        assert!(err.contains(how), "{spec}: {err}");
    }
    // With a tag in the same push, the other ref still has to be moved by hand.
    r.git(&["tag", "v1"]);
    r.push_blocked(&["origin", "v1", "feature"], "`git branch -f feature");
}

#[test]
fn rewrite_mode_reports_the_tags_it_leaves_on_the_old_commits() {
    let r = Repo::hooked(&format!("{IDENTITY_CFG}hook: {{mode: rewrite}}\n"));
    r.commit_at("bad.txt", "bad", T0);
    r.git(&["tag", "v1"]);
    let err = r.push_blocked(&["-u", "origin", "main"], "run `git push` again");
    assert!(
        err.contains("gcma: pre-push: tags/notes point at commits that will be rewritten"),
        "{err}"
    );
    assert!(err.contains(": v1"), "{err}");
    assert!(!err.contains("--retag"), "a hook has no --retag: {err}");
}

#[test]
fn many_tags_are_judged_with_one_plan() {
    let r = Repo::hooked(IDENTITY_CFG);
    for i in 0..30 {
        let name = format!("f{i}.txt");
        r.commit_as(&name, &name, T0 + i * 100, "Jane Doe", "jane@work.com");
        r.git(&["tag", &format!("v{i}")]);
    }
    r.add_remote("mirror");
    let trace = r.home.path().join("trace");
    let o = r
        .cmd("git")
        .env("GIT_TRACE", &trace)
        .args(["push", "-q", "--tags", "mirror"])
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", stderr(&o));
    let trace = std::fs::read_to_string(trace).unwrap();
    let ranges = trace.matches("rev-list --topo-order").count();
    assert!(ranges <= 2, "{ranges} ranges listed for 30 tags");
}

/// `main` pushed, then a nonconforming commit on it, and `make` (a branch or tag at the tip).
fn pushed_main_then_bad_tip(cfg: &str, make: &[&str]) -> Repo {
    let r = Repo::hooked(cfg);
    r.commit_as("a.txt", "a", T0, "Jane Doe", "jane@work.com");
    r.push_ok(&["-u", "origin", "main"]);
    r.commit_at("bad.txt", "bad", T0 + 100);
    r.git(make);
    r
}

const NEW_BRANCH: &[&str] = &["branch", "rel"];
const NEW_TAG: &[&str] = &["tag", "-a", "v4", "-m", "release"];
const MOVE_REL: &str = "`git branch -f rel <new commit>`";
const RETAG: &str = "run `gcma apply --retag`";

#[test]
fn another_branch_pushed_with_the_pushed_branch_has_to_be_moved_by_hand() {
    let r = pushed_main_then_bad_tip(IDENTITY_CFG, NEW_BRANCH);
    r.push_blocked(&["origin", "main", "rel"], MOVE_REL);
    r.gcma_ok(&["apply"]);
    r.git(&["branch", "-f", "rel", "main"]);
    r.push_ok(&["origin", "main", "rel"]);
    assert_eq!(
        remote_tip(&r.remote(), "rel"),
        r.git(&["rev-parse", "HEAD"])
    );
}

#[test]
fn a_tag_pushed_with_the_pushed_branch_needs_retag() {
    let r = pushed_main_then_bad_tip(IDENTITY_CFG, NEW_TAG);
    r.push_blocked(&["origin", "main", "v4"], RETAG);
    r.push_blocked(&["--follow-tags", "origin", "main"], RETAG);
}

#[test]
fn rewrite_mode_blocks_another_branch_or_a_tag_pushed_with_the_pushed_branch() {
    let cfg = format!("{IDENTITY_CFG}hook: {{mode: rewrite}}\n");
    let cases: [(&[&str], &[&str], &str); 3] = [
        (NEW_BRANCH, &["origin", "main", "rel"], MOVE_REL),
        (NEW_TAG, &["origin", "main", "v4"], RETAG),
        (NEW_TAG, &["--follow-tags", "origin", "main"], RETAG),
    ];
    for (make, push, expected) in cases {
        let r = pushed_main_then_bad_tip(&cfg, make);
        let tip = r.git(&["rev-parse", "HEAD"]);
        r.push_blocked(push, expected);
        assert_eq!(
            r.git(&["rev-parse", "HEAD"]),
            tip,
            "{push:?}: rewriting the branch would leave the other ref behind"
        );
    }
}

#[test]
fn rewrite_mode_rewrites_a_first_push_whose_new_tag_is_below_the_rewritten_commits() {
    for backend in BACKENDS {
        let cfg = format!("{IDENTITY_CFG}hook: {{mode: rewrite}}\nbackend: {backend}\n");
        let r = Repo::hooked(&cfg);
        let a = r.commit_as("a.txt", "a", T0, "Jane Doe", "jane@work.com");
        r.git(&["tag", "v0"]);
        r.commit_at("bad.txt", "bad", T0 + 100);
        let tip = r.git(&["rev-parse", "HEAD"]);
        // The tag is on a commit the rewrite keeps: it needs no `--retag`.
        let err = r.push_blocked(&["-u", "origin", "main", "v0"], "run `git push` again");
        assert!(!err.contains("--retag"), "{backend}: {err}");
        assert_ne!(r.git(&["rev-parse", "HEAD"]), tip, "{backend}");
        assert_eq!(r.git(&["rev-parse", "v0"]), a, "{backend}");
        r.push_ok(&["-u", "origin", "main", "v0"]);
    }
}
