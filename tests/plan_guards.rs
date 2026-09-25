//! What `apply` and `plan` refuse beyond plain tampering (see `plans.rs`): hand-edited drops,
//! plans gone stale, impossible ranges.
//! Every refusal leaves the branch and the refs exactly as they were.

mod common;

use common::*;

const IDENTITY_CFG: &str = "version: 1\nidentity:\n  - match: {email: me@home.org}\n    set: {name: Jane Doe, email: jane@work.com}\n";
const SECRETS_CFG: &str = "version: 1\npaths:\n  exclude: [\"secrets/\"]\n";
const T0: i64 = 1_600_000_000;

fn stderr(o: &std::process::Output) -> String {
    String::from_utf8_lossy(&o.stderr).to_string()
}

fn untouched(r: &Repo, tip: &str) {
    assert_eq!(r.git(&["rev-parse", "HEAD"]), tip, "the branch moved");
    assert!(
        r.git(&["for-each-ref", "refs/gcma/"]).is_empty(),
        "gcma refs appeared"
    );
}

/// Saves a plan for `--from root`, lets `edit` change its JSON, and returns the file's path.
fn edited_plan(r: &Repo, edit: impl FnOnce(&mut serde_json::Value)) -> String {
    let path = r.path().join("plan.json").to_str().unwrap().to_string();
    r.gcma_ok(&["plan", "--from", "root", "--out", &path]);
    let mut plan: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    edit(&mut plan);
    std::fs::write(&path, serde_json::to_vec(&plan).unwrap()).unwrap();
    path
}

#[test]
fn a_plan_cannot_drop_a_commit_that_changes_more_than_excluded_paths() {
    let r = Repo::new();
    r.commit_files(&[("a.txt", "a\n")], "add a", T0);
    r.commit_files(&[("secrets/k", "k\n")], "add key", T0 + 1000);
    let victim = r.commit_files(&[("b.txt", "b\n")], "add b", T0 + 2000);
    r.commit_files(&[("c.txt", "c\n")], "add c", T0 + 3000);
    let tip = r.git(&["rev-parse", "HEAD"]);
    r.config(SECRETS_CFG);
    // `a` stays frozen, so the entries are b and c (the key commit is dropped); move b to `dropped`.
    let plan = edited_plan(&r, |p| {
        let entries = p["entries"].as_array_mut().unwrap();
        assert_eq!(entries.len(), 2);
        let gone = entries.remove(0);
        entries[0]["parents"] = gone["parents"].clone();
        p["dropped"]
            .as_array_mut()
            .unwrap()
            .push(victim.clone().into());
    });
    let o = r.gcma(&["apply", "--plan", &plan]);
    assert_eq!(Repo::code(&o), 2, "{}", stderr(&o));
    assert!(stderr(&o).contains("not excluded"), "{}", stderr(&o));
    untouched(&r, &tip);
}

#[test]
fn a_plan_cannot_drop_a_merge_commit() {
    let r = Repo::new();
    r.commit_files(&[("a.txt", "a\n")], "add a", T0);
    r.git(&["checkout", "-q", "-b", "side"]);
    r.commit_files(&[("secrets/s", "s\n")], "side secret", T0 + 1000);
    r.git(&["checkout", "-q", "main"]);
    r.commit_files(&[("m.txt", "m\n")], "add m", T0 + 2000);
    r.git(&["merge", "-q", "--no-ff", "-m", "merge side", "side"]);
    let merge = r.git(&["rev-parse", "HEAD"]);
    r.commit_files(&[("z.txt", "z\n")], "add z", T0 + 3000);
    let tip = r.git(&["rev-parse", "HEAD"]);
    r.config(SECRETS_CFG);
    // Entries are the merge and z (the side commit is dropped); move the merge to `dropped`.
    let plan = edited_plan(&r, |p| {
        let entries = p["entries"].as_array_mut().unwrap();
        assert_eq!(entries.len(), 2);
        let gone = entries.remove(0);
        entries[0]["parents"] = gone["parents"].clone();
        p["dropped"]
            .as_array_mut()
            .unwrap()
            .push(merge.clone().into());
    });
    let o = r.gcma(&["apply", "--plan", &plan]);
    assert_eq!(Repo::code(&o), 2, "{}", stderr(&o));
    assert!(stderr(&o).contains("merge commit"), "{}", stderr(&o));
    untouched(&r, &tip);
}

#[test]
fn a_plan_that_became_pushed_after_it_was_saved_exits_5_unless_allowed() {
    let r = Repo::new();
    r.linear(3, T0);
    r.config(IDENTITY_CFG);
    let tip = r.git(&["rev-parse", "HEAD"]);
    let plan = edited_plan(&r, |_| {});
    // Between planning and applying the commits are pushed.
    r.bare_remote();
    r.git(&["push", "-q", "-u", "origin", "main"]);
    let o = r.gcma(&["apply", "--plan", &plan]);
    assert_eq!(Repo::code(&o), 5, "{}", stderr(&o));
    assert!(stderr(&o).contains("--rewrite-pushed"), "{}", stderr(&o));
    untouched(&r, &tip);

    r.gcma_ok(&["apply", "--plan", &plan, "--rewrite-pushed"]);
    assert_ne!(r.git(&["rev-parse", "HEAD"]), tip);
    assert!(r.log().iter().all(|x| x.an == "Jane Doe"));
}

#[test]
fn a_branch_where_every_commit_only_touches_excluded_paths_is_refused() {
    let r = Repo::new();
    r.commit_files(&[("secrets/one", "1\n")], "one", T0);
    r.commit_files(&[("secrets/two", "2\n")], "two", T0 + 1000);
    r.config("version: 1\npaths:\n  exclude: [\"secrets/\"]\n  gitignore: false\n");
    let tip = r.git(&["rev-parse", "HEAD"]);
    for cmd in ["plan", "apply"] {
        let o = r.gcma(&[cmd, "--from", "root"]);
        assert_eq!(Repo::code(&o), 3, "{cmd}: {}", stderr(&o));
        assert!(
            stderr(&o).contains("nothing would be left"),
            "{cmd}: {}",
            stderr(&o)
        );
    }
    untouched(&r, &tip);
}

#[test]
fn from_must_be_an_ancestor_of_the_branch() {
    let r = Repo::new();
    r.linear(2, T0);
    r.git(&["checkout", "-q", "-b", "other", "HEAD~1"]);
    r.commit_at("other.txt", "elsewhere", T0 + 500);
    let elsewhere = r.git(&["rev-parse", "HEAD"]);
    r.git(&["checkout", "-q", "main"]);
    r.config(IDENTITY_CFG);
    let tip = r.git(&["rev-parse", "HEAD"]);
    for cmd in ["plan", "apply"] {
        let o = r.gcma(&[cmd, "--from", &elsewhere]);
        assert_eq!(Repo::code(&o), 2, "{cmd}: {}", stderr(&o));
        assert!(
            stderr(&o).contains("not an ancestor"),
            "{cmd}: {}",
            stderr(&o)
        );
        let o = r.gcma(&[cmd, "--from", "no-such-rev"]);
        assert_eq!(Repo::code(&o), 2, "{cmd}: {}", stderr(&o));
    }
    untouched(&r, &tip);
}

#[test]
fn a_schedule_ending_before_the_commit_it_must_follow_is_refused() {
    let r = Repo::new();
    // The first commit is far later than the configured `to`, and stays out of the range.
    let late = r.commit_at("late.txt", "late", 1_900_000_000);
    r.commit_at("next.txt", "next", 1_900_100_000);
    r.config("version: 1\nfrom: 2026-01-01\nto: 2026-01-10\nschedule:\n  days: [mon, tue, wed, thu, fri]\n  hours: \"09:00-17:00\"\n");
    let tip = r.git(&["rev-parse", "HEAD"]);
    for cmd in ["plan", "apply"] {
        let o = r.gcma(&[cmd, "--from", &late]);
        assert_eq!(Repo::code(&o), 3, "{cmd}: {}", stderr(&o));
        assert!(
            stderr(&o).contains("not after the floor"),
            "{cmd}: {}",
            stderr(&o)
        );
    }
    untouched(&r, &tip);
}
