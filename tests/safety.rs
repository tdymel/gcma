//! Regression tests for the review findings: guards around plans, hooks, restore and files.

mod common;

use common::*;

const IDENTITY_CFG: &str = "version: 1\nidentity:\n  - match: {email: me@home.org}\n    set: {name: Jane Doe, email: jane@work.com}\n";
const SECRETS_CFG: &str = "version: 1\npaths:\n  exclude: [\"secrets/\"]\n";

fn stderr(o: &std::process::Output) -> String {
    String::from_utf8_lossy(&o.stderr).to_string()
}

fn status_without_config(r: &Repo) -> String {
    r.git(&["status", "--porcelain"])
        .lines()
        .filter(|l| !l.contains("ghma.yml"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn plan_check_exits_6_and_changes_nothing() {
    let r = Repo::new();
    r.linear(3, 1_600_000_000);
    r.config(IDENTITY_CFG);
    let tip = r.git(&["rev-parse", "HEAD"]);
    let o = r.ghma(&["plan", "--check", "--from", "root"]);
    assert_eq!(Repo::code(&o), 6, "{}", stderr(&o));
    assert!(stderr(&o).contains("3 commit(s) do not follow"));
    assert_eq!(r.git(&["rev-parse", "HEAD"]), tip);
    assert!(r.git(&["for-each-ref", "refs/ghma/"]).is_empty());
}

#[test]
fn the_hook_judges_a_nonconforming_ancestor_pushed_as_a_revision() {
    let r = Repo::new();
    r.bare_remote();
    r.config(IDENTITY_CFG);
    r.commit_at("bad.txt", "bad", 1_600_000_000);
    r.commit_as("ok.txt", "ok", 1_600_100_000, "Jane Doe", "jane@work.com");
    r.ghma_ok(&["hook", "install"]);
    let o = r.git_out(&["push", "-q", "origin", "HEAD~1:refs/heads/main"]);
    assert!(
        !o.status.success(),
        "the old identity must not slip through"
    );
    assert!(
        stderr(&o).contains("do not follow the ghma rules"),
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
    r.ghma_ok(&["hook", "install"]);
    r.git(&["push", "-q", "-u", "origin", "main"]);
    r.commit_files(&[("secrets/k", "k\n")], "add key", 1_600_100_000);
    let o = r.git_out(&["push", "-q", "origin", "main"]);
    assert!(!o.status.success(), "the secret commit must not be pushed");
    assert!(stderr(&o).contains("1 commit(s)"), "{}", stderr(&o));
}

#[test]
fn restore_after_path_rules_leaves_index_and_gitignore_as_before() {
    let r = Repo::new();
    r.commit_files(&[("a.txt", "a\n")], "add a", 1_600_000_000);
    r.commit_files(
        &[("secrets/k", "k\n"), ("b.txt", "b\n")],
        "add b",
        1_600_100_000,
    );
    r.config(SECRETS_CFG);
    let tip = r.git(&["rev-parse", "HEAD"]);
    r.ghma_ok(&["apply", "--from", "root"]);
    assert!(r.path().join(".gitignore").exists());
    let id = r.git(&["for-each-ref", "--format=%(refname)", "refs/ghma/backup/"]);
    let id = id
        .lines()
        .next()
        .unwrap()
        .rsplit('/')
        .nth(1)
        .unwrap()
        .to_string();
    r.ghma_ok(&["restore", &id]);
    assert_eq!(r.git(&["rev-parse", "HEAD"]), tip);
    assert_eq!(status_without_config(&r), "", "{}", r.git(&["status"]));
    assert!(
        !r.path().join(".gitignore").exists(),
        "ghma's own .gitignore is gone again"
    );
    assert!(r.path().join("secrets/k").exists());
}

#[test]
fn a_plan_cannot_retarget_other_refs_or_bring_its_own_path_rules() {
    let r = Repo::new();
    r.commit_files(&[("a.txt", "a\n")], "add a", 1_600_000_000);
    r.commit_files(
        &[("secrets/k", "k\n"), ("b.txt", "b\n")],
        "add b",
        1_600_100_000,
    );
    r.config(SECRETS_CFG);
    let plan_path = r.path().join("plan.json");
    let plan_arg = plan_path.to_str().unwrap().to_string();
    r.ghma_ok(&["plan", "--from", "root", "--out", &plan_arg]);
    let tip = r.git(&["rev-parse", "HEAD"]);

    // Another ref at the tip (a tag) must not be rewritten.
    r.git(&["tag", "v1"]);
    let mut plan: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&plan_path).unwrap()).unwrap();
    plan["branch_ref"] = "refs/tags/v1".into();
    std::fs::write(&plan_path, serde_json::to_vec(&plan).unwrap()).unwrap();
    let o = r.ghma(&["apply", "--plan", &plan_arg]);
    assert_eq!(Repo::code(&o), 2, "{}", stderr(&o));
    assert_eq!(r.git(&["rev-parse", "v1"]), tip);

    // A plan made under other rules than the config's is refused.
    r.ghma_ok(&["plan", "--from", "root", "--out", &plan_arg]);
    r.config("version: 1\npaths:\n  exclude: [\"b.txt\"]\n");
    let o = r.ghma(&["apply", "--plan", &plan_arg]);
    assert_eq!(Repo::code(&o), 2, "{}", stderr(&o));
    assert!(stderr(&o).contains("path rules"), "{}", stderr(&o));
    assert_eq!(r.git(&["rev-parse", "HEAD"]), tip);
}

#[test]
fn a_plan_for_another_branch_is_refused() {
    let r = Repo::new();
    r.linear(2, 1_600_000_000);
    r.config(IDENTITY_CFG);
    let plan_path = r.path().join("plan.json");
    let plan_arg = plan_path.to_str().unwrap().to_string();
    r.ghma_ok(&["plan", "--from", "root", "--out", &plan_arg]);
    r.git(&["checkout", "-q", "-b", "other"]);
    let o = r.ghma(&["apply", "--plan", &plan_arg]);
    assert_eq!(Repo::code(&o), 3, "{}", stderr(&o));
}

#[test]
fn local_gitignore_edits_are_kept_and_reported() {
    let r = Repo::new();
    r.commit_files(
        &[(".gitignore", "target\n"), ("a.txt", "a\n")],
        "init",
        1_600_000_000,
    );
    r.commit_files(
        &[("secrets/k", "k\n"), ("b.txt", "b\n")],
        "add b",
        1_600_100_000,
    );
    r.config(SECRETS_CFG);
    r.write(".gitignore", "target\nmine\n");
    let o = r.ghma(&["apply", "--from", "root"]);
    assert!(o.status.success());
    assert!(
        stderr(&o).contains(".gitignore has local changes"),
        "{}",
        stderr(&o)
    );
    assert!(
        stderr(&o).contains("rotate"),
        "the secrets warning is printed: {}",
        stderr(&o)
    );
}

#[test]
fn identity_values_that_would_corrupt_a_commit_are_rejected() {
    let r = Repo::new();
    r.linear(1, 1_600_000_000);
    for bad in ["\"Jane\\ncommitter x\"", "\"Jane <x>\""] {
        r.config(&format!(
            "version: 1\nidentity:\n  - match: {{email: me@home.org}}\n    set: {{name: {bad}, email: j@w.com}}\n"
        ));
        assert_eq!(Repo::code(&r.ghma(&["plan", "--from", "root"])), 2, "{bad}");
    }
}

#[test]
fn a_symlinked_gitignore_is_never_written_through() {
    let r = Repo::new();
    r.commit_files(&[("a.txt", "a\n")], "init", 1_600_000_000);
    r.commit_files(
        &[("secrets/k", "k\n"), ("b.txt", "b\n")],
        "add b",
        1_600_100_000,
    );
    r.config(SECRETS_CFG);
    let outside = r.home.path().join("outside");
    std::os::unix::fs::symlink(&outside, r.path().join(".gitignore")).unwrap();
    let o = r.ghma(&["apply", "--from", "root"]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(!outside.exists(), "the link target must not be created");
    assert!(stderr(&o).contains("symbolic link"), "{}", stderr(&o));
}

#[test]
fn a_config_whose_to_precedes_from_is_a_usage_error() {
    let r = Repo::new();
    r.linear(1, 1_600_000_000);
    r.config("version: 1\nfrom: 2026-01-01\nto: 2025-01-01\nschedule: {}\n");
    assert_eq!(Repo::code(&r.ghma(&["plan", "--from", "root"])), 2);
}

#[test]
fn init_keeps_the_config_out_of_commits_and_starts_inert() {
    let r = Repo::new();
    r.linear(2, 1_600_000_000);
    r.ghma_ok(&["init"]);
    assert!(
        r.git(&["status", "--porcelain"]).is_empty(),
        "the config is excluded"
    );
    let o = r.ghma(&["plan", "--from", "root"]);
    assert!(o.status.success());
    assert!(
        stderr(&o).contains("no rules are configured"),
        "{}",
        stderr(&o)
    );
    assert!(String::from_utf8_lossy(&o.stdout).contains("Nothing to do"));
}
