//! Every situation `apply` must refuse: the exit code is 3, the message says why, and not a single
//! ref, backup or file changes.

mod common;

use common::*;

/// All refs and their targets: the thing that must not change when a run is refused.
fn refs(r: &Repo) -> String {
    r.git(&["for-each-ref", "--format=%(refname) %(objectname)"])
}

fn refused(r: &Repo, args: &[&str], why: &str) {
    let before = refs(r);
    let head = r.git(&["rev-parse", "HEAD"]);
    let o = r.gcma(args);
    assert_eq!(Repo::code(&o), 3, "{args:?}: {}", stderr(&o));
    assert!(
        stderr(&o).to_lowercase().contains(&why.to_lowercase()),
        "expected {why:?} in: {}",
        stderr(&o)
    );
    assert_eq!(refs(r), before, "no ref may change");
    assert_eq!(r.git(&["rev-parse", "HEAD"]), head);
    assert!(
        !r.git(&["for-each-ref", "refs/gcma/"]).contains("backup"),
        "no backup is created for a refused run"
    );
}

fn setup() -> Repo {
    let r = Repo::new();
    r.linear(3, 1_600_000_000);
    r.config(IDENTITY_CFG);
    r
}

/// Two branches that change the same line differently, so that merging/picking conflicts.
fn conflicting(r: &Repo) {
    r.write("clash.txt", "base\n");
    r.git(&["add", "clash.txt"]);
    r.git(&["commit", "-q", "-m", "base clash"]);
    r.git(&["checkout", "-q", "-b", "side"]);
    r.write("clash.txt", "side\n");
    r.git(&["commit", "-q", "-am", "side change"]);
    r.git(&["checkout", "-q", "main"]);
    r.write("clash.txt", "main\n");
    r.git(&["commit", "-q", "-am", "main change"]);
}

#[test]
fn a_detached_head_is_refused() {
    let r = setup();
    r.git(&["checkout", "-q", "--detach"]);
    refused(&r, &["apply", "--from", "root"], "detached HEAD");
    refused(&r, &["plan", "--from", "root"], "detached HEAD");
}

#[test]
fn staged_changes_are_refused_by_apply_but_not_by_plan() {
    let r = setup();
    r.write("staged.txt", "s\n");
    r.git(&["add", "staged.txt"]);
    r.gcma_ok(&["plan", "--from", "root"]);
    refused(&r, &["apply", "--from", "root"], "staged changes");
    assert!(
        r.git(&["diff", "--cached", "--name-only"])
            .contains("staged.txt")
    );
}

#[test]
fn unstaged_and_untracked_files_do_not_block_and_survive() {
    let r = setup();
    r.write("f0.txt", "edited but not staged\n");
    r.write("untracked.txt", "new\n");
    r.gcma_ok(&["apply", "--from", "root"]);
    assert_eq!(
        std::fs::read_to_string(r.path().join("f0.txt")).unwrap(),
        "edited but not staged\n"
    );
    assert!(r.path().join("untracked.txt").exists());
    let status = r.git(&["status", "--porcelain"]);
    assert!(status.contains(" M f0.txt"), "{status}");
    assert!(status.contains("?? untracked.txt"), "{status}");
    r.fsck();
}

#[test]
fn a_shallow_clone_is_refused_even_for_a_dry_run() {
    let r = setup();
    let clone = r.home.path().join("shallow");
    let o = r
        .cmd("git")
        .args(["clone", "-q", "--depth", "1"])
        .arg(format!("file://{}", r.path().display()))
        .arg(&clone)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", stderr(&o));
    std::fs::copy(r.path().join("gcma.yml"), clone.join("gcma.yml")).unwrap();
    for cmd in ["plan", "apply"] {
        let o = base_cmd(bin(), &clone, r.home.path())
            .args([cmd, "--from", "root"])
            .output()
            .unwrap();
        assert_eq!(Repo::code(&o), 3, "{cmd}: {}", stderr(&o));
        assert!(stderr(&o).contains("shallow"), "{}", stderr(&o));
    }
}

#[test]
fn replace_refs_are_refused() {
    let r = setup();
    let first = r.git(&["rev-list", "--max-parents=0", "HEAD"]);
    let tip = r.git(&["rev-parse", "HEAD"]);
    r.git(&["replace", &first, &tip]);
    refused(&r, &["apply", "--from", "root"], "replace refs");
}

#[test]
fn grafts_are_refused() {
    let r = setup();
    let first = r.git(&["rev-list", "--max-parents=0", "HEAD"]);
    let tip = r.git(&["rev-parse", "HEAD"]);
    std::fs::create_dir_all(r.path().join(".git/info")).unwrap();
    std::fs::write(
        r.path().join(".git/info/grafts"),
        format!("{tip} {first}\n"),
    )
    .unwrap();
    refused(&r, &["apply", "--from", "root"], "grafts");
}

#[test]
fn a_merge_in_progress_is_refused() {
    let r = setup();
    conflicting(&r);
    let o = r.git_out(&["merge", "side"]);
    assert!(!o.status.success(), "the fixture must conflict");
    refused(&r, &["apply", "--from", "root"], "merge is in progress");
    r.gcma_ok(&["plan", "--from", "root"]);
}

#[test]
fn a_cherry_pick_in_progress_is_refused() {
    let r = setup();
    conflicting(&r);
    let o = r.git_out(&["cherry-pick", "side"]);
    assert!(!o.status.success(), "the fixture must conflict");
    refused(
        &r,
        &["apply", "--from", "root"],
        "cherry-pick is in progress",
    );
}

#[test]
fn a_revert_in_progress_is_refused() {
    let r = setup();
    conflicting(&r);
    // Reverting the first clash change conflicts with the later one.
    let base = r.git(&["log", "--format=%H", "--grep=base clash"]);
    let o = r.git_out(&["revert", "--no-edit", &base]);
    assert!(!o.status.success(), "the fixture must conflict");
    refused(&r, &["apply", "--from", "root"], "revert is in progress");
}

#[test]
fn a_rebase_in_progress_is_refused() {
    let r = setup();
    conflicting(&r);
    r.git(&["checkout", "-q", "side"]);
    let o = r.git_out(&["rebase", "main"]);
    assert!(!o.status.success(), "the fixture must conflict");
    // Detached HEAD and a running rebase: either reason is a refusal.
    let before = refs(&r);
    let o = r.gcma(&["apply", "--from", "root"]);
    assert_eq!(Repo::code(&o), 3, "{}", stderr(&o));
    assert_eq!(refs(&r), before);
}

#[test]
fn outside_a_repository_is_an_error_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("gcma.yml"), IDENTITY_CFG).unwrap();
    let o = base_cmd(bin(), dir.path(), home.path())
        .args(["apply", "--from", "root"])
        .env("GIT_CEILING_DIRECTORIES", dir.path().parent().unwrap())
        .output()
        .unwrap();
    assert_eq!(Repo::code(&o), 2, "{}", stderr(&o));
    assert!(
        stderr(&o).contains("not inside a git work tree"),
        "{}",
        stderr(&o)
    );
    assert!(!dir.path().join(".git").exists());
}

#[test]
fn a_repository_without_commits_is_a_clear_error() {
    let r = Repo::new();
    r.config(IDENTITY_CFG);
    let o = r.gcma(&["apply", "--from", "root"]);
    assert!(!o.status.success());
    assert_ne!(Repo::code(&o), 101, "no panic: {}", stderr(&o));
    assert!(stderr(&o).starts_with("gcma:"), "{}", stderr(&o));
}

#[test]
fn without_a_config_file_gcma_is_inert_and_says_so_but_a_named_missing_file_is_an_error() {
    let r = Repo::new();
    r.linear(2, 1_600_000_000);
    let o = r.gcma(&["plan", "--from", "root"]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(
        stderr(&o).contains("no rules are configured"),
        "{}",
        stderr(&o)
    );
    let o = r.gcma(&["--config", "does-not-exist.yml", "plan", "--from", "root"]);
    assert_eq!(Repo::code(&o), 2, "{}", stderr(&o));
    assert!(stderr(&o).contains("does-not-exist.yml"), "{}", stderr(&o));
}

#[test]
fn no_upstream_requires_from() {
    let r = Repo::new();
    r.linear(2, 1_600_000_000);
    r.config(IDENTITY_CFG);
    let o = r.gcma(&["plan"]);
    assert_eq!(Repo::code(&o), 2, "{}", String::from_utf8_lossy(&o.stderr));
    let o = r.gcma(&["plan", "--from", "HEAD~5"]);
    assert_eq!(Repo::code(&o), 2);
}

#[test]
fn from_rev_is_exclusive() {
    let r = Repo::new();
    let c = r.linear(4, 1_600_000_000);
    r.config(IDENTITY_CFG);
    r.gcma_ok(&["apply", "--from", &c[1]]);
    let rows = r.log();
    assert_eq!(rows[0].an, "Old Me");
    assert_eq!(rows[1].an, "Old Me");
    assert_eq!(rows[1].oid, c[1], "the --from commit itself is untouched");
    assert_eq!(rows[2].an, "Jane Doe");
    assert_eq!(rows[3].an, "Jane Doe");
}
