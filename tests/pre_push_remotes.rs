//! The pre-push hook and a second remote (a mirror, a fork): history some remote already has is not
//! judged by the rules again, but a commit gcma replaced is blocked unless the remote the push goes
//! to has it.

mod common;

use common::*;

fn push_unhooked(r: &Repo, remote: &str, spec: &str) {
    r.git(&["push", "-q", "--no-verify", remote, spec]);
}

#[test]
fn a_mirror_takes_the_published_history_but_not_a_new_nonconforming_commit() {
    let r = Repo::hooked(IDENTITY_CFG);
    r.commit_at("a.txt", "public before the rules", T0);
    r.add_remote("mirror");
    push_unhooked(&r, "mirror", "main"); // the mirror is behind origin
    r.commit_at("b.txt", "public before the rules", T0 + 100);
    r.git(&["push", "-q", "--no-verify", "-u", "origin", "main"]);
    r.add_remote("empty");
    r.push_ok(&["empty", "main"]);
    r.push_ok(&["mirror", "main"]);

    r.commit_at("c.txt", "bad", T0 + 200);
    for remote in ["mirror", "empty"] {
        r.push_blocked(&[remote, "main"], NONCONFORMING);
    }
}

#[test]
fn a_tag_to_a_fork_with_stale_tracking_refs_is_judged_from_what_any_remote_has() {
    let r = Repo::hooked(IDENTITY_CFG);
    r.commit_at("a.txt", "a", T0);
    r.add_remote("fork");
    push_unhooked(&r, "fork", "main"); // fork/main stays at `a`
    r.commit_at("b.txt", "b", T0 + 100);
    r.git(&["push", "-q", "--no-verify", "-u", "origin", "main"]);
    r.git(&["tag", "v1"]);
    r.push_ok(&["fork", "v1"]);
}

#[test]
fn tags_pushed_to_a_new_remote_are_judged_from_what_the_other_remotes_have() {
    let r = Repo::hooked(IDENTITY_CFG);
    r.commit_at("old.txt", "public before the rules", T0);
    r.git(&["tag", "v0"]);
    r.git(&["push", "-q", "--no-verify", "-u", "origin", "main"]);
    r.commit_as("a.txt", "a", T0 + 100, "Jane Doe", "jane@work.com");
    r.git(&["tag", "v1"]);
    r.add_remote("mirror");
    r.push_ok(&["mirror", "v0"]);
    r.push_ok(&["mirror", "v1"]);
    r.commit_at("bad.txt", "bad", T0 + 200);
    r.git(&["tag", "v2"]);
    r.push_blocked(&["mirror", "v2"], NONCONFORMING); // the new commit is still judged
}

#[test]
fn a_replaced_commit_on_origin_is_blocked_on_its_way_to_a_new_remote() {
    let r = Repo::hooked(SECRETS_CFG);
    r.commit_files(&[("a.txt", "a\n")], "a", T0);
    r.commit_files(
        &[("secrets/k", "k\n"), ("k.txt", "k\n")],
        "add key",
        T0 + 100,
    );
    r.git(&["tag", "v1"]);
    r.commit_files(&[("c.txt", "c\n")], "c", T0 + 200);
    r.git(&["push", "-q", "--no-verify", "-u", "origin", "main"]);
    r.gcma_ok(&["apply", "--rewrite-pushed", "--from", "root"]);
    r.add_remote("pub");
    let replaced = "refs/tags/v1 would push commits that gcma replaced";
    r.push_blocked(&["pub", "v1"], replaced);
    r.push_blocked(&["--tags", "pub"], replaced);
    assert!(r.git(&["ls-remote", "pub"]).is_empty());
    // origin has the replaced commits already: the tag sends nothing new there.
    r.push_ok(&["origin", "v1"]);
}
