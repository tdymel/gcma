//! The pre-push hook and the commits a gcma rewrite replaced: a tag or other ref that would send
//! them is blocked, whatever is checked out; commits a rewrite kept, or that a branch still holds,
//! are not replaced.

mod common;

use common::*;

/// `a` pushed, then the secret (tagged `v1`) and `b` local, and the branch rewritten by `apply`.
fn tag_left_on_a_replaced_commit() -> Repo {
    let r = Repo::hooked(SECRETS_CFG);
    r.commit_files(&[("a.txt", "a\n")], "a", T0);
    r.git(&["push", "-q", "-u", "origin", "main"]);
    r.commit_files(
        &[("secrets/k", "k\n"), ("k.txt", "k\n")],
        "add key",
        T0 + 100,
    );
    r.git(&["tag", "-a", "v1", "-m", "release"]);
    r.commit_files(&[("b.txt", "b\n")], "b", T0 + 200);
    r.gcma_ok(&["apply"]);
    r
}

/// What the hook says when `v1` would send a replaced commit, with how to move it.
const V1_REPLACED: &str = "refs/tags/v1 would push commits that gcma replaced when it rewrote a \
                           branch; move the tag (`git tag -f v1 <new commit>`)";

#[test]
fn a_tag_left_on_a_replaced_commit_is_blocked_until_it_is_moved() {
    let r = tag_left_on_a_replaced_commit();
    r.push_blocked(&["origin", "v1"], V1_REPLACED);
    r.push_blocked(&["--tags", "origin"], V1_REPLACED);
    r.git(&["tag", "-f", "-a", "v1", "-m", "release", "HEAD~1"]);
    r.push_ok(&["origin", "v1"]);
    let remote = r.git(&["ls-remote", "origin", "refs/tags/v1^{}"]);
    assert!(
        remote.starts_with(&r.git(&["rev-parse", "HEAD~1"])),
        "{remote}"
    );
}

#[test]
fn a_replaced_commit_already_on_the_remote_passes() {
    let r = tag_left_on_a_replaced_commit();
    let o = r.git_out(&[
        "push",
        "-q",
        "--no-verify",
        "origin",
        "v1^{}:refs/heads/old",
    ]);
    assert!(o.status.success(), "{}", stderr(&o));
    r.push_ok(&["origin", "v1"]);
}

#[test]
fn a_tag_unrelated_to_the_backups_passes() {
    let r = tag_left_on_a_replaced_commit();
    r.git(&["checkout", "-q", "-b", "other", "origin/main"]);
    r.commit_files(&[("c.txt", "c\n")], "c", T0 + 300);
    r.git(&["tag", "elsewhere"]);
    r.git(&["checkout", "-q", "main"]);
    r.push_ok(&["origin", "elsewhere"]);
}

#[test]
fn rewrite_mode_blocks_the_tags_its_rewrite_left_behind() {
    let r = Repo::hooked(&format!("{SECRETS_CFG}hook: {{mode: rewrite}}\n"));
    r.commit_files(&[("a.txt", "a\n")], "a", T0);
    r.git(&["push", "-q", "-u", "origin", "main"]);
    r.commit_files(
        &[("secrets/k", "k\n"), ("k.txt", "k\n")],
        "add key",
        T0 + 100,
    );
    r.git(&["tag", "v1"]);
    r.push_blocked(&["origin", "main"], "run `git push` again");
    r.push_blocked(&["--tags", "origin"], V1_REPLACED);
    r.push_ok(&["origin", "main"]);
}

#[test]
fn a_tag_on_commits_a_rewrite_kept_passes() {
    let r = Repo::hooked(IDENTITY_CFG);
    r.commit_as("a.txt", "a", T0, "Jane Doe", "jane@work.com");
    r.git(&["push", "-q", "-u", "origin", "main"]);
    r.git(&["checkout", "-q", "-b", "feature"]);
    r.commit_as("b.txt", "b", T0 + 100, "Jane Doe", "jane@work.com"); // kept by the rewrite
    r.commit_at("bad.txt", "bad", T0 + 200);
    r.gcma_ok(&["apply", "--from", "main"]);
    r.git(&["tag", "ft", "feature"]);
    r.git(&["checkout", "-q", "main"]);
    r.push_ok(&["origin", "ft"]);
}

#[test]
fn a_commit_a_backup_replaced_but_a_branch_still_holds_passes() {
    let r = Repo::hooked_post_commit(&berlin_cfg("hook:\n  mode: rewrite\n"));
    let commit = |name: &str| {
        r.write(name, "x\n");
        r.git(&["add", name]);
        r.git(&["commit", "-q", "-m", name]);
    };
    commit("base.txt");
    r.git(&["push", "-q", "-u", "origin", "main"]);
    commit("a.txt");
    r.git(&["tag", "vA"]);
    r.git(&["checkout", "-q", "-b", "feature"]);
    commit("b.txt"); // feature's backup keeps the old `b`, whose parent is main's tip
    r.git(&["checkout", "-q", "-b", "other", "origin/main"]);
    r.push_ok(&["origin", "vA"]);
}

#[test]
fn a_tag_on_a_branch_brought_back_by_restore_passes() {
    let r = tag_left_on_a_replaced_commit();
    r.gcma_ok(&["restore", &r.backup_id()]);
    r.git(&["checkout", "-q", "-b", "other", "origin/main"]);
    r.push_ok(&["origin", "v1"]);
}

#[test]
fn a_detached_head_does_not_skip_the_replaced_commits() {
    let r = tag_left_on_a_replaced_commit();
    r.git(&["checkout", "-q", "--detach", "v1"]);
    r.push_blocked(&["origin", "v1"], V1_REPLACED);
    r.push_blocked(
        &["origin", "HEAD:refs/heads/old"],
        "HEAD would push commits that gcma replaced",
    );
}
