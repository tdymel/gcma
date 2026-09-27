//! The pre-push hook: what it blocks, rewrites and leaves alone, how it installs, and how it
//! behaves next to path rules, a broken config, a custom hooks path and the different ways a push
//! can be spelled.

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

// ---------- the hook next to path rules, config errors and other push shapes ----------

fn hook_repo(cfg: &str) -> (Repo, std::path::PathBuf) {
    let r = Repo::new();
    let remote = r.bare_remote();
    r.config(cfg);
    r.gcma_ok(&["hook", "install"]);
    (r, remote)
}

#[test]
fn the_hook_in_verify_mode_blocks_secrets_until_the_history_is_cleaned() {
    let (r, remote) = hook_repo(SECRETS_CFG);
    r.commit_files(
        &[("src/a.rs", "a\n"), ("secrets/key.pem", "k\n")],
        "feature",
        1_600_000_000,
    );
    let o = r.git_out(&["push", "-q", "-u", "origin", "main"]);
    assert!(!o.status.success());
    assert!(
        stderr(&o).contains("do not follow the gcma rules"),
        "{}",
        stderr(&o)
    );
    assert!(
        r.git(&["ls-remote", "origin"]).is_empty(),
        "nothing reached the remote"
    );

    r.gcma_ok(&["apply", "--from", "root"]);
    let o = r.git_out(&["push", "-q", "-u", "origin", "main"]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(!remote_files(&remote).contains("secrets/"));
    assert!(remote_files(&remote).contains("src/a.rs"));
}

#[test]
fn the_hook_in_rewrite_mode_removes_secrets_aborts_and_the_retry_succeeds() {
    let (r, remote) = hook_repo(&format!("{SECRETS_CFG}hook:\n  mode: rewrite\n"));
    r.commit_files(
        &[("src/a.rs", "a\n"), ("secrets/key.pem", "k\n")],
        "feature",
        1_600_000_000,
    );
    let o = r.git_out(&["push", "-q", "-u", "origin", "main"]);
    assert!(!o.status.success());
    assert!(
        stderr(&o).contains("run `git push` again"),
        "{}",
        stderr(&o)
    );
    assert!(
        !r.git(&["ls-tree", "-r", "--name-only", "HEAD"])
            .contains("secrets/")
    );
    assert!(r.path().join("secrets/key.pem").exists());
    assert!(r.git(&["ls-remote", "origin"]).is_empty());

    let o = r.git_out(&["push", "-q", "-u", "origin", "main"]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert_eq!(remote_tip(&remote, "main"), r.git(&["rev-parse", "HEAD"]));
    assert!(!remote_files(&remote).contains("secrets/"));
    r.fsck();
}

#[test]
fn a_broken_config_blocks_the_push_and_no_verify_bypasses_the_hook() {
    let (r, remote) = hook_repo("version: 1\n");
    r.config("version: 1\nschedule: {days: [funday]}\nfrom: 2026-01-01\n");
    r.commit_at("a.txt", "a", 1_600_000_000);
    let o = r.git_out(&["push", "-q", "-u", "origin", "main"]);
    assert!(!o.status.success());
    assert!(
        stderr(&o).contains("funday"),
        "the config error is shown: {}",
        stderr(&o)
    );
    assert!(r.git(&["ls-remote", "origin"]).is_empty());

    let o = r.git_out(&["push", "-q", "--no-verify", "-u", "origin", "main"]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert_eq!(remote_tip(&remote, "main"), r.git(&["rev-parse", "HEAD"]));
}

#[test]
fn without_any_rules_the_hook_lets_everything_through() {
    let (r, remote) = hook_repo("version: 1\n");
    r.commit_at("a.txt", "a", 1_600_000_000);
    let o = r.git_out(&["push", "-q", "-u", "origin", "main"]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert_eq!(remote_tip(&remote, "main"), r.git(&["rev-parse", "HEAD"]));
}

#[test]
fn without_a_config_file_the_hook_lets_everything_through() {
    let r = Repo::new();
    let remote = r.bare_remote();
    r.gcma_ok(&["hook", "install"]);
    r.commit_at("a.txt", "a", 1_600_000_000);
    let o = r.git_out(&["push", "-q", "-u", "origin", "main"]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert_eq!(remote_tip(&remote, "main"), r.git(&["rev-parse", "HEAD"]));
}

#[test]
fn the_hook_honours_a_custom_hooks_path() {
    let r = Repo::new();
    r.bare_remote();
    r.config(IDENTITY_CFG);
    let hooks = r.path().join(".githooks");
    std::fs::create_dir_all(&hooks).unwrap();
    r.git(&["config", "core.hooksPath", ".githooks"]);
    r.gcma_ok(&["hook", "install"]);
    r.commit_at("bad.txt", "bad", 1_600_000_000);
    let o = r.git_out(&["push", "-q", "-u", "origin", "main"]);
    assert!(
        !o.status.success(),
        "the hook must run from core.hooksPath ({}): {}",
        hooks.display(),
        stderr(&o)
    );
}

#[test]
fn the_hook_survives_a_push_of_a_new_branch_from_a_clean_history() {
    let (r, _) = hook_repo(IDENTITY_CFG);
    r.commit_as("a.txt", "a", 1_600_000_000, "Jane Doe", "jane@work.com");
    r.git(&["checkout", "-q", "-b", "feature"]);
    r.commit_as("b.txt", "b", 1_600_100_000, "Jane Doe", "jane@work.com");
    let o = r.git_out(&["push", "-q", "-u", "origin", "feature"]);
    assert!(o.status.success(), "{}", stderr(&o));
}

#[test]
fn the_hook_judges_a_nonconforming_ancestor_pushed_as_a_revision() {
    let r = Repo::new();
    r.bare_remote();
    r.config(IDENTITY_CFG);
    r.commit_at("bad.txt", "bad", 1_600_000_000);
    r.commit_as("ok.txt", "ok", 1_600_100_000, "Jane Doe", "jane@work.com");
    r.gcma_ok(&["hook", "install"]);
    let o = r.git_out(&["push", "-q", "origin", "HEAD~1:refs/heads/main"]);
    assert!(
        !o.status.success(),
        "the old identity must not slip through"
    );
    assert!(
        stderr(&o).contains("do not follow the gcma rules"),
        "{}",
        stderr(&o)
    );
    assert!(r.git_out(&["ls-remote", "origin"]).stdout.is_empty());
}

#[test]
fn the_hook_blocks_commits_that_would_only_be_dropped() {
    let r = Repo::new();
    r.bare_remote();
    r.config("version: 1\npaths:\n  exclude: [\"secrets/\"]\n  gitignore: false\n");
    r.commit_files(&[("a.txt", "a\n")], "add a", 1_600_000_000);
    r.gcma_ok(&["hook", "install"]);
    r.git(&["push", "-q", "-u", "origin", "main"]);
    r.commit_files(&[("secrets/k", "k\n")], "add key", 1_600_100_000);
    let o = r.git_out(&["push", "-q", "origin", "main"]);
    assert!(!o.status.success(), "the secret commit must not be pushed");
    assert!(stderr(&o).contains("1 commit(s)"), "{}", stderr(&o));
}
