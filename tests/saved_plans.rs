//! A plan saved with `--out` and applied later: it applies when nothing changed, and is refused,
//! before anything is written, when it no longer fits the repository or the config (the branch
//! moved, another branch is checked out, it names another ref, the path rules changed).

mod common;

use common::*;

#[test]
fn tip_moved_after_planning_is_refused() {
    let r = Repo::new();
    r.linear(3, T0);
    r.config(IDENTITY_CFG);
    let plan = r.path().join("plan.json");
    r.gcma_ok(&["plan", "--from", "root", "--out", plan.to_str().unwrap()]);
    r.commit_at("extra.txt", "extra", 1_600_900_000);
    let tip = r.git(&["rev-parse", "HEAD"]);
    let o = r.gcma(&["apply", "--plan", plan.to_str().unwrap()]);
    assert_eq!(Repo::code(&o), 4, "{}", stderr(&o));
    assert_eq!(r.git(&["rev-parse", "HEAD"]), tip);
}

#[test]
fn saved_plan_applies_later() {
    let r = Repo::new();
    r.linear(3, T0);
    r.config(IDENTITY_CFG);
    let plan = r.path().join("plan.json");
    r.gcma_ok(&["plan", "--from", "root", "--out", plan.to_str().unwrap()]);
    r.gcma_ok(&["apply", "--plan", plan.to_str().unwrap()]);
    assert!(r.log().iter().all(|x| x.an == "Jane Doe"));
}

#[test]
fn a_plan_cannot_retarget_other_refs_or_bring_its_own_path_rules() {
    let r = Repo::new();
    r.commit_files(&[("a.txt", "a\n")], "add a", T0);
    r.commit_files(
        &[("secrets/k", "k\n"), ("b.txt", "b\n")],
        "add b",
        1_600_100_000,
    );
    r.config(SECRETS_CFG);
    let plan_path = r.path().join("plan.json");
    let plan_arg = plan_path.to_str().unwrap().to_string();
    r.gcma_ok(&["plan", "--from", "root", "--out", &plan_arg]);
    let tip = r.git(&["rev-parse", "HEAD"]);

    // Another ref at the tip (a tag) must not be rewritten.
    r.git(&["tag", "v1"]);
    let mut plan: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&plan_path).unwrap()).unwrap();
    plan["branch_ref"] = "refs/tags/v1".into();
    std::fs::write(&plan_path, serde_json::to_vec(&plan).unwrap()).unwrap();
    let o = r.gcma(&["apply", "--plan", &plan_arg]);
    assert_eq!(Repo::code(&o), 2, "{}", stderr(&o));
    assert_eq!(r.git(&["rev-parse", "v1"]), tip);

    // A plan made under other rules than the config's is refused.
    r.gcma_ok(&["plan", "--from", "root", "--out", &plan_arg]);
    r.config("version: 1\npaths:\n  exclude: [\"b.txt\"]\n");
    let o = r.gcma(&["apply", "--plan", &plan_arg]);
    assert_eq!(Repo::code(&o), 2, "{}", stderr(&o));
    assert!(stderr(&o).contains("path rules"), "{}", stderr(&o));
    assert_eq!(r.git(&["rev-parse", "HEAD"]), tip);
}

#[test]
fn a_plan_for_another_branch_is_refused() {
    let r = Repo::new();
    r.linear(2, T0);
    r.config(IDENTITY_CFG);
    let plan_path = r.path().join("plan.json");
    let plan_arg = plan_path.to_str().unwrap().to_string();
    r.gcma_ok(&["plan", "--from", "root", "--out", &plan_arg]);
    r.git(&["checkout", "-q", "-b", "other"]);
    let o = r.gcma(&["apply", "--plan", &plan_arg]);
    assert_eq!(Repo::code(&o), 3, "{}", stderr(&o));
}
