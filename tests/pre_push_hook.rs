//! The pre-push hook: what it blocks (verify mode), rewrites (rewrite mode) and leaves alone, and
//! how it behaves next to path rules, a broken or missing config and the different ways a push can
//! be spelled. Also the case the hook meets on every push: a settled history that gains new
//! commits, of which only the new ones are rescheduled. Installing it is in `hook_install.rs`.

mod common;

use common::*;

#[test]
fn verify_mode_blocks_nonconforming_pushes_until_they_are_fixed() {
    let r = Repo::new();
    r.bare_remote();
    r.config(IDENTITY_CFG);
    r.linear(3, T0);
    r.gcma_ok(&["hook", "install"]);

    // First push of a new branch: nonconforming -> blocked.
    let o = r.git_out(&["push", "-q", "-u", "origin", "main"]);
    assert!(!o.status.success());
    assert!(stderr(&o).contains("gcma apply"), "{}", stderr(&o));
    assert!(
        r.git(&["ls-remote", "origin"]).is_empty(),
        "nothing was pushed"
    );

    r.gcma_ok(&["apply", "--from", "root"]);
    let o = r.git_out(&["push", "-q", "-u", "origin", "main"]);
    assert!(o.status.success(), "{}", stderr(&o));

    // Later: a new commit by the old identity blocks; fixing it (range = upstream..HEAD) unblocks.
    r.commit_at("later.txt", "later", 1_700_000_000);
    let o = r.git_out(&["push", "-q"]);
    assert!(!o.status.success());
    r.gcma_ok(&["apply"]);
    let o = r.git_out(&["push", "-q"]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert_eq!(
        r.git(&["rev-parse", "HEAD"]),
        r.git(&["rev-parse", "origin/main"])
    );
}

#[test]
fn rewrite_mode_rewrites_aborts_and_the_retry_succeeds() {
    let r = Repo::new();
    let remote = r.bare_remote();
    r.config(&format!("{IDENTITY_CFG}hook:\n  mode: rewrite\n"));
    r.linear(3, T0);
    r.gcma_ok(&["hook", "install"]);
    let old_tip = r.git(&["rev-parse", "HEAD"]);

    let o = r.git_out(&["push", "-q", "-u", "origin", "main"]);
    assert!(!o.status.success());
    assert!(
        stderr(&o).contains("run `git push` again"),
        "{}",
        stderr(&o)
    );
    let new_tip = r.git(&["rev-parse", "HEAD"]);
    assert_ne!(old_tip, new_tip, "branch was rewritten");
    assert!(r.log().iter().all(|x| x.an == "Jane Doe"));
    assert!(r.git(&["ls-remote", "origin"]).is_empty());

    let o = r.git_out(&["push", "-q", "-u", "origin", "main"]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert_eq!(remote_tip(&remote, "main"), new_tip);
    r.fsck();
}

#[test]
fn deletes_and_other_branches_are_not_judged_but_the_checked_out_one_is() {
    let r = Repo::new();
    r.bare_remote();
    r.config(IDENTITY_CFG);
    r.commit_as("ok.txt", "ok", T0, "Jane Doe", "jane@work.com");
    r.gcma_ok(&["hook", "install"]);
    r.git(&["push", "-q", "-u", "origin", "main"]);
    // A nonconforming commit on another branch is not judged when pushing main... and a delete is a no-op.
    r.git(&["checkout", "-q", "-b", "other"]);
    r.commit_at("bad.txt", "bad", 1_600_100_000);
    r.git(&["checkout", "-q", "main"]);
    r.git(&["push", "-q", "origin", "other"]);
    let o = r.git_out(&["push", "-q", "origin", "--delete", "other"]);
    assert!(o.status.success(), "{}", stderr(&o));

    // The hook is live all along: a nonconforming commit on the checked-out branch is blocked.
    r.commit_at("bad-main.txt", "bad on main", 1_600_200_000);
    let o = r.git_out(&["push", "-q", "origin", "main"]);
    assert!(
        !o.status.success(),
        "the hook must judge the checked-out branch"
    );
    assert!(stderr(&o).contains("gcma apply"));
}

#[test]
fn a_conforming_ancestor_pushed_as_a_revision_passes() {
    let r = Repo::new();
    r.bare_remote();
    r.config(IDENTITY_CFG);
    r.commit_as("a.txt", "a", T0, "Jane Doe", "jane@work.com");
    r.commit_as("b.txt", "b", 1_600_100_000, "Jane Doe", "jane@work.com");
    r.commit_at("bad.txt", "bad", 1_600_200_000); // nonconforming tip
    r.gcma_ok(&["hook", "install"]);
    // Pushing HEAD~1 to main only sends conforming commits, so it passes (the opposite case is
    // `a_nonconforming_ancestor_pushed_as_a_revision_is_blocked`).
    let o = r.git_out(&["push", "-q", "origin", "HEAD~1:refs/heads/main"]);
    assert!(o.status.success(), "{}", stderr(&o));
    // Pushing the branch itself includes the bad tip and is blocked.
    let o = r.git_out(&["push", "-q", "origin", "main"]);
    assert!(!o.status.success());
}

#[test]
fn pushes_spelled_as_head_or_a_sha_are_judged() {
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
        let err = stderr(&o);
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
fn rewrite_mode_fixes_a_push_spelled_as_head() {
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
    assert!(o.status.success(), "{}", stderr(&o));
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
fn verify_mode_blocks_secrets_until_the_history_is_cleaned() {
    let (r, remote) = hook_repo(SECRETS_CFG);
    r.commit_files(
        &[("src/a.rs", "a\n"), ("secrets/key.pem", "k\n")],
        "feature",
        T0,
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
fn rewrite_mode_removes_secrets_aborts_and_the_retry_succeeds() {
    let (r, remote) = hook_repo(&format!("{SECRETS_CFG}hook:\n  mode: rewrite\n"));
    r.commit_files(
        &[("src/a.rs", "a\n"), ("secrets/key.pem", "k\n")],
        "feature",
        T0,
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
    r.commit_at("a.txt", "a", T0);
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
fn without_any_rules_every_push_passes() {
    let (r, remote) = hook_repo("version: 1\n");
    r.commit_at("a.txt", "a", T0);
    let o = r.git_out(&["push", "-q", "-u", "origin", "main"]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert_eq!(remote_tip(&remote, "main"), r.git(&["rev-parse", "HEAD"]));
}

#[test]
fn without_a_config_file_every_push_passes() {
    let r = Repo::new();
    let remote = r.bare_remote();
    r.gcma_ok(&["hook", "install"]);
    r.commit_at("a.txt", "a", T0);
    let o = r.git_out(&["push", "-q", "-u", "origin", "main"]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert_eq!(remote_tip(&remote, "main"), r.git(&["rev-parse", "HEAD"]));
}

#[test]
fn a_new_branch_of_conforming_commits_is_pushed() {
    let (r, _) = hook_repo(IDENTITY_CFG);
    r.commit_as("a.txt", "a", T0, "Jane Doe", "jane@work.com");
    r.git(&["checkout", "-q", "-b", "feature"]);
    r.commit_as("b.txt", "b", 1_600_100_000, "Jane Doe", "jane@work.com");
    let o = r.git_out(&["push", "-q", "-u", "origin", "feature"]);
    assert!(o.status.success(), "{}", stderr(&o));
}

#[test]
fn a_nonconforming_ancestor_pushed_as_a_revision_is_blocked() {
    let r = Repo::new();
    r.bare_remote();
    r.config(IDENTITY_CFG);
    r.commit_at("bad.txt", "bad", T0);
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
fn commits_that_would_only_be_dropped_are_blocked() {
    let r = Repo::new();
    r.bare_remote();
    r.config(SECRETS_NO_GITIGNORE_CFG);
    r.commit_files(&[("a.txt", "a\n")], "add a", T0);
    r.gcma_ok(&["hook", "install"]);
    r.git(&["push", "-q", "-u", "origin", "main"]);
    r.commit_files(&[("secrets/k", "k\n")], "add key", 1_600_100_000);
    let o = r.git_out(&["push", "-q", "origin", "main"]);
    assert!(!o.status.success(), "the secret commit must not be pushed");
    assert!(stderr(&o).contains("1 commit(s)"), "{}", stderr(&o));
}

#[test]
fn a_settled_history_reschedules_only_its_new_commits() {
    let r = Repo::new();
    r.linear(10, 1_500_000_000);
    r.config(&berlin_cfg(""));
    r.gcma_ok(&["apply", "--from", "root"]);
    let settled = r.log();
    // Three new commits made "now-ish" (outside the allowed window).
    let t = settled.last().unwrap().ct + 3600 * 24 * 3 + 7 * 3600; // a night, a few days later
    for i in 0..3 {
        r.commit_at(&format!("new{i}.txt"), &format!("new {i}"), t + i * 60);
    }
    let plan = r.gcma_ok(&["plan", "--from", "root"]);
    assert!(plan.contains("10 kept as-is, 3 to rewrite"), "{plan}");
    r.gcma_ok(&["apply", "--from", "root"]);
    let after = r.log();
    for (a, b) in settled.iter().zip(&after) {
        assert_eq!(a.oid, b.oid, "settled commits keep their OIDs");
    }
    assert_eq!(after.len(), 13);
    assert_scheduled(&after);
    assert!(after[10].ct >= settled.last().unwrap().ct);
}

#[test]
fn rewrite_mode_prints_the_warnings_of_the_rewrite() {
    let (r, _remote) = hook_repo(&format!("{SECRETS_CFG}hook:\n  mode: rewrite\n"));
    r.commit_files(&[(".gitignore", "target\n"), ("a.txt", "a\n")], "init", T0);
    r.commit_files(&[("secrets/k", "k\n"), ("b.txt", "b\n")], "add b", T0 + 100);
    r.write(".gitignore", "target\nmine\n"); // unstaged local edit
    let o = r.git_out(&["push", "-q", "-u", "origin", "main"]);
    assert!(!o.status.success());
    let err = stderr(&o);
    assert!(err.contains("run `git push` again"), "{err}");
    assert!(
        err.contains("gcma: pre-push: the working copy's .gitignore has local changes"),
        "{err}"
    );
    assert!(err.contains("rotate"), "the secrets note is printed: {err}");
}
