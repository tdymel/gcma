//! Plans are files someone may edit: every kind of tampering, and a plan that does not fit the
//! repository or the config it is applied to (another branch or ref, other path rules), must be
//! refused with a specific exit code and message, before anything is written.

mod common;

use common::*;
use serde_json::{Value, json};

/// A history with a merge: a - b - (s1 | m1) - merge - z.
fn merge_repo() -> Repo {
    let r = Repo::new();
    r.commit_at("a.txt", "a", T0);
    r.commit_at("b.txt", "b", 1_600_100_000);
    r.git(&["checkout", "-q", "-b", "side"]);
    r.commit_at("s1.txt", "s1", 1_600_200_000);
    r.git(&["checkout", "-q", "main"]);
    r.commit_at("m1.txt", "m1", 1_600_300_000);
    let date = "1600400000 +0000";
    let o = r
        .cmd("git")
        .args(["merge", "-q", "--no-ff", "-m", "merge", "side"])
        .env("GIT_AUTHOR_DATE", date)
        .env("GIT_COMMITTER_DATE", date)
        .output()
        .unwrap();
    assert!(o.status.success());
    r.commit_at("z.txt", "z", 1_600_500_000);
    r.config(IDENTITY_CFG);
    r
}

/// Saves a plan, lets `edit` change its JSON, and applies it. Nothing may change unless `code` is 0.
fn tampered(edit: impl FnOnce(&mut Value), code: i32, message: &str) {
    let r = merge_repo();
    let plan = r.path().join("plan.json");
    r.gcma_ok(&["plan", "--from", "root", "--out", plan.to_str().unwrap()]);
    let mut json: Value = serde_json::from_slice(&std::fs::read(&plan).unwrap()).unwrap();
    edit(&mut json);
    std::fs::write(&plan, serde_json::to_vec(&json).unwrap()).unwrap();
    let before = r.refs();
    let o = r.gcma(&["apply", "--plan", plan.to_str().unwrap()]);
    assert_eq!(Repo::code(&o), code, "{message}: {}", stderr(&o));
    assert!(
        stderr(&o).contains(message),
        "expected {message:?} in: {}",
        stderr(&o)
    );
    assert_eq!(
        r.refs(),
        before,
        "a refused plan must not move or create any ref"
    );
    r.fsck();
}

#[test]
fn a_dropped_entry_leaves_a_dangling_parent_reference() {
    tampered(
        |p| {
            p["entries"].as_array_mut().unwrap().remove(2);
        },
        2,
        "forward parent reference",
    );
}

#[test]
fn swapped_merge_parents_are_refused() {
    tampered(
        |p| p["entries"][4]["parents"].as_array_mut().unwrap().reverse(),
        2,
        "parents do not match",
    );
}

#[test]
fn an_extra_or_missing_parent_is_refused() {
    tampered(
        |p| {
            p["entries"][2]["parents"]
                .as_array_mut()
                .unwrap()
                .push(json!({"in": 0}))
        },
        2,
        "parents do not match",
    );
    tampered(
        |p| p["entries"][2]["parents"] = json!([]),
        2,
        "parents do not match",
    );
}

#[test]
fn a_forged_or_foreign_tip_means_the_branch_moved() {
    tampered(|p| p["tip_oid"] = json!("0".repeat(40)), 4, "re-plan");
    tampered(
        |p| {
            let o = p["entries"][3]["old_oid"].clone();
            p["tip_oid"] = o;
        },
        4,
        "re-plan",
    );
}

#[test]
fn only_branch_refs_are_accepted() {
    tampered(
        |p| p["branch_ref"] = json!("refs/tags/x"),
        2,
        "not a branch ref",
    );
    tampered(
        |p| p["branch_ref"] = json!("refs/heads/main\nrefs/heads/other"),
        2,
        "not a branch ref",
    );
    tampered(|p| p["branch_ref"] = json!("main"), 2, "not a branch ref");
}

#[test]
fn a_parent_reference_pointing_forward_is_refused() {
    tampered(
        |p| p["entries"][1]["parents"] = json!([{"in": 3}]),
        2,
        "forward parent reference",
    );
    tampered(
        |p| p["entries"][1]["parents"] = json!([{"in": 1}]),
        2,
        "forward parent reference",
    );
}

#[test]
fn object_ids_must_be_well_formed_hex() {
    tampered(
        |p| p["entries"][0]["old_oid"] = json!("--output=x"),
        2,
        "invalid object id",
    );
    tampered(
        |p| p["entries"][0]["parents"] = json!([{"base": "../../etc"}]),
        2,
        "invalid object id",
    );
    tampered(|p| p["tip_oid"] = json!("abc"), 2, "invalid object id");
}

#[test]
fn an_entry_naming_a_commit_that_does_not_exist_is_an_error() {
    tampered(
        |p| p["entries"][2]["old_oid"] = json!("1".repeat(40)),
        1,
        &"1".repeat(40),
    );
}

#[test]
fn a_parent_commit_that_vanished_is_a_precondition_failure() {
    tampered(
        |p| p["entries"][1]["parents"] = json!([{"base": "2".repeat(40)}]),
        3,
        "no longer exists",
    );
}

#[test]
fn the_same_commit_twice_is_refused() {
    tampered(
        |p| {
            let last = p["entries"][5].clone();
            p["entries"].as_array_mut().unwrap().push(last);
        },
        2,
        "more than once",
    );
    tampered(
        |p| p["dropped"] = json!(["3".repeat(40), "3".repeat(40)]),
        2,
        "more than once",
    );
}

#[test]
fn unknown_versions_and_values_are_refused() {
    tampered(|p| p["version"] = json!(0), 2, "unsupported plan version");
    tampered(|p| p["version"] = json!(9), 2, "unsupported plan version");
    tampered(|p| p["signing"] = json!("nope"), 2, "unknown variant");
    tampered(|p| p["entries"][0]["message_b64"] = json!("%%%"), 2, "json");
    tampered(
        |p| p["entries"][0]["author"]["time"] = json!("yesterday"),
        2,
        "json",
    );
}

#[test]
fn trees_and_drops_need_path_rules_in_the_plan() {
    tampered(
        |p| p["entries"][0]["tree"] = json!("3".repeat(40)),
        2,
        "no path rules",
    );
    tampered(
        |p| p["dropped"] = json!(["3".repeat(40)]),
        2,
        "no path rules",
    );
    tampered(
        |p| p["entries"][0]["gitignore"] = json!(true),
        2,
        "no path rules",
    );
}

#[test]
fn a_truncated_or_missing_plan_file_is_a_usage_error() {
    let r = merge_repo();
    let plan = r.path().join("plan.json");
    std::fs::write(&plan, "{").unwrap();
    let o = r.gcma(&["apply", "--plan", plan.to_str().unwrap()]);
    assert_eq!(Repo::code(&o), 2, "{}", stderr(&o));
    let o = r.gcma(&["apply", "--plan", "nowhere.json"]);
    assert_eq!(Repo::code(&o), 2, "{}", stderr(&o));
    assert!(stderr(&o).contains("nowhere.json"));
}

#[test]
fn identities_that_would_corrupt_a_commit_header_are_refused() {
    let why = "cannot be part of a commit header";
    tampered(
        |p| p["entries"][0]["author"]["name"] = json!("Evil\n<x@y> 1 +0000\ncommitter Z"),
        2,
        why,
    );
    tampered(
        |p| p["entries"][0]["author"]["email"] = json!("a>b"),
        2,
        why,
    );
    tampered(
        |p| p["entries"][1]["committer"]["name"] = json!("a<b"),
        2,
        why,
    );
    tampered(
        |p| p["entries"][1]["committer"]["email"] = json!("nul\u{0}byte"),
        2,
        why,
    );
    tampered(
        |p| p["entries"][0]["author"]["name"] = json!({"base64": "bgpuZXc="}),
        2,
        why,
    );
}

#[test]
fn times_and_offsets_must_be_sane() {
    tampered(
        |p| p["entries"][0]["committer"]["tz"] = json!(99999),
        2,
        "out of range",
    );
    tampered(
        |p| p["entries"][0]["author"]["time"] = json!(-5),
        2,
        "out of range",
    );
}

#[test]
fn swapped_sibling_entries_are_refused_or_produce_a_valid_history() {
    let r = merge_repo();
    let plan = r.path().join("plan.json");
    r.gcma_ok(&["plan", "--from", "root", "--out", plan.to_str().unwrap()]);
    let mut json: Value = serde_json::from_slice(&std::fs::read(&plan).unwrap()).unwrap();
    json["entries"].as_array_mut().unwrap().swap(2, 3);
    std::fs::write(&plan, serde_json::to_vec(&json).unwrap()).unwrap();
    let before = r.refs();
    let o = r.gcma(&["apply", "--plan", plan.to_str().unwrap()]);
    if o.status.success() {
        r.fsck();
        assert_eq!(r.log().len(), 6);
    } else {
        assert_ne!(Repo::code(&o), 101, "no panic: {}", stderr(&o));
        assert_eq!(r.refs(), before, "{}", stderr(&o));
    }
}

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
fn tampered_plan_is_rejected_before_anything_is_written() {
    let r = Repo::new();
    r.linear(3, T0);
    r.config(IDENTITY_CFG);
    let tip = r.git(&["rev-parse", "HEAD"]);
    let path = r.path().join("plan.json");
    r.gcma_ok(&["plan", "--from", "root", "--out", path.to_str().unwrap()]);
    let mut v: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    // Point entry 2's parent at entry 0 instead of entry 1.
    v["entries"][2]["parents"] = serde_json::json!([{"in": 0}]);
    std::fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
    let o = r.gcma(&["apply", "--plan", path.to_str().unwrap()]);
    assert!(!o.status.success());
    assert_eq!(r.git(&["rev-parse", "HEAD"]), tip);
    assert_eq!(r.backup_count(), 0);
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
