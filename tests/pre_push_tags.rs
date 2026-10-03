//! The pre-push hook and pushes of tags (and other refs that are not branches): a ref whose commit
//! is part of the checked-out branch is judged like the branch, one whose commit is not is ignored.

mod common;

use common::*;

fn hooked(cfg: &str) -> Repo {
    let r = Repo::new();
    r.hooked(cfg);
    r
}

fn assert_blocked(r: &Repo, spec: &str) {
    let o = r.git_out(&["push", "-q", "origin", spec]);
    assert!(!o.status.success(), "push {spec} must be blocked");
    let err = stderr(&o);
    assert!(
        err.contains("do not follow the gcma rules"),
        "{spec}: {err}"
    );
    assert!(
        r.git(&["ls-remote", "origin"]).is_empty(),
        "push {spec}: nothing may reach the remote"
    );
}

fn assert_passes(r: &Repo, spec: &str) {
    let o = r.git_out(&["push", "-q", "origin", spec]);
    assert!(o.status.success(), "push {spec}: {}", stderr(&o));
}

#[test]
fn a_lightweight_tag_below_the_tip_is_judged() {
    let r = hooked(SECRETS_CFG);
    r.commit_files(&[("a.txt", "a\n")], "a", T0);
    r.commit_files(&[("secrets/k2", "k\n")], "add key", T0 + 100);
    r.git(&["tag", "v2"]);
    r.commit_files(&[("b.txt", "b\n")], "b", T0 + 200);
    assert_blocked(&r, "v2");
    assert_blocked(&r, "v2:refs/heads/main");
    assert_blocked(&r, "refs/tags/v2:refs/tags/v2");
}

#[test]
fn an_annotated_tag_at_the_tip_is_judged() {
    let r = hooked(IDENTITY_CFG);
    r.commit_at("bad.txt", "bad", T0);
    r.git(&["tag", "-a", "v1", "-m", "release"]);
    assert_blocked(&r, "v1");
    assert_blocked(&r, "v1:refs/heads/main");
}

#[test]
fn rewrite_mode_blocks_a_tag_instead_of_rewriting_the_branch_under_it() {
    let r = hooked(&format!("{IDENTITY_CFG}hook: {{mode: rewrite}}\n"));
    r.commit_at("bad.txt", "bad", T0);
    r.git(&["tag", "-a", "v1", "-m", "release"]);
    let tip = r.git(&["rev-parse", "HEAD"]);
    assert_blocked(&r, "v1");
    assert_eq!(
        r.git(&["rev-parse", "HEAD"]),
        tip,
        "rewriting the branch would not fix the tag"
    );
}

#[test]
fn a_tag_of_a_conforming_history_is_pushed() {
    let r = hooked(IDENTITY_CFG);
    r.commit_as("a.txt", "a", T0, "Jane Doe", "jane@work.com");
    r.git(&["tag", "-a", "v1", "-m", "release"]);
    r.git(&["tag", "light"]);
    r.commit_at("bad.txt", "bad", T0 + 100); // nonconforming, but above the tags
    assert_passes(&r, "v1");
    assert_passes(&r, "light");
    assert!(r.git(&["ls-remote", "origin"]).contains("refs/tags/v1"));
}

#[test]
fn a_tag_unrelated_to_the_branch_or_not_on_a_commit_is_ignored() {
    let r = hooked(IDENTITY_CFG);
    r.commit_as("a.txt", "a", T0, "Jane Doe", "jane@work.com");
    r.git(&["checkout", "-q", "-b", "other"]);
    r.commit_at("bad.txt", "bad", T0 + 100);
    r.git(&["tag", "elsewhere"]);
    r.git(&["checkout", "-q", "main"]);
    r.git(&["tag", "tree", "HEAD^{tree}"]);
    assert_passes(&r, "elsewhere");
    assert_passes(&r, "tree");
}

#[test]
fn rewrite_mode_reports_the_tags_it_leaves_on_the_old_commits() {
    let r = hooked(&format!("{IDENTITY_CFG}hook: {{mode: rewrite}}\n"));
    r.commit_at("bad.txt", "bad", T0);
    r.git(&["tag", "v1"]);
    let o = r.git_out(&["push", "-q", "-u", "origin", "main"]);
    assert!(!o.status.success());
    let err = stderr(&o);
    assert!(err.contains("run `git push` again"), "{err}");
    assert!(
        err.contains("gcma: pre-push: tags/notes point at commits that will be rewritten"),
        "{err}"
    );
    assert!(err.contains(": v1"), "{err}");
    assert!(!err.contains("--retag"), "a hook has no --retag: {err}");
}

/// A second bare remote, `name`, with no remote-tracking refs yet.
fn new_remote(r: &Repo, name: &str) {
    let path = r.home.path().join(format!("{name}.git"));
    let path = path.to_str().unwrap();
    r.git(&["init", "-q", "--bare", path]);
    r.git(&["remote", "add", name, path]);
}

#[test]
fn tags_pushed_to_a_new_remote_are_judged_from_what_the_other_remotes_have() {
    let r = hooked(IDENTITY_CFG);
    r.commit_at("old.txt", "public before the rules", T0);
    r.git(&["tag", "v0"]);
    r.git(&["push", "-q", "--no-verify", "-u", "origin", "main"]);
    r.commit_as("a.txt", "a", T0 + 100, "Jane Doe", "jane@work.com");
    r.git(&["tag", "v1"]);
    new_remote(&r, "mirror");
    for tag in ["v0", "v1"] {
        let o = r.git_out(&["push", "-q", "mirror", tag]);
        assert!(o.status.success(), "push {tag}: {}", stderr(&o));
    }
    r.commit_at("bad.txt", "bad", T0 + 200);
    r.git(&["tag", "v2"]);
    let o = r.git_out(&["push", "-q", "mirror", "v2"]);
    assert!(!o.status.success(), "the new commit is still judged");
}

#[test]
fn many_tags_are_judged_with_one_plan() {
    let r = hooked(IDENTITY_CFG);
    for i in 0..30 {
        let name = format!("f{i}.txt");
        r.commit_as(&name, &name, T0 + i * 100, "Jane Doe", "jane@work.com");
        r.git(&["tag", &format!("v{i}")]);
    }
    new_remote(&r, "mirror");
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
