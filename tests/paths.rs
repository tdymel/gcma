//! Path rules: excluded paths leave history, the files stay in the working copy and get ignored.

mod common;

use common::*;

/// a, key only, b + key2, c.
fn history_with_secrets(r: &Repo) {
    r.commit_files(&[("a.txt", "a\n")], "add a", 1_600_000_000);
    r.commit_files(&[("secrets/key.pem", "k1\n")], "add key", 1_600_100_000);
    r.commit_files(
        &[("b.txt", "b\n"), ("secrets/key2.pem", "k2\n")],
        "add b and key2",
        1_600_200_000,
    );
    r.commit_files(&[("c.txt", "c\n")], "add c", 1_600_300_000);
}

fn tracked(r: &Repo, rev: &str) -> Vec<String> {
    r.git(&["ls-tree", "-r", "--name-only", rev])
        .lines()
        .map(String::from)
        .collect()
}

fn status_without_config(r: &Repo) -> String {
    r.git(&["status", "--porcelain"])
        .lines()
        .filter(|l| !l.contains("gcma.yml"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn excluded_paths_leave_history_but_not_the_project() {
    let r = Repo::new();
    history_with_secrets(&r);
    r.config(SECRETS_CFG);
    let old_tip = r.git(&["rev-parse", "HEAD"]);

    let plan = r.gcma_ok(&["plan", "--from", "root"]);
    assert!(
        plan.contains("to drop") && plan.contains("DROPPED"),
        "{plan}"
    );
    assert_eq!(
        r.git(&["rev-parse", "HEAD"]),
        old_tip,
        "plan must not move the branch"
    );

    let out = r.gcma_ok(&["apply", "--from", "root"]);
    assert!(out.contains("dropped 1"), "{out}");
    r.fsck();

    let rows = r.log();
    let subjects: Vec<&str> = rows.iter().map(|x| x.subject.as_str()).collect();
    assert_eq!(
        subjects,
        ["add a", "add b and key2", "add c"],
        "key-only commit is dropped"
    );
    assert_eq!(
        rows[1].parents,
        vec![rows[0].oid.clone()],
        "parents rewired"
    );

    // No secrets anywhere in the new history; .gitignore appears where they first did.
    for row in &rows {
        assert!(
            !tracked(&r, &row.oid)
                .iter()
                .any(|p| p.starts_with("secrets/")),
            "{}",
            row.subject
        );
    }
    assert_eq!(
        tracked(&r, "HEAD"),
        [".gitignore", "a.txt", "b.txt", "c.txt"]
    );
    assert!(!tracked(&r, &rows[0].oid).contains(&".gitignore".to_string()));
    let ignore = r.git(&["show", &format!("{}:.gitignore", rows[1].oid)]);
    assert!(ignore.contains("secrets/"), "{ignore}");
    assert!(
        tracked(&r, &rows[1].oid).contains(&".gitignore".to_string()),
        "added in the first kept commit that had excluded paths"
    );

    // The files are still there, untracked and ignored; the index follows the branch.
    assert_eq!(
        std::fs::read_to_string(r.path().join("secrets/key.pem")).unwrap(),
        "k1\n"
    );
    assert_eq!(status_without_config(&r), "", "{}", r.git(&["status"]));
    assert!(
        std::fs::read_to_string(r.path().join(".gitignore"))
            .unwrap()
            .contains("secrets/")
    );

    // Stable, and undoable.
    assert!(
        r.gcma_ok(&["apply", "--from", "root"])
            .contains("Nothing to do")
    );
    let id = r.git(&["for-each-ref", "--format=%(refname)", "refs/gcma/backup/"]);
    let id = id
        .lines()
        .next()
        .unwrap()
        .rsplit('/')
        .nth(1)
        .unwrap()
        .to_string();
    r.gcma_ok(&["restore", &id]);
    assert_eq!(r.git(&["rev-parse", "HEAD"]), old_tip);
    assert_eq!(r.log().len(), 4);
}

#[test]
fn a_tip_that_only_touches_excluded_paths_stays_as_the_gitignore_carrier() {
    let r = Repo::new();
    r.commit_files(&[("a.txt", "a\n")], "add a", 1_600_000_000);
    r.commit_files(&[("secrets/key.pem", "k\n")], "add key", 1_600_100_000);
    r.config(SECRETS_CFG);
    r.gcma_ok(&["apply", "--from", "root"]);
    r.fsck();
    // The files are still in the project, so something on the branch must ignore them.
    let rows = r.log();
    assert_eq!(rows.len(), 2);
    assert_eq!(tracked(&r, "HEAD"), [".gitignore", "a.txt"]);
    assert_eq!(tracked(&r, &rows[0].oid), ["a.txt"]);
    assert_eq!(status_without_config(&r), "");
}

#[test]
fn a_tip_that_only_touches_excluded_paths_is_dropped_without_gitignore() {
    let r = Repo::new();
    r.commit_files(&[("a.txt", "a\n")], "add a", 1_600_000_000);
    r.commit_files(&[("secrets/key.pem", "k\n")], "add key", 1_600_100_000);
    r.config("version: 1\npaths:\n  exclude: [\"secrets/\"]\n  gitignore: false\n");
    r.gcma_ok(&["apply", "--from", "root"]);
    r.fsck();
    let rows = r.log();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].subject, "add a");
    assert_eq!(tracked(&r, "HEAD"), ["a.txt"]);
    assert!(r.path().join("secrets/key.pem").exists());
}

#[test]
fn a_dropped_tip_hands_the_branch_to_a_kept_ancestor_that_carries_the_entry() {
    let r = Repo::new();
    r.commit_files(&[("a.txt", "a\n")], "add a", 1_600_000_000);
    r.commit_files(
        &[("b.txt", "b\n"), ("secrets/k1", "1\n")],
        "add b and key",
        1_600_100_000,
    );
    r.commit_files(&[("secrets/k2", "2\n")], "add key2", 1_600_200_000);
    r.config(SECRETS_CFG);
    r.gcma_ok(&["apply", "--from", "root"]);
    r.fsck();
    let rows = r.log();
    assert_eq!(rows.len(), 2, "the tip is dropped, nothing extra is kept");
    assert_eq!(tracked(&r, "HEAD"), [".gitignore", "a.txt", "b.txt"]);
    assert_eq!(status_without_config(&r), "");
}

#[test]
fn keeping_commits_that_only_touch_excluded_paths_leaves_them_empty() {
    let r = Repo::new();
    r.commit_files(&[("a.txt", "a\n")], "add a", 1_600_000_000);
    r.commit_files(&[("secrets/key.pem", "k\n")], "add key", 1_600_100_000);
    r.config("version: 1\npaths:\n  exclude: [\"secrets/\"]\n  only_excluded_commits: keep\n");
    r.gcma_ok(&["apply", "--from", "root"]);
    r.fsck();
    let rows = r.log();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1].subject, "add key");
    // The kept commit only gains the .gitignore entry.
    assert_eq!(tracked(&r, "HEAD"), [".gitignore", "a.txt"]);
}

#[test]
fn gitignore_entries_can_be_turned_off() {
    let r = Repo::new();
    history_with_secrets(&r);
    r.config("version: 1\npaths:\n  exclude: [\"secrets/\"]\n  gitignore: false\n");
    r.gcma_ok(&["apply", "--from", "root"]);
    assert_eq!(tracked(&r, "HEAD"), ["a.txt", "b.txt", "c.txt"]);
}

#[test]
fn an_existing_gitignore_is_extended_not_replaced() {
    let r = Repo::new();
    r.commit_files(
        &[(".gitignore", "target\n"), ("a.txt", "a\n")],
        "init",
        1_600_000_000,
    );
    r.commit_files(
        &[("prod.env", "S=1\n"), ("b.txt", "b\n")],
        "add b",
        1_600_100_000,
    );
    r.config("version: 1\npaths:\n  exclude: [\"*.env\"]\n");
    r.gcma_ok(&["apply", "--from", "root"]);
    r.fsck();
    let ignore = r.git(&["show", "HEAD:.gitignore"]);
    assert!(
        ignore.starts_with("target\n") && ignore.contains("*.env"),
        "{ignore}"
    );
    assert_eq!(tracked(&r, "HEAD"), [".gitignore", "a.txt", "b.txt"]);
    // The first commit never had an excluded path and keeps its .gitignore untouched.
    assert_eq!(
        r.git(&["show", &format!("{}:.gitignore", r.log()[0].oid)]),
        "target"
    );
}

#[test]
fn dropping_a_side_branch_commit_rewires_the_merge() {
    let r = Repo::new();
    r.commit_files(&[("a.txt", "a\n")], "base", 1_600_000_000);
    r.git(&["checkout", "-q", "-b", "feat"]);
    r.commit_files(&[("secrets/key.pem", "k\n")], "feat: key", 1_600_100_000);
    r.commit_files(&[("x.txt", "x\n")], "feat: x", 1_600_200_000);
    r.git(&["checkout", "-q", "main"]);
    r.commit_files(&[("y.txt", "y\n")], "main: y", 1_600_300_000);
    r.git(&["merge", "-q", "--no-ff", "-m", "merge feat", "feat"]);
    r.config(SECRETS_CFG);
    let old = r.log();
    r.gcma_ok(&["apply", "--from", "root"]);
    r.fsck();
    let rows = r.log();
    assert_eq!(rows.len(), old.len() - 1);
    let merge = rows.last().unwrap();
    assert_eq!(merge.parents.len(), 2, "still a real merge");
    assert!(!rows.iter().any(|x| x.subject == "feat: key"));
    assert_eq!(
        tracked(&r, "HEAD"),
        [".gitignore", "a.txt", "x.txt", "y.txt"]
    );
}

#[test]
fn rules_combine_with_schedule_and_identity() {
    let r = Repo::new();
    history_with_secrets(&r);
    r.config(&berlin_cfg(
        "paths:\n  exclude: [\"secrets/\"]\nidentity:\n  - match: {email: me@home.org}\n    set: {name: Jane Doe, email: jane@work.com}\n",
    ));
    r.gcma_ok(&["apply", "--from", "root"]);
    r.fsck();
    let rows = r.log();
    assert_eq!(rows.len(), 3);
    assert_scheduled(&rows);
    assert!(rows.iter().all(|x| x.ae == "jane@work.com"));
    assert!(
        r.gcma_ok(&["apply", "--from", "root"])
            .contains("Nothing to do")
    );
}

#[test]
fn an_edited_plan_cannot_change_trees() {
    let r = Repo::new();
    history_with_secrets(&r);
    r.config(SECRETS_CFG);
    let plan_path = r.path().join("plan.json");
    r.gcma_ok(&[
        "plan",
        "--from",
        "root",
        "--out",
        plan_path.to_str().unwrap(),
    ]);
    let mut plan: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&plan_path).unwrap()).unwrap();
    // Point the last entry at the unfiltered tree of the old tip.
    let old_tree = r.git(&["rev-parse", "HEAD^{tree}"]);
    let last = plan["entries"].as_array().unwrap().len() - 1;
    plan["entries"][last]["tree"] = serde_json::Value::String(old_tree);
    std::fs::write(&plan_path, serde_json::to_vec(&plan).unwrap()).unwrap();
    let tip = r.git(&["rev-parse", "HEAD"]);
    let o = r.gcma(&["apply", "--plan", plan_path.to_str().unwrap()]);
    assert_eq!(Repo::code(&o), 2, "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(r.git(&["rev-parse", "HEAD"]), tip, "nothing moved");
}

#[test]
fn modified_gitignore_in_the_working_copy_is_left_alone() {
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
    r.write(".gitignore", "target\nmine\n"); // unstaged local edit
    r.gcma_ok(&["apply", "--from", "root"]);
    assert_eq!(
        std::fs::read_to_string(r.path().join(".gitignore")).unwrap(),
        "target\nmine\n"
    );
}
