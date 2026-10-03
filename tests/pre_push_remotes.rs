//! The pre-push hook and a second remote (a mirror, a fork): history some remote already has is not
//! judged by the rules again, but a commit gcma replaced is blocked unless the remote the push goes
//! to has it.

mod common;

use common::*;

fn assert_passes(r: &Repo, args: &[&str]) {
    let o = r.git_out(args);
    assert!(o.status.success(), "{args:?}: {}", stderr(&o));
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
    for args in [
        &["push", "-q", "pub", "v1"][..],
        &["push", "-q", "--tags", "pub"],
    ] {
        let o = r.git_out(args);
        assert!(!o.status.success(), "{args:?} must be blocked");
        let err = stderr(&o);
        assert!(
            err.contains("refs/tags/v1 would push commits that gcma replaced"),
            "{args:?}: {err}"
        );
        assert!(r.git(&["ls-remote", "pub"]).is_empty(), "{args:?}");
    }
    // origin has the replaced commits already: the tag sends nothing new there.
    assert_passes(&r, &["push", "-q", "origin", "v1"]);
}
