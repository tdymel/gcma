//! Randomized end-to-end check: random DAGs (branches, merges, octopus merges, odd timestamps) and
//! random configs must preserve every tree, the graph shape, and every commit (via the backup), be
//! idempotent, and be restorable.

mod common;

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use common::*;

static MERGES: AtomicUsize = AtomicUsize::new(0);
static OCTOPUS: AtomicUsize = AtomicUsize::new(0);
static PARTIAL: AtomicUsize = AtomicUsize::new(0);

/// A random history before the run under test, and what the checks afterwards compare against.
struct Case {
    seed: u64,
    /// Which rule set the config uses (see `config_for`).
    variant: u64,
    old: Vec<Row>,
    old_tip: String,
    old_count: String,
    old_msgs: HashMap<String, String>,
    heads_before: String,
}

impl Case {
    /// Whether the config schedules commit times (and so `assert_scheduled` applies).
    fn schedules(&self) -> bool {
        self.variant == 1 || self.variant == 2
    }

    /// Whether the config rewrites the identity to Jane Doe.
    fn rewrites_identity(&self) -> bool {
        self.variant == 0 || self.variant == 2
    }
}

/// Builds the random history of `seed` and notes what it looks like.
fn start_case(seed: u64) -> (Repo, Case) {
    let mut rng = Rand::new(seed);
    let r = Repo::for_seed(seed);
    r.random_dag(&mut rng, 14 + (seed as usize % 12));
    let old = r.log();
    MERGES.fetch_add(
        old.iter().filter(|x| x.parents.len() >= 2).count(),
        Ordering::Relaxed,
    );
    OCTOPUS.fetch_add(
        old.iter().filter(|x| x.parents.len() >= 3).count(),
        Ordering::Relaxed,
    );
    let case = Case {
        seed,
        variant: seed % 5,
        old_tip: old.last().unwrap().oid.clone(),
        old_count: r.git(&["rev-list", "--count", "HEAD"]),
        old_msgs: r.messages("HEAD"),
        heads_before: heads_except_main(&r),
        old,
    };
    (r, case)
}

/// The config of a variant: identity, schedule, identity and schedule with sign-off stripping,
/// sign-off stripping alone, or trailer swapping.
fn config_for(seed: u64, variant: u64) -> String {
    let dist = ["uniform", "weekday-weighted", "bursty"][(seed % 3) as usize];
    match variant {
        0 => IDENTITY_CFG.to_string(),
        1 => berlin_cfg("").replace("bursty", dist),
        2 => format!(
            "{}{IDENTITY_RULE}messages:\n  strip_trailers: [Signed-off-by]\n",
            berlin_cfg("").replace("bursty", dist)
        ),
        3 => "version: 1\nmessages:\n  strip_trailers: [Signed-off-by]\n".to_string(),
        _ => "version: 1\nmessages:\n  strip_trailers: [Co-authored-by]\n  add_trailers:\n    - \"Assisted-By: Claude <noreply@anthropic.com>\"\n".to_string(),
    }
}

/// Applies the config and checks the rewritten history: same shape and trees, fsck-clean, on
/// schedule and with the new identity where the config asks for it. Returns the new history.
fn apply_and_check_shape(r: &Repo, case: &Case) -> Vec<Row> {
    let seed = case.seed;
    let o = r.gcma(&["apply", "--from", "root"]);
    assert!(
        o.status.success(),
        "seed {seed}: apply failed: {}{}",
        stdout(&o),
        stderr(&o)
    );
    let new = r.log();
    assert_eq!(new.len(), case.old.len(), "seed {seed}: commit count");
    assert_same_content(&case.old, &new);
    r.fsck();

    if case.schedules() {
        assert_scheduled(&new);
    }
    if case.rewrites_identity() {
        assert!(
            new.iter().all(|x| x.an == "Jane Doe" && x.cn == "Jane Doe"),
            "seed {seed}: identity"
        );
    }
    new
}

/// The trailer rules did exactly what they say and nothing else changed in the messages, and the
/// branches other than the rewritten one are untouched.
fn check_messages_and_other_branches(r: &Repo, case: &Case, new: &[Row]) {
    let seed = case.seed;
    let map = map_commits(&case.old, new);
    let new_msgs = r.messages("HEAD");
    for (o, n) in &map {
        check_message(seed, case.variant, &case.old_msgs[o], &new_msgs[n]);
    }
    let heads_after = heads_except_main(r);
    assert_eq!(
        case.heads_before, heads_after,
        "seed {seed}: other branches moved"
    );
}

/// Nothing lost: the entire original history is still reachable from the backup. Returns whether
/// a backup exists (it does not when every commit already conformed).
fn check_backup(r: &Repo, case: &Case) -> bool {
    let seed = case.seed;
    let backups = r.git(&[
        "for-each-ref",
        "--format=%(refname) %(objectname)",
        "refs/gcma/backup/",
    ]);
    if backups.is_empty() {
        assert_eq!(new_tip_of(r), case.old_tip);
        return false;
    }
    let old_ref = backups
        .lines()
        .find(|l| l.contains("/old "))
        .unwrap()
        .split(' ')
        .nth(1)
        .unwrap()
        .to_string();
    assert_eq!(
        old_ref, case.old_tip,
        "seed {seed}: backup points at the old tip"
    );
    assert_eq!(
        r.git(&["rev-list", "--count", &old_ref]),
        case.old_count,
        "seed {seed}: old history intact"
    );
    true
}

/// A second `apply` is a no-op and `plan --check` passes.
fn check_idempotent(r: &Repo, case: &Case) {
    let seed = case.seed;
    let again = r.gcma(&["apply", "--from", "root"]);
    assert!(again.status.success());
    assert!(
        stdout(&again).contains("Nothing to do"),
        "seed {seed}: second apply must be a no-op"
    );
    assert!(
        r.gcma(&["plan", "--check", "--from", "root"])
            .status
            .success(),
        "seed {seed}: --check"
    );
}

/// Hook-like stage: appends new nonconforming commits to the settled history. Everything that was
/// already settled must keep its OID; only the new commits are rewritten. Leaves the repository
/// where the restore check expects it.
fn check_new_commits_are_rewritten_alone(r: &Repo, case: &Case) {
    let seed = case.seed;
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
    let plan = r.gcma_ok(&["plan", "--from", "root"]);
    assert!(plan.contains("3 to rewrite"), "seed {seed}: {plan}");
    r.gcma_ok(&["apply", "--from", "root"]);
    let after = r.log();
    assert_eq!(after.len(), settled.len() + 3);
    for (a, b) in settled.iter().zip(&after) {
        assert_eq!(a, &b.oid, "seed {seed}: settled commits keep their OIDs");
    }
    if case.schedules() {
        assert_scheduled(&after);
    }
    PARTIAL.fetch_add(1, Ordering::Relaxed);
    assert!(
        r.gcma(&["plan", "--check", "--from", "root"])
            .status
            .success()
    );
    // Put the repo back where the restore check expects it.
    r.git(&["reset", "-q", "--hard", &settled.last().unwrap().clone()]);
}

/// Restore returns the exact original tip.
fn check_restore(r: &Repo, case: &Case) {
    let seed = case.seed;
    let old_tip = &case.old_tip;
    let listing = r.gcma_ok(&["restore"]);
    let id = listing
        .lines()
        .find(|l| l.contains(&format!("old {old_tip}")))
        .unwrap_or_else(|| panic!("seed {seed}: no backup for the original tip in:\n{listing}"))
        .split_whitespace()
        .next()
        .unwrap()
        .to_string();
    r.gcma_ok(&["restore", &id]);
    assert_eq!(new_tip_of(r), *old_tip, "seed {seed}: restore");
    assert_eq!(r.log().len(), case.old.len());
}

fn run_case(seed: u64) {
    let (r, case) = start_case(seed);
    r.config(&config_for(seed, case.variant));
    let new = apply_and_check_shape(&r, &case);
    check_messages_and_other_branches(&r, &case, &new);
    let backed_up = check_backup(&r, &case);
    check_idempotent(&r, &case);
    if case.variant != 3 {
        check_new_commits_are_rewritten_alone(&r, &case);
    }
    if backed_up {
        check_restore(&r, &case);
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
