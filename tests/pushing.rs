//! Pushed commits, the pre-push hook and path rules / config errors in their presence.

mod common;

use common::*;

fn pushed_history() -> (Repo, std::path::PathBuf) {
    let r = Repo::new();
    r.linear(3, 1_600_000_000);
    let remote = r.bare_remote();
    r.git(&["push", "-q", "-u", "origin", "main"]);
    r.commit_at("local.txt", "local only", 1_600_900_000);
    (r, remote)
}

// ---------- pushed commits ----------

#[test]
fn every_command_that_would_touch_pushed_commits_exits_5() {
    let (r, _) = pushed_history();
    r.config(IDENTITY_CFG);
    let tip = r.git(&["rev-parse", "HEAD"]);
    for args in [
        vec!["plan", "--from", "root"],
        vec!["plan", "--from", "root", "--check"],
        vec!["apply", "--from", "root"],
        vec!["export", "--from", "root"],
    ] {
        let o = r.gcma(&args);
        assert_eq!(Repo::code(&o), 5, "{args:?}: {}", stderr(&o));
        assert!(
            stderr(&o).contains("--rewrite-pushed"),
            "{args:?}: {}",
            stderr(&o)
        );
    }
    assert_eq!(r.git(&["rev-parse", "HEAD"]), tip);
    assert!(r.git(&["for-each-ref", "refs/gcma/"]).is_empty());
}

#[test]
fn a_check_on_unpushed_commits_still_reports_nonconformance_with_exit_6() {
    let (r, _) = pushed_history();
    r.config(IDENTITY_CFG);
    let o = r.gcma(&["plan", "--check"]);
    assert_eq!(Repo::code(&o), 6, "{}", stderr(&o));
    assert!(stderr(&o).contains("1 commit(s)"), "{}", stderr(&o));
}

#[test]
fn nothing_to_rewrite_in_the_pushed_range_is_fine_without_the_flag() {
    // The pushed commits already follow the identity rule, so even an explicit whole-branch run
    // has nothing to rewrite and must not trip over them being pushed.
    let r = Repo::new();
    for i in 0..3 {
        r.commit_as(
            &format!("f{i}.txt"),
            &format!("c{i}"),
            1_600_000_000 + i * 1000,
            "Jane Doe",
            "jane@work.com",
        );
    }
    r.bare_remote();
    r.git(&["push", "-q", "-u", "origin", "main"]);
    r.config(IDENTITY_CFG);
    for cmd in ["plan", "apply"] {
        let o = r.gcma(&[cmd, "--from", "root"]);
        assert!(o.status.success(), "{cmd}: {}", stderr(&o));
        assert!(
            String::from_utf8_lossy(&o.stdout).contains("Nothing to do"),
            "{cmd}: {}",
            String::from_utf8_lossy(&o.stdout)
        );
        assert!(
            !stderr(&o).contains("no rules"),
            "{cmd}: the config is not inert"
        );
    }
    // The same range does trip once a pushed commit has to change.
    r.commit_at("later.txt", "pushed too", 1_600_900_000);
    r.git(&["push", "-q"]);
    let o = r.gcma(&["plan", "--from", "root"]);
    assert_eq!(Repo::code(&o), 5, "{}", stderr(&o));
}

#[test]
fn rewriting_pushed_secrets_then_force_pushing_removes_them_from_the_remote_branch() {
    let r = Repo::new();
    r.commit_files(&[("src/lib.rs", "fn main() {}\n")], "init", 1_600_000_000);
    r.commit_files(
        &[("src/a.rs", "a\n"), ("secrets/key.pem", "k\n")],
        "feature",
        1_600_100_000,
    );
    let remote = r.bare_remote();
    r.git(&["push", "-q", "-u", "origin", "main"]);
    assert!(remote_files(&remote).contains("secrets/key.pem"));
    r.config(SECRETS_CFG);

    let o = r.gcma(&["apply", "--from", "root"]);
    assert_eq!(Repo::code(&o), 5, "{}", stderr(&o));
    r.gcma_ok(&["apply", "--from", "root", "--rewrite-pushed"]);
    assert!(
        !r.git(&["ls-tree", "-r", "--name-only", "HEAD"])
            .contains("secrets/")
    );
    assert!(
        r.path().join("secrets/key.pem").exists(),
        "the project keeps the file"
    );

    // The history diverged: a plain push is rejected, a forced one replaces the remote branch.
    let o = r.git_out(&["push", "-q", "origin", "main"]);
    assert!(!o.status.success(), "non-fast-forward");
    r.git(&["push", "-q", "--force-with-lease", "origin", "main"]);
    assert_eq!(remote_tip(&remote, "main"), r.git(&["rev-parse", "HEAD"]));
    assert!(!remote_files(&remote).contains("secrets/"));
    r.fsck();
}

// ---------- the hook ----------

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
fn pushed_commits_need_the_flag() {
    let r = Repo::new();
    r.linear(3, 1_600_000_000);
    r.bare_remote();
    r.git(&["push", "-q", "-u", "origin", "main"]);
    r.commit_at("local.txt", "local only", 1_600_900_000);
    r.config(IDENTITY_CFG);

    // Default range is upstream..HEAD: only the unpushed commit changes.
    let plan = r.gcma_ok(&["plan"]);
    assert!(plan.contains("1 to rewrite"), "{plan}");

    // Forcing the whole branch hits pushed commits.
    let o = r.gcma(&["plan", "--from", "root"]);
    assert_eq!(Repo::code(&o), 5, "{}", String::from_utf8_lossy(&o.stderr));
    let o = r.gcma(&["apply", "--from", "root"]);
    assert_eq!(Repo::code(&o), 5);
    assert_eq!(r.log().iter().filter(|x| x.an == "Jane Doe").count(), 0);

    r.gcma_ok(&["apply", "--from", "root", "--rewrite-pushed"]);
    assert_eq!(r.log().iter().filter(|x| x.an == "Jane Doe").count(), 4);
}

#[test]
fn default_range_rewrites_only_unpushed_commits() {
    let r = Repo::new();
    let pushed = r.linear(3, 1_600_000_000);
    r.bare_remote();
    r.git(&["push", "-q", "-u", "origin", "main"]);
    r.commit_at("l1.txt", "l1", 1_600_900_000);
    r.commit_at("l2.txt", "l2", 1_600_950_000);
    r.config(IDENTITY_CFG);
    r.gcma_ok(&["apply"]);
    let rows = r.log();
    assert_eq!(rows[2].oid, pushed[2], "pushed commits are untouched");
    assert_eq!(rows[3].an, "Jane Doe");
    assert_eq!(rows[4].an, "Jane Doe");
    r.fsck();
}
