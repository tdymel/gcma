//! Randomized check of the path rules: random histories (branches, merges, commits touching only
//! excluded paths, both, or neither) must lose exactly the commits that only touched excluded paths,
//! keep every other tree minus the excluded paths, and stay restorable and idempotent.

mod common;

use common::*;

fn build(r: &Repo, rng: &mut Rand, ops: usize) {
    r.commit_files(&[("root.txt", "root\n")], "root", 1_500_000_000);
    let mut branches = vec!["main".to_string()];
    for n in 1..=ops {
        let t = 1_400_000_000 + rng.below(400_000_000) as i64;
        let plain = (format!("f{n}.txt"), format!("{n}\n"));
        let secret = (format!("secrets/s{n}.txt"), format!("s{n}\n"));
        let files: Vec<(&str, &str)> = match rng.below(4) {
            0 => vec![(secret.0.as_str(), secret.1.as_str())],
            1 => vec![
                (plain.0.as_str(), plain.1.as_str()),
                (secret.0.as_str(), secret.1.as_str()),
            ],
            _ => vec![(plain.0.as_str(), plain.1.as_str())],
        };
        match rng.below(10) {
            0 | 1 => {
                let all = r.git(&["rev-list", "--all"]);
                let all: Vec<&str> = all.lines().collect();
                let from = all[rng.below(all.len() as u64) as usize];
                let name = format!("b{n}");
                r.git(&["checkout", "-q", "-b", &name, from]);
                branches.push(name);
                r.commit_files(&files, &format!("branch {n}"), t);
            }
            2 | 3 => {
                let b = branches[rng.below(branches.len() as u64) as usize].clone();
                r.git(&["checkout", "-q", &b]);
                r.commit_files(&files, &format!("switch {n}"), t);
            }
            4 => {
                let cur = r.git(&["rev-parse", "--abbrev-ref", "HEAD"]);
                let others: Vec<&String> = branches.iter().filter(|b| **b != cur).collect();
                if others.is_empty() {
                    r.commit_files(&files, &format!("commit {n}"), t);
                    continue;
                }
                let pick = others[rng.below(others.len() as u64) as usize];
                let date = format!("{t} +0000");
                let o = r
                    .cmd("git")
                    .args(["merge", "-q", "--no-ff", "-m", "merge", pick])
                    .env("GIT_AUTHOR_DATE", &date)
                    .env("GIT_COMMITTER_DATE", &date)
                    .output()
                    .unwrap();
                if !o.status.success() {
                    let _ = r.git_out(&["merge", "--abort"]);
                }
            }
            _ => {
                r.commit_files(&files, &format!("commit {n}"), t);
            }
        }
    }
    r.git(&["checkout", "-q", "main"]);
}

/// Commits (single parent or root) whose change touches only `secrets/`.
fn only_secret_commits(r: &Repo) -> usize {
    r.git(&["rev-list", "--parents", "HEAD"])
        .lines()
        .filter(|l| l.split(' ').count() <= 2)
        .filter(|l| {
            let oid = l.split(' ').next().unwrap();
            let changed = r.git(&[
                "diff-tree",
                "--root",
                "--no-commit-id",
                "-r",
                "--name-only",
                oid,
            ]);
            !changed.is_empty() && changed.lines().all(|p| p.starts_with("secrets/"))
        })
        .count()
}

/// `path -> "mode blob"` of a commit's tree, without `secrets/` and the ignore file.
fn visible_files(r: &Repo, rev: &str) -> std::collections::BTreeMap<String, String> {
    r.git(&["ls-tree", "-r", rev])
        .lines()
        .map(|l| {
            let (meta, path) = l.split_once('\t').unwrap();
            (path.to_string(), meta.to_string())
        })
        .filter(|(p, _)| !p.starts_with("secrets/") && p != ".gitignore")
        .collect()
}

/// Pairs every old commit with its replacement and compares the trees one by one. A commit is
/// dropped when it is not the root of a merge and only touched `secrets/`; its children are
/// attached to its replacement parents. The old tip may stay as the commit that carries the
/// ignore patterns.
fn check_every_tree(seed: u64, r: &Repo, old: &[Row], old_tip: &str, tip_subject: &str) {
    let new = r.log();
    let secret_only = |oid: &str, parents: &[String]| -> bool {
        if parents.len() > 1 {
            return false;
        }
        let changed = r.git(&[
            "diff-tree",
            "--root",
            "--no-commit-id",
            "-r",
            "--name-only",
            oid,
        ]);
        !changed.is_empty() && changed.lines().all(|p| p.starts_with("secrets/"))
    };
    // old oid -> new oid(s) standing in for it as a parent
    let mut stands_for: std::collections::HashMap<String, Vec<String>> = Default::default();
    let mut taken: std::collections::HashSet<String> = Default::default();
    for o in old {
        let parents: Vec<String> = o
            .parents
            .iter()
            .flat_map(|p| stands_for[p].clone())
            .collect();
        let candidate = new
            .iter()
            .find(|n| !taken.contains(&n.oid) && n.subject == o.subject && n.parents == parents);
        let dropped = secret_only(&o.oid, &o.parents);
        match (candidate, dropped) {
            (Some(n), false) => {
                assert_eq!(
                    visible_files(r, &n.oid),
                    visible_files(r, &o.oid),
                    "seed {seed}: tree of {:?} differs from the old one minus secrets/",
                    o.subject
                );
                // Where the old tree held secrets, the new one ignores them.
                let had_secrets = r
                    .git(&["ls-tree", "-r", "--name-only", &o.oid])
                    .lines()
                    .any(|p| p.starts_with("secrets/"));
                if had_secrets {
                    let ignore = r.git(&["show", &format!("{}:.gitignore", n.oid)]);
                    assert!(
                        ignore.lines().any(|l| l == "secrets/"),
                        "seed {seed}: {:?} has secrets but its .gitignore does not name them",
                        o.subject
                    );
                }
                taken.insert(n.oid.clone());
                stands_for.insert(o.oid.clone(), vec![n.oid.clone()]);
            }
            // The tip stays when nothing else carries the patterns.
            (Some(n), true) if o.oid == old_tip && n.subject == tip_subject => {
                taken.insert(n.oid.clone());
                stands_for.insert(o.oid.clone(), vec![n.oid.clone()]);
            }
            (_, true) => {
                stands_for.insert(o.oid.clone(), parents);
            }
            (None, false) => panic!(
                "seed {seed}: no replacement for {:?} ({})",
                o.subject, o.oid
            ),
        }
    }
    assert_eq!(
        taken.len(),
        new.len(),
        "seed {seed}: unexplained new commits"
    );
}

fn run_case(seed: u64) {
    let mut rng = Rand::new(seed);
    let r = Repo::for_seed(seed);
    build(&r, &mut rng, 14 + (seed as usize % 12));
    let old_rows = r.log();
    let old_tip = r.git(&["rev-parse", "HEAD"]);
    let old_tip_subject = r.git(&["log", "-1", "--format=%s"]);
    let old_count: usize = r.git(&["rev-list", "--count", "HEAD"]).parse().unwrap();
    let doomed = only_secret_commits(&r);
    let old_files: Vec<String> = r
        .git(&["ls-tree", "-r", "--name-only", "HEAD"])
        .lines()
        .map(String::from)
        .collect();

    let schedule = seed.is_multiple_of(2);
    r.config(&if schedule {
        berlin_cfg("paths:\n  exclude: [\"secrets/\"]\n")
    } else {
        SECRETS_CFG.to_string()
    });
    let o = r.gcma(&["apply", "--from", "root"]);
    assert!(
        o.status.success(),
        "seed {seed}: {}{}",
        stdout(&o),
        stderr(&o)
    );
    r.fsck();

    let new_count: usize = r.git(&["rev-list", "--count", "HEAD"]).parse().unwrap();
    // The old tip is the one commit that may survive despite touching only excluded paths: it
    // carries the .gitignore entry when nothing else on the branch does.
    if new_count == old_count - doomed + 1 {
        assert_eq!(
            r.git(&["log", "-1", "--format=%s"]),
            old_tip_subject,
            "seed {seed}"
        );
    } else {
        assert_eq!(
            new_count,
            old_count - doomed,
            "seed {seed}: dropped exactly the secret-only commits"
        );
    }
    for row in r.log() {
        let files = r.git(&["ls-tree", "-r", "--name-only", &row.oid]);
        assert!(
            !files.lines().any(|p| p.starts_with("secrets/")),
            "seed {seed}"
        );
    }
    check_every_tree(seed, &r, &old_rows, &old_tip, &old_tip_subject);
    let mut expected: Vec<String> = old_files
        .iter()
        .filter(|p| !p.starts_with("secrets/"))
        .cloned()
        .collect();
    let new_files: Vec<String> = r
        .git(&["ls-tree", "-r", "--name-only", "HEAD"])
        .lines()
        .map(String::from)
        .collect();
    if new_files.contains(&".gitignore".to_string()) {
        expected.push(".gitignore".into());
        expected.sort();
    }
    assert_eq!(new_files, expected, "seed {seed}: tip content");
    if schedule {
        assert_scheduled(&r.log());
    }

    // Working copy: secret files remain and nothing but the config shows up as changed.
    for p in old_files.iter().filter(|p| p.starts_with("secrets/")) {
        assert!(
            r.path().join(p).exists(),
            "seed {seed}: {p} vanished from the project"
        );
    }
    let status: Vec<String> = r
        .git(&["status", "--porcelain"])
        .lines()
        .filter(|l| !l.contains("gcma.yml"))
        .map(String::from)
        .collect();
    assert!(status.is_empty(), "seed {seed}: {status:?}");

    // Idempotent and undoable; the old history is intact behind the backup.
    assert!(
        r.gcma_ok(&["apply", "--from", "root"])
            .contains("Nothing to do"),
        "seed {seed}"
    );
    assert!(
        r.gcma(&["plan", "--check", "--from", "root"])
            .status
            .success(),
        "seed {seed}"
    );
    if r.backup_count() == 0 {
        // Nothing to rewrite: the history never had a secret and no schedule applied.
        assert!(
            !schedule && old_files.iter().all(|p| !p.starts_with("secrets/")),
            "seed {seed}"
        );
        assert_eq!(r.git(&["rev-parse", "HEAD"]), old_tip, "seed {seed}");
        return;
    }
    let id = r.backup_id_from_refs();
    assert_eq!(
        r.git(&[
            "rev-list",
            "--count",
            &format!("refs/gcma/backup/main/{id}/old")
        ]),
        old_count.to_string()
    );
    r.gcma_ok(&["restore", &id]);
    assert_eq!(r.git(&["rev-parse", "HEAD"]), old_tip, "seed {seed}");
}

#[test]
fn random_histories_lose_only_secret_commits() {
    for seed in 1..=30 {
        run_case(seed);
    }
}
