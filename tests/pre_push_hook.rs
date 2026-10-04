//! The pre-push hook: what it blocks (verify mode), rewrites (rewrite mode) and leaves alone, and
//! how it behaves next to path rules, a broken or missing config and the different ways a push can
//! be spelled. Also the case the hook meets on every push: a settled history that gains new
//! commits, of which only the new ones are rescheduled. Installing it is in `hook_install.rs`.

mod common;

use common::*;

#[test]
fn verify_mode_blocks_nonconforming_pushes_until_they_are_fixed() {
    let r = Repo::hooked(IDENTITY_CFG);
    r.linear(3, T0);

    // First push of a new branch: nonconforming -> blocked.
    r.push_blocked(&["-u", "origin", "main"], "gcma apply");

    r.gcma_ok(&["apply", "--from", "root"]);
    r.push_ok(&["-u", "origin", "main"]);

    // Later: a new commit by the old identity blocks; fixing it (range = upstream..HEAD) unblocks.
    r.commit_at("later.txt", "later", 1_700_000_000);
    r.push_blocked(&["origin", "main"], NONCONFORMING);
    r.gcma_ok(&["apply"]);
    r.push_ok(&[]);
    assert_eq!(
        r.git(&["rev-parse", "HEAD"]),
        r.git(&["rev-parse", "origin/main"])
    );
}

#[test]
fn rewrite_mode_rewrites_aborts_and_the_retry_succeeds() {
    let r = Repo::hooked(&format!("{IDENTITY_CFG}hook:\n  mode: rewrite\n"));
    let remote = r.remote();
    r.linear(3, T0);
    let old_tip = r.git(&["rev-parse", "HEAD"]);

    r.push_blocked(&["-u", "origin", "main"], "run `git push` again");
    let new_tip = r.git(&["rev-parse", "HEAD"]);
    assert_ne!(old_tip, new_tip, "branch was rewritten");
    assert!(r.log().iter().all(|x| x.an == "Jane Doe"));

    r.push_ok(&["-u", "origin", "main"]);
    assert_eq!(remote_tip(&remote, "main"), new_tip);
    r.fsck();
}

#[test]
fn deletes_and_other_branches_are_not_judged_but_the_checked_out_one_is() {
    let r = Repo::hooked(IDENTITY_CFG);
    r.commit_as("ok.txt", "ok", T0, "Jane Doe", "jane@work.com");
    r.git(&["push", "-q", "-u", "origin", "main"]);
    // A nonconforming commit on another branch is not judged when pushing main... and a delete is a no-op.
    r.git(&["checkout", "-q", "-b", "other"]);
    r.commit_at("bad.txt", "bad", 1_600_100_000);
    r.git(&["checkout", "-q", "main"]);
    r.git(&["push", "-q", "origin", "other"]);
    r.push_ok(&["origin", "--delete", "other"]);

    // The hook is live all along: a nonconforming commit on the checked-out branch is blocked.
    r.commit_at("bad-main.txt", "bad on main", 1_600_200_000);
    r.push_blocked(&["origin", "main"], "gcma apply");
}

#[test]
fn a_conforming_ancestor_pushed_as_a_revision_passes() {
    let r = Repo::hooked(IDENTITY_CFG);
    r.commit_as("a.txt", "a", T0, "Jane Doe", "jane@work.com");
    r.commit_as("b.txt", "b", 1_600_100_000, "Jane Doe", "jane@work.com");
    r.commit_at("bad.txt", "bad", 1_600_200_000); // nonconforming tip
    // Pushing HEAD~1 to main only sends conforming commits, so it passes (the opposite case is
    // `a_nonconforming_ancestor_pushed_as_a_revision_is_blocked`).
    r.push_ok(&["origin", "HEAD~1:refs/heads/main"]);
    // Pushing the branch itself includes the bad tip and is blocked.
    r.push_blocked(&["origin", "main"], NONCONFORMING);
}

#[test]
fn pushes_spelled_as_head_or_a_sha_are_judged() {
    let r = Repo::hooked(IDENTITY_CFG);
    r.commit_at("bad.txt", "bad", 1_600_200_000); // nonconforming tip
    let tip = r.git(&["rev-parse", "HEAD"]);
    let remote_spec = format!("{tip}:refs/heads/o2");
    for spec in [
        "HEAD",
        "HEAD:refs/heads/main",
        "HEAD:refs/heads/other",
        &remote_spec,
    ] {
        let err = r.push_blocked(&["origin", spec], NONCONFORMING);
        assert!(err.contains("--from"), "no-upstream hint missing: {err}");
    }
}

#[test]
fn rewrite_mode_fixes_a_push_spelled_as_head() {
    let r = Repo::hooked(&format!("{IDENTITY_CFG}hook: {{mode: rewrite}}\n"));
    r.commit_at("bad.txt", "bad", 1_600_200_000);
    let old_tip = r.git(&["rev-parse", "HEAD"]);
    r.push_blocked(&["origin", "HEAD"], "run `git push` again");
    assert_ne!(
        r.git(&["rev-parse", "HEAD"]),
        old_tip,
        "branch was rewritten"
    );
    assert_eq!(r.log().last().unwrap().an, "Jane Doe");
    r.push_ok(&["origin", "HEAD"]);
}

// ---------- the hook next to path rules, config errors and other push shapes ----------

#[test]
fn verify_mode_blocks_secrets_until_the_history_is_cleaned() {
    let r = Repo::hooked(SECRETS_CFG);
    let remote = r.remote();
    r.commit_files(
        &[("src/a.rs", "a\n"), ("secrets/key.pem", "k\n")],
        "feature",
        T0,
    );
    r.push_blocked(&["-u", "origin", "main"], NONCONFORMING);

    r.gcma_ok(&["apply", "--from", "root"]);
    r.push_ok(&["-u", "origin", "main"]);
    assert!(!remote_files(&remote).contains("secrets/"));
    assert!(remote_files(&remote).contains("src/a.rs"));
}

#[test]
fn rewrite_mode_removes_secrets_aborts_and_the_retry_succeeds() {
    let r = Repo::hooked(&format!("{SECRETS_CFG}hook:\n  mode: rewrite\n"));
    let remote = r.remote();
    r.commit_files(
        &[("src/a.rs", "a\n"), ("secrets/key.pem", "k\n")],
        "feature",
        T0,
    );
    r.push_blocked(&["-u", "origin", "main"], "run `git push` again");
    assert!(
        !r.git(&["ls-tree", "-r", "--name-only", "HEAD"])
            .contains("secrets/")
    );
    assert!(r.path().join("secrets/key.pem").exists());

    r.push_ok(&["-u", "origin", "main"]);
    assert_eq!(remote_tip(&remote, "main"), r.git(&["rev-parse", "HEAD"]));
    assert!(!remote_files(&remote).contains("secrets/"));
    r.fsck();
}

#[test]
fn a_broken_config_blocks_the_push_and_no_verify_bypasses_the_hook() {
    let r = Repo::hooked("version: 1\n");
    let remote = r.remote();
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
    let r = Repo::hooked("version: 1\n");
    let remote = r.remote();
    r.commit_at("a.txt", "a", T0);
    r.push_ok(&["-u", "origin", "main"]);
    assert_eq!(remote_tip(&remote, "main"), r.git(&["rev-parse", "HEAD"]));
}

#[test]
fn without_a_config_file_every_push_passes() {
    let r = Repo::new();
    let remote = r.bare_remote();
    r.gcma_ok(&["hook", "install"]);
    r.commit_at("a.txt", "a", T0);
    r.push_ok(&["-u", "origin", "main"]);
    assert_eq!(remote_tip(&remote, "main"), r.git(&["rev-parse", "HEAD"]));
}

#[test]
fn a_new_branch_of_conforming_commits_is_pushed() {
    let r = Repo::hooked(IDENTITY_CFG);
    r.commit_as("a.txt", "a", T0, "Jane Doe", "jane@work.com");
    r.git(&["checkout", "-q", "-b", "feature"]);
    r.commit_as("b.txt", "b", 1_600_100_000, "Jane Doe", "jane@work.com");
    r.push_ok(&["-u", "origin", "feature"]);
}

#[test]
fn a_nonconforming_ancestor_pushed_as_a_revision_is_blocked() {
    let r = Repo::hooked(IDENTITY_CFG);
    r.commit_at("bad.txt", "bad", T0);
    r.commit_as("ok.txt", "ok", 1_600_100_000, "Jane Doe", "jane@work.com");
    r.push_blocked(&["origin", "HEAD~1:refs/heads/main"], NONCONFORMING);
}

#[test]
fn commits_that_would_only_be_dropped_are_blocked() {
    let r = Repo::hooked(SECRETS_NO_GITIGNORE_CFG);
    r.commit_files(&[("a.txt", "a\n")], "add a", T0);
    r.push_ok(&["-u", "origin", "main"]);
    r.commit_files(&[("secrets/k", "k\n")], "add key", 1_600_100_000);
    let err = r.push_blocked(&["origin", "main"], NONCONFORMING);
    assert!(err.contains("1 commit(s)"), "{err}");
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
    let r = Repo::hooked(&format!("{SECRETS_CFG}hook:\n  mode: rewrite\n"));
    r.commit_files(&[(".gitignore", "target\n"), ("a.txt", "a\n")], "init", T0);
    r.commit_files(&[("secrets/k", "k\n"), ("b.txt", "b\n")], "add b", T0 + 100);
    r.write(".gitignore", "target\nmine\n"); // unstaged local edit
    let err = r.push_blocked(&["-u", "origin", "main"], "run `git push` again");
    assert!(
        err.contains("gcma: pre-push: the working copy's .gitignore has local changes"),
        "{err}"
    );
    assert!(err.contains("rotate"), "the secrets note is printed: {err}");
}
