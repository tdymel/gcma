//! The pre-push hook and a second remote (a mirror, a fork, a peer): the rules leave out only what
//! the remote pushed to has (its tracking refs) and what the branch's remote-tracking upstream has,
//! so commits taken from another remote are judged on their way to this one. A commit gcma replaced
//! is blocked unless the remote the push goes to has it.

mod common;

use common::*;

fn push_unhooked(r: &Repo, remote: &str, spec: &str) {
    r.git(&["push", "-q", "--no-verify", remote, spec]);
}

#[test]
fn a_mirror_takes_what_the_upstream_has_but_not_a_new_nonconforming_commit() {
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
fn a_tag_to_a_fork_with_stale_tracking_refs_is_judged_from_what_the_upstream_has() {
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
fn a_new_branch_to_a_fork_is_judged_from_its_upstream() {
    let r = Repo::hooked(IDENTITY_CFG);
    r.commit_at("old.txt", "public before the rules", T0);
    r.git(&["push", "-q", "--no-verify", "-u", "origin", "main"]);
    r.git(&["checkout", "-q", "-b", "feat", "--track", "origin/main"]);
    r.commit_as("a.txt", "a", T0 + 100, "Jane Doe", "jane@work.com");
    r.add_remote("fork");
    r.push_ok(&["-u", "fork", "feat"]);
    r.commit_at("bad.txt", "bad", T0 + 200);
    r.push_blocked(&["fork", "feat"], NONCONFORMING);
}

#[test]
fn tags_pushed_to_a_new_remote_are_judged_from_what_the_upstream_has() {
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
fn history_only_another_remote_has_is_judged_on_its_way_to_a_new_one() {
    for mode in ["verify", "rewrite"] {
        let r = Repo::hooked(&format!("{IDENTITY_CFG}hook: {{mode: {mode}}}\n"));
        r.commit_at("old.txt", "public before the rules", T0);
        r.git(&["tag", "v0"]);
        push_unhooked(&r, "origin", "main"); // no upstream: origin/main is just another remote's ref
        r.add_remote("mirror");
        let tip = r.git(&["rev-parse", "HEAD"]);
        // `gcma apply` would refuse them without `--rewrite-pushed`, which a hook cannot pass.
        r.push_blocked(
            &["mirror", "main"],
            "they are on another remote already: if mirror may have them as they are, push with \
             `--no-verify`; otherwise rewrite them with `gcma apply --from <rev> \
             --rewrite-pushed` (the branch has no upstream;",
        );
        r.push_blocked(&["mirror", "v0"], "they are on another remote already");
        assert_eq!(r.git(&["rev-parse", "HEAD"]), tip, "{mode}");
        r.push_ok(&["origin", "main"]); // origin has it
        r.push_ok(&["origin", "v0"]);
    }
}

#[test]
fn no_verify_is_offered_only_when_every_blocked_commit_is_on_a_remote() {
    for backend in BACKENDS {
        let r = Repo::hooked(&format!("{SECRETS_CFG}backend: {backend}\n"));
        r.commit_files(&[("a.txt", "a\n")], "a", T0);
        r.push_ok(&["-u", "origin", "main"]);
        r.commit_files(&[("secrets/old", "o\n")], "old key", T0 + 100);
        r.add_remote("fork");
        push_unhooked(&r, "fork", "main"); // only the fork has it
        r.commit_files(&[("secrets/k", "k\n")], "add key", T0 + 200); // on no remote
        let err = r.push_blocked(
            &["origin", "main"],
            "2 commit(s) about to be pushed do not follow the gcma rules; 1 of them are on \
             another remote already: rewrite them with `gcma apply --rewrite-pushed`",
        );
        assert!(
            !err.contains("--no-verify"),
            "{backend}: it would upload the key: {err}"
        );
    }
}

#[test]
fn commits_taken_from_a_peer_are_judged_on_their_way_to_origin() {
    let r = Repo::hooked(IDENTITY_CFG);
    r.commit_as("a.txt", "a", T0, "Jane Doe", "jane@work.com");
    r.push_ok(&["-u", "origin", "main"]);
    r.add_remote("peer");
    r.commit_at("bad.txt", "bad", T0 + 100);
    push_unhooked(&r, "peer", "main"); // as if fetched from the peer: peer/main has it
    r.push_blocked(&["origin", "main"], NONCONFORMING);
}

#[test]
fn a_local_upstream_does_not_hide_its_unpushed_commits() {
    let r = Repo::hooked(IDENTITY_CFG);
    r.commit_as("a.txt", "a", T0, "Jane Doe", "jane@work.com");
    r.push_ok(&["-u", "origin", "main"]);
    r.commit_at("bad.txt", "bad", T0 + 100); // on main, not pushed
    r.git(&["checkout", "-q", "-b", "feat", "--track", "main"]);
    r.commit_as("b.txt", "b", T0 + 200, "Jane Doe", "jane@work.com");
    // `gcma apply` starts at the upstream, which has the commit too, and refuses to rewrite it
    // there without `--rewrite-pushed`: the hook says to fix the upstream first.
    r.push_blocked(
        &["origin", "feat"],
        "1 commit(s) about to be pushed do not follow the gcma rules; the upstream is a local \
         branch that has some of them too: run `gcma apply` on it and rebase this branch onto it, \
         or run `gcma apply --from <rev> --rewrite-pushed`",
    );
}

#[test]
fn rewrite_mode_leaves_what_a_local_upstream_has_too_to_gcma_apply() {
    let r = Repo::hooked(&format!("{IDENTITY_CFG}hook: {{mode: rewrite}}\n"));
    r.commit_as("a.txt", "a", T0, "Jane Doe", "jane@work.com");
    r.push_ok(&["-u", "origin", "main"]);
    r.commit_at("bad.txt", "bad", T0 + 100); // on main, not pushed
    r.git(&["checkout", "-q", "--track", "-b", "topic", "main"]);
    r.commit_at("bad2.txt", "bad too", T0 + 200);
    let tip = r.git(&["rev-parse", "HEAD"]);
    r.push_blocked(
        &["-u", "origin", "topic"],
        "run `gcma apply` on it and rebase this branch onto it, or run `gcma apply --from <rev> \
         --rewrite-pushed`",
    );
    assert_eq!(r.git(&["rev-parse", "HEAD"]), tip);
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
