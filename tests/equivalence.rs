//! The git CLI and gix backends must write byte-identical commits for every feature: the same
//! input and config give the same object ids, whichever backend ran.
#![cfg(feature = "gix")]

mod common;

use common::*;

const SIGNED_OFF: &str = "Signed-off-by: Dev <dev@x.org>";

/// Builds the same repository twice, runs `args` once per backend, and compares everything that
/// is visible: tips, every commit's raw bytes, and the backup refs.
fn same_result(build: impl Fn(&Repo), cfg: &str, args: &[&str]) -> (Repo, Repo) {
    let (a, b) = (Repo::new(), Repo::new());
    for r in [&a, &b] {
        build(r);
        r.config(cfg);
    }
    assert_eq!(
        a.git(&["rev-parse", "HEAD"]),
        b.git(&["rev-parse", "HEAD"]),
        "fixtures differ"
    );
    let run = |r: &Repo, backend: &str| {
        let mut full = vec!["--backend", backend];
        full.extend_from_slice(args);
        let o = r.gcma(&full);
        assert!(
            o.status.success(),
            "{backend}: {}{}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        );
    };
    run(&a, "git");
    run(&b, "gix");
    let all = |r: &Repo| r.git(&["rev-list", "--all", "--topo-order"]);
    assert_eq!(all(&a), all(&b), "the same commits");
    for oid in all(&a).lines() {
        assert_eq!(a.cat(oid), b.cat(oid), "raw bytes of {oid}");
    }
    let heads = |r: &Repo| {
        r.git(&[
            "for-each-ref",
            "--format=%(refname) %(objectname)",
            "refs/heads/",
            "refs/tags/",
        ])
    };
    assert_eq!(heads(&a), heads(&b));
    let backups = |r: &Repo| {
        r.git(&[
            "for-each-ref",
            "--format=%(objectname)",
            "refs/gcma/backup/",
        ])
    };
    assert_eq!(backups(&a), backups(&b));
    a.fsck();
    b.fsck();
    (a, b)
}

fn merges(r: &Repo) {
    r.commit_at("base.txt", "base", 1_500_000_000);
    for (i, name) in ["x", "y", "z"].iter().enumerate() {
        r.git(&["checkout", "-q", "-b", name, "main"]);
        r.commit_at(
            &format!("{name}.txt"),
            &format!("work {name}"),
            1_500_100_000 + i as i64 * 1000,
        );
    }
    r.git(&["checkout", "-q", "main"]);
    r.commit_at("m.txt", "main work", 1_500_400_000);
    let date = "1500500000 +0000";
    let o = r
        .cmd("git")
        .args([
            "merge",
            "-q",
            "--no-ff",
            "-m",
            "octopus\n\nSigned-off-by: Dev <dev@x.org>",
            "x",
            "y",
            "z",
        ])
        .env("GIT_AUTHOR_DATE", date)
        .env("GIT_COMMITTER_DATE", date)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    r.commit_at("after.txt", "after", 1_500_600_000);
}

const SCHEDULE: &str = "from: 2025-01-01\nto: 2025-06-30\ntimezone: Europe/Berlin\nschedule:\n  days: [mon, tue, wed, thu, fri]\n  hours: [\"09:30-12:00\", \"18:00-02:00\"]\n  seed: 3\n";
const IDENTITY: &str =
    "identity:\n  - match: {email: me@home.org}\n    set: {name: Jane Doe, email: jane@work.com}\n";

#[test]
fn identity_and_schedule_on_a_linear_history() {
    for dist in ["uniform", "weekday-weighted", "bursty"] {
        same_result(
            |r| {
                r.linear(10, 1_500_000_000);
            },
            &format!("version: 1\n{IDENTITY}{SCHEDULE}")
                .replace("seed: 3", &format!("seed: 3\n  distribution: {dist}")),
            &["apply", "--from", "root"],
        );
    }
}

#[test]
fn octopus_merges_with_schedule_and_trailers() {
    let (a, _) = same_result(
        merges,
        &format!(
            "version: 1\n{IDENTITY}{SCHEDULE}messages:\n  strip_trailers: [Signed-off-by]\n  add_trailers: [\"Assisted-By: Claude <noreply@anthropic.com>\"]\n"
        ),
        &["apply", "--from", "root"],
    );
    let merge = a
        .log()
        .into_iter()
        .find(|x| x.parents.len() >= 3)
        .expect("the octopus survived");
    assert!(!a.messages(&merge.oid)[&merge.oid].contains(SIGNED_OFF));
}

#[test]
fn trailer_rules_alone() {
    same_result(
        |r| {
            for i in 0..6 {
                r.commit_at(
                    &format!("f{i}.txt"),
                    &format!("commit {i}\n\nBody.\n\nSigned-off-by: Dev <dev@x.org>\nCo-authored-by: Pair <p@x.org>"),
                    1_600_000_000 + i * 100_000,
                );
            }
        },
        "version: 1\nmessages:\n  strip_trailers: [Co-authored-by]\n  add_trailers: [\"Assisted-By: A <a@b.c>\"]\n",
        &["apply", "--from", "root"],
    );
}

fn with_secrets(r: &Repo) {
    r.commit_files(&[("src/lib.rs", "fn main() {}\n")], "init", 1_600_000_000);
    r.commit_files(&[("secrets/key.pem", "k\n")], "only a key", 1_600_100_000);
    r.commit_files(
        &[("src/a.rs", "a\n"), ("secrets/b.pem", "b\n")],
        "both",
        1_600_200_000,
    );
    r.git(&["checkout", "-q", "-b", "side", "HEAD~1"]);
    r.commit_files(
        &[("side.txt", "s\n"), ("secrets/side.pem", "s\n")],
        "side both",
        1_600_300_000,
    );
    r.git(&["checkout", "-q", "main"]);
    let date = "1600400000 +0000";
    let o = r
        .cmd("git")
        .args(["merge", "-q", "--no-ff", "-m", "merge side", "side"])
        .env("GIT_AUTHOR_DATE", date)
        .env("GIT_COMMITTER_DATE", date)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    r.commit_files(
        &[("secrets/tail.pem", "t\n")],
        "tail only a key",
        1_600_500_000,
    );
}

#[test]
fn path_rules_drop_commits_and_write_the_gitignore() {
    for extra in [
        "",
        "  gitignore: false\n",
        "  only_excluded_commits: keep\n",
    ] {
        same_result(
            with_secrets,
            &format!("version: 1\npaths:\n  exclude: [\"secrets/\", \"*.pem\"]\n{extra}"),
            &["apply", "--from", "root"],
        );
    }
}

#[test]
fn path_rules_with_schedule_identity_and_trailers_together() {
    same_result(
        with_secrets,
        &format!(
            "version: 1\n{IDENTITY}{SCHEDULE}messages:\n  add_trailers: [\"Assisted-By: A <a@b.c>\"]\npaths:\n  exclude: [\"secrets/\"]\n"
        ),
        &["apply", "--from", "root"],
    );
}

#[test]
fn the_all_flag_rewrites_conforming_commits_identically() {
    same_result(
        |r| {
            r.linear(5, 1_600_000_000);
        },
        "version: 1\n",
        &["apply", "--from", "root", "--all"],
    );
}

#[test]
fn non_utf8_names_and_messages_are_carried_over_byte_for_byte() {
    let build = |r: &Repo| {
        r.commit_at("a.txt", "plain", 1_600_000_000);
        r.commit_as_bytes(
            "b.txt",
            "latin1 name",
            1_600_100_000,
            b"J\xf6rg",
            b"j\xf6@x.org",
        );
        let msg = r.path().join("msg.bin");
        std::fs::write(&msg, b"caf\xe9\n\nbody \xe9\n").unwrap();
        r.write("c.txt", "c\n");
        r.git(&["add", "c.txt"]);
        let date = "1600200000 +0000";
        let o = r
            .cmd("git")
            .args([
                "-c",
                "i18n.commitEncoding=ISO-8859-1",
                "commit",
                "-q",
                "-F",
                msg.to_str().unwrap(),
            ])
            .env("GIT_AUTHOR_DATE", date)
            .env("GIT_COMMITTER_DATE", date)
            .output()
            .unwrap();
        assert!(o.status.success());
    };
    same_result(
        build,
        &format!("version: 1\n{IDENTITY}{SCHEDULE}"),
        &["apply", "--from", "root"],
    );
}

#[test]
fn signature_and_extra_headers_are_handled_alike() {
    // A commit carrying a (fake) signature, a mergetag-like header and an encoding header.
    let build = |r: &Repo| {
        use std::io::Write;
        r.commit_at("a.txt", "first", 1_600_000_000);
        let tree = r.git(&["rev-parse", "HEAD^{tree}"]);
        let parent = r.git(&["rev-parse", "HEAD"]);
        let raw = format!(
            "tree {tree}\nparent {parent}\nauthor Old Me <me@home.org> 1600100000 +0000\ncommitter Old Me <me@home.org> 1600100000 +0000\nencoding ISO-8859-1\ngpgsig -----BEGIN PGP SIGNATURE-----\n \n abcdef\n -----END PGP SIGNATURE-----\n\nsigned\n"
        );
        let mut child = r
            .cmd("git")
            .args([
                "hash-object",
                "-t",
                "commit",
                "-w",
                "--literally",
                "--stdin",
            ])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(raw.as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        let oid = String::from_utf8(out.stdout).unwrap().trim().to_string();
        r.git(&["update-ref", "HEAD", &oid]);
    };
    same_result(
        build,
        &format!("version: 1\n{IDENTITY}signing: strip\n"),
        &["apply", "--from", "root"],
    );
}

#[test]
fn plans_and_dry_runs_are_identical_too() {
    let (a, b) = (Repo::new(), Repo::new());
    for r in [&a, &b] {
        with_secrets(r);
        r.config(&format!(
            "version: 1\n{IDENTITY}{SCHEDULE}paths:\n  exclude: [\"secrets/\"]\n"
        ));
    }
    let plan = |r: &Repo, backend: &str| {
        let out = r.path().join("plan.json");
        r.gcma_ok(&[
            "--backend",
            backend,
            "plan",
            "--from",
            "root",
            "--out",
            out.to_str().unwrap(),
        ]);
        std::fs::read_to_string(out).unwrap()
    };
    assert_eq!(plan(&a, "git"), plan(&b, "gix"));
}
