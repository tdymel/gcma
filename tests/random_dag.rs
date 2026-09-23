//! Randomized end-to-end check: random DAGs (branches, merges, octopus merges, odd timestamps) and
//! random configs must preserve every tree, the graph shape, and every commit (via the backup), be
//! idempotent, and be restorable.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};

use common::*;

static MERGES: AtomicUsize = AtomicUsize::new(0);
static OCTOPUS: AtomicUsize = AtomicUsize::new(0);
static PARTIAL: AtomicUsize = AtomicUsize::new(0);

struct Rand(u64);
impl Rand {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// Two thirds of the commits carry sign-off and co-author trailers.
fn body(n: usize, subject: &str) -> String {
    if n % 3 == 1 {
        subject.to_string()
    } else {
        format!(
            "{subject}\n\nBody of {n}.\n\nSigned-off-by: Dev <dev@x.org>\nCo-authored-by: Pair <pair@x.org>"
        )
    }
}

fn build_random_repo(r: &Repo, rng: &mut Rand, ops: usize) {
    r.commit_as("root.txt", "root", 1_500_000_000, "Old Me", "me@home.org");
    let mut branches: Vec<String> = vec!["main".into()];
    let mut n = 0;
    for _ in 0..ops {
        n += 1;
        // Odd, non-monotone timestamps, sometimes far in the future.
        let t = 1_400_000_000 + (rng.below(400_000_000) as i64);
        let who = if rng.below(3) == 0 {
            ("Jane Doe", "jane@work.com")
        } else {
            ("Old Me", "me@home.org")
        };
        match rng.below(10) {
            0 | 1 => {
                // New branch from a random existing commit.
                let commits = r.git(&["rev-list", "--all"]);
                let all: Vec<&str> = commits.lines().collect();
                let from = all[rng.below(all.len() as u64) as usize];
                let name = format!("b{n}");
                r.git(&["checkout", "-q", "-b", &name, from]);
                branches.push(name);
                r.commit_as(
                    &format!("f{n}.txt"),
                    &body(n, &format!("branch commit {n}")),
                    t,
                    who.0,
                    who.1,
                );
            }
            2 | 3 => {
                // Switch branch.
                let b = branches[rng.below(branches.len() as u64) as usize].clone();
                r.git(&["checkout", "-q", &b]);
                r.commit_as(
                    &format!("f{n}.txt"),
                    &body(n, &format!("commit {n}")),
                    t,
                    who.0,
                    who.1,
                );
            }
            4 | 5 => {
                // Merge one or two other branches into the current one (octopus when two).
                let cur = r.git(&["rev-parse", "--abbrev-ref", "HEAD"]);
                let mut others: Vec<String> =
                    branches.iter().filter(|b| **b != cur).cloned().collect();
                if others.is_empty() {
                    r.commit_as(
                        &format!("f{n}.txt"),
                        &body(n, &format!("commit {n}")),
                        t,
                        who.0,
                        who.1,
                    );
                    continue;
                }
                let k = if others.len() > 1 && rng.below(3) == 0 {
                    2
                } else {
                    1
                };
                let mut picked = Vec::new();
                for _ in 0..k {
                    picked.push(others.remove(rng.below(others.len() as u64) as usize));
                }
                let date = format!("{t} +0000");
                let mut args: Vec<&str> = vec!["merge", "-q", "--no-ff", "-m", "merge"];
                args.extend(picked.iter().map(|s| s.as_str()));
                let o = r
                    .cmd("git")
                    .args(&args)
                    .env("GIT_AUTHOR_DATE", &date)
                    .env("GIT_COMMITTER_DATE", &date)
                    .output()
                    .unwrap();
                if !o.status.success() {
                    // Conflict or nothing to merge: clean up and carry on.
                    let _ = r.git_out(&["merge", "--abort"]);
                }
            }
            _ => {
                r.commit_as(
                    &format!("f{n}.txt"),
                    &body(n, &format!("commit {n}")),
                    t,
                    who.0,
                    who.1,
                );
            }
        }
    }
    // Always finish on a branch that has everything: merge all others into main when possible.
    r.git(&["checkout", "-q", "main"]);
}

fn run_case(seed: u64) {
    let mut rng = Rand(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let r = Repo::new();
    build_random_repo(&r, &mut rng, 14 + (seed as usize % 12));
    let old = r.log();
    MERGES.fetch_add(
        old.iter().filter(|x| x.parents.len() >= 2).count(),
        Ordering::Relaxed,
    );
    OCTOPUS.fetch_add(
        old.iter().filter(|x| x.parents.len() >= 3).count(),
        Ordering::Relaxed,
    );
    let old_tip = old.last().unwrap().oid.clone();
    let heads_before = heads_except_main(&r);
    let old_msgs_all = r.messages("HEAD");
    let old_all = r.git(&["rev-list", "--count", "HEAD"]);

    let variant = seed % 5;
    let dist = ["uniform", "weekday-weighted", "bursty"][(seed % 3) as usize];
    let cfg = match variant {
        0 => "version: 1\nidentity:\n  - match: {email: me@home.org}\n    set: {name: Jane Doe, email: jane@work.com}\n".to_string(),
        1 => berlin_cfg("").replace("bursty", dist),
        2 => format!(
            "{}identity:\n  - match: {{email: me@home.org}}\n    set: {{name: Jane Doe, email: jane@work.com}}\nmessages:\n  strip_trailers: [Signed-off-by]\n",
            berlin_cfg("").replace("bursty", dist)
        ),
        3 => "version: 1\nmessages:\n  strip_trailers: [Signed-off-by]\n".to_string(),
        _ => "version: 1\nmessages:\n  strip_trailers: [Co-authored-by]\n  add_trailers:\n    - \"Assisted-By: Claude <noreply@anthropic.com>\"\n".to_string(),
    };
    r.config(&cfg);

    let o = r.ghma(&["apply", "--from", "root"]);
    assert!(
        o.status.success(),
        "seed {seed}: apply failed: {}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    let new = r.log();
    assert_eq!(new.len(), old.len(), "seed {seed}: commit count");
    assert_same_content(&old, &new);
    r.fsck();

    if variant == 1 || variant == 2 {
        assert_scheduled(&new);
    }
    if variant == 0 || variant == 2 {
        assert!(
            new.iter().all(|x| x.an == "Jane Doe" && x.cn == "Jane Doe"),
            "seed {seed}: identity"
        );
    }
    // Messages: the trailer rules did exactly what they say, and nothing else changed.
    let map = map_commits(&old, &new);
    let (old_msgs, new_msgs) = (old_msgs_all.clone(), r.messages("HEAD"));
    for (o, n) in &map {
        check_message(seed, variant, &old_msgs[o], &new_msgs[n]);
    }
    // Branches other than the rewritten one are untouched.
    let heads_after = heads_except_main(&r);
    assert_eq!(
        heads_before, heads_after,
        "seed {seed}: other branches moved"
    );

    // Nothing lost: the entire original history is still reachable from the backup.
    let backups = r.git(&[
        "for-each-ref",
        "--format=%(refname) %(objectname)",
        "refs/ghma/backup/",
    ]);
    if backups.is_empty() {
        // Nothing needed rewriting (e.g. a case where every commit already conformed).
        assert_eq!(new_tip_of(&r), old_tip);
    } else {
        let old_ref = backups
            .lines()
            .find(|l| l.contains("/old "))
            .unwrap()
            .split(' ')
            .nth(1)
            .unwrap()
            .to_string();
        assert_eq!(
            old_ref, old_tip,
            "seed {seed}: backup points at the old tip"
        );
        assert_eq!(
            r.git(&["rev-list", "--count", &old_ref]),
            old_all,
            "seed {seed}: old history intact"
        );
    }

    // Idempotent.
    let again = r.ghma(&["apply", "--from", "root"]);
    assert!(again.status.success());
    assert!(
        String::from_utf8_lossy(&again.stdout).contains("Nothing to do"),
        "seed {seed}: second apply must be a no-op"
    );
    assert!(
        r.ghma(&["plan", "--check", "--from", "root"])
            .status
            .success(),
        "seed {seed}: --check"
    );

    // Hook-like stage: append new nonconforming commits to the settled history. Everything that was
    // already settled must keep its OID; only the new commits are rewritten.
    if variant != 3 {
        let settled: Vec<String> = r.log().iter().map(|x| x.oid.clone()).collect();
        for k in 0..3 {
            r.commit_as(
                &format!("late{k}.txt"),
                &format!("late {k}"),
                1_300_000_000 + k * 7,
                "Old Me",
                "me@home.org",
            );
        }
        let plan = r.ghma_ok(&["plan", "--from", "root"]);
        assert!(plan.contains("3 to rewrite"), "seed {seed}: {plan}");
        r.ghma_ok(&["apply", "--from", "root"]);
        let after = r.log();
        assert_eq!(after.len(), settled.len() + 3);
        for (a, b) in settled.iter().zip(&after) {
            assert_eq!(a, &b.oid, "seed {seed}: settled commits keep their OIDs");
        }
        if variant == 1 || variant == 2 {
            assert_scheduled(&after);
        }
        PARTIAL.fetch_add(1, Ordering::Relaxed);
        assert!(
            r.ghma(&["plan", "--check", "--from", "root"])
                .status
                .success()
        );
        // Put the repo back where the restore check below expects it.
        r.git(&["reset", "-q", "--hard", &settled.last().unwrap().clone()]);
    }

    // Restore returns the exact original tip.
    if !backups.is_empty() {
        let listing = r.ghma_ok(&["restore"]);
        let id = listing
            .lines()
            .find(|l| l.contains(&format!("old {old_tip}")))
            .unwrap_or_else(|| panic!("seed {seed}: no backup for the original tip in:\n{listing}"))
            .split_whitespace()
            .next()
            .unwrap()
            .to_string();
        r.ghma_ok(&["restore", &id]);
        assert_eq!(new_tip_of(&r), old_tip, "seed {seed}: restore");
        assert_eq!(r.log().len(), old.len());
    }
}

fn heads_except_main(r: &Repo) -> String {
    r.git(&[
        "for-each-ref",
        "--format=%(refname) %(objectname)",
        "refs/heads/",
    ])
    .lines()
    .filter(|l| !l.starts_with("refs/heads/main "))
    .collect::<Vec<_>>()
    .join("\n")
}

fn check_message(seed: u64, variant: u64, old: &str, new: &str) {
    let lines = |m: &str| m.lines().map(str::to_string).collect::<Vec<_>>();
    let (ol, nl) = (lines(old), lines(new));
    assert_eq!(ol[0], nl[0], "seed {seed}: subject changed");
    let has = |l: &[String], key: &str| {
        l.iter().any(|x| {
            x.to_ascii_lowercase()
                .starts_with(&format!("{}:", key.to_ascii_lowercase()))
        })
    };
    let count = |l: &[String], key: &str| {
        l.iter()
            .filter(|x| {
                x.to_ascii_lowercase()
                    .starts_with(&format!("{}:", key.to_ascii_lowercase()))
            })
            .count()
    };
    match variant {
        // Sign-offs go, everything else stays.
        2 | 3 => {
            assert!(!has(&nl, "signed-off-by"), "seed {seed}: {new:?}");
            let expected: Vec<&String> = ol
                .iter()
                .filter(|l| !l.starts_with("Signed-off-by:"))
                .collect();
            let got: Vec<&String> = nl.iter().filter(|l| !l.is_empty()).collect();
            let expected: Vec<&String> = expected.into_iter().filter(|l| !l.is_empty()).collect();
            assert_eq!(
                got, expected,
                "seed {seed}: only the sign-off line may differ"
            );
        }
        // Co-authors go, sign-offs stay, one Assisted-By is appended as the last line.
        4 => {
            assert!(!has(&nl, "co-authored-by"), "seed {seed}: {new:?}");
            assert_eq!(
                count(&nl, "signed-off-by"),
                count(&ol, "signed-off-by"),
                "seed {seed}"
            );
            assert_eq!(count(&nl, "assisted-by"), 1, "seed {seed}: {new:?}");
            assert_eq!(
                nl.last().unwrap(),
                "Assisted-By: Claude <noreply@anthropic.com>",
                "seed {seed}: appended last"
            );
            let kept: Vec<&String> = ol
                .iter()
                .filter(|l| !l.starts_with("Co-authored-by:") && !l.is_empty())
                .collect();
            let got: Vec<&String> = nl
                .iter()
                .filter(|l| !l.is_empty() && !l.starts_with("Assisted-By:"))
                .collect();
            assert_eq!(
                got, kept,
                "seed {seed}: the rest of the message is untouched"
            );
        }
        // Identity and schedule runs leave messages alone.
        _ => assert_eq!(
            old.trim_end(),
            new.trim_end(),
            "seed {seed}: message changed"
        ),
    }
}

fn new_tip_of(r: &Repo) -> String {
    r.git(&["rev-parse", "HEAD"])
}

#[test]
fn random_histories_keep_trees_shape_and_idempotency() {
    for seed in 1..=36 {
        run_case(seed);
    }
    // Guard against a vacuous generator.
    assert!(
        MERGES.load(Ordering::Relaxed) >= 10,
        "merges: {}",
        MERGES.load(Ordering::Relaxed)
    );
    assert!(
        OCTOPUS.load(Ordering::Relaxed) >= 1,
        "octopus merges: {}",
        OCTOPUS.load(Ordering::Relaxed)
    );
    assert!(PARTIAL.load(Ordering::Relaxed) >= 20);
}
