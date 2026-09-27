//! The pre-push hook: what it blocks, rewrites and leaves alone, and how it installs.

mod common;

use common::*;

#[test]
fn hook_verify_blocks_nonconforming_pushes_and_allows_conforming_ones() {
    let r = Repo::new();
    r.bare_remote();
    r.config(IDENTITY_CFG);
    r.linear(3, 1_600_000_000);
    r.gcma_ok(&["hook", "install"]);

    // First push of a new branch: nonconforming -> blocked.
    let o = r.git_out(&["push", "-q", "-u", "origin", "main"]);
    assert!(!o.status.success());
    assert!(
        String::from_utf8_lossy(&o.stderr).contains("gcma apply"),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(
        r.git(&["ls-remote", "origin"]).is_empty(),
        "nothing was pushed"
    );

    r.gcma_ok(&["apply", "--from", "root"]);
    let o = r.git_out(&["push", "-q", "-u", "origin", "main"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));

    // Later: a new commit by the old identity blocks; fixing it (range = upstream..HEAD) unblocks.
    r.commit_at("later.txt", "later", 1_700_000_000);
    let o = r.git_out(&["push", "-q"]);
    assert!(!o.status.success());
    r.gcma_ok(&["apply"]);
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
    r.gcma_ok(&["hook", "install"]);
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
    assert_eq!(remote_tip(&remote, "main"), new_tip);
    r.fsck();
}

#[test]
fn hook_ignores_deletes_and_other_branches_and_install_is_safe() {
    let r = Repo::new();
    r.bare_remote();
    r.config(IDENTITY_CFG);
    r.commit_as("ok.txt", "ok", 1_600_000_000, "Jane Doe", "jane@work.com");
    r.gcma_ok(&["hook", "install"]);
    r.git(&["push", "-q", "-u", "origin", "main"]);
    // A nonconforming commit on another branch is not judged when pushing main... and a delete is a no-op.
    r.git(&["checkout", "-q", "-b", "other"]);
    r.commit_at("bad.txt", "bad", 1_600_100_000);
    r.git(&["checkout", "-q", "main"]);
    r.git(&["push", "-q", "origin", "other"]);
    let o = r.git_out(&["push", "-q", "origin", "--delete", "other"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));

    // The hook is live all along: a nonconforming commit on the checked-out branch is blocked.
    r.commit_at("bad-main.txt", "bad on main", 1_600_200_000);
    let o = r.git_out(&["push", "-q", "origin", "main"]);
    assert!(
        !o.status.success(),
        "the hook must judge the checked-out branch"
    );
    assert!(String::from_utf8_lossy(&o.stderr).contains("gcma apply"));

    // Install refuses to clobber a foreign hook, uninstall refuses to remove one.
    r.gcma_ok(&["hook", "uninstall"]);
    let hook = r.path().join(".git/hooks/pre-push");
    std::fs::write(&hook, "#!/bin/sh\nexit 0\n").unwrap();
    assert_eq!(Repo::code(&r.gcma(&["hook", "install"])), 3);
    assert_eq!(Repo::code(&r.gcma(&["hook", "uninstall"])), 3);
    r.gcma_ok(&["hook", "install", "--force"]);
    r.gcma_ok(&["hook", "uninstall"]);
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
    r.gcma_ok(&["hook", "install"]);
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
    r.gcma_ok(&["hook", "install"]);
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
            err.contains("do not follow the gcma rules"),
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
    r.gcma_ok(&["hook", "install"]);
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

#[test]
fn uninstalling_without_a_hook_says_so_and_succeeds() {
    let r = Repo::new();
    r.linear(1, 1_600_000_000);
    let out = r.gcma_ok(&["hook", "uninstall"]);
    assert!(out.contains("no hook installed"), "{out}");
    r.gcma_ok(&["hook", "install"]);
    assert!(r.gcma_ok(&["hook", "uninstall"]).contains("hook removed"));
    assert!(
        r.gcma_ok(&["hook", "uninstall"])
            .contains("no hook installed")
    );
}
