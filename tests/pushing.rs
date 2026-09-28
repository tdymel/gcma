//! Pushed commits: every command that would change one exits 5 unless `--rewrite-pushed` is given,
//! the default range stops at the upstream, and rewriting pushed secrets ends in a force push.

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
