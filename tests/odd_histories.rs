//! Histories and repositories that are not the plain linear case: unrelated roots, names that are
//! not UTF-8, offsets that disagree with the schedule, other branches' backups, and the warnings
//! `apply` prints.

mod common;

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;

use chrono::TimeZone;
use common::*;

const IDENTITY_CFG: &str = "version: 1\nidentity:\n  - match: {email: me@home.org}\n    set: {name: Jane Doe, email: jane@work.com}\n";
const T0: i64 = 1_600_000_000;

fn stderr(o: &std::process::Output) -> String {
    String::from_utf8_lossy(&o.stderr).to_string()
}

#[test]
fn a_backup_of_another_branch_is_refused_and_leaves_both_branches_alone() {
    let r = Repo::new();
    r.linear(3, T0);
    r.config(IDENTITY_CFG);
    r.gcma_ok(&["apply", "--from", "root"]);
    let listing = r.gcma_ok(&["restore"]);
    let id = listing.split_whitespace().next().unwrap().to_string();
    r.git(&["checkout", "-q", "-b", "other"]);
    let (main, other) = (
        r.git(&["rev-parse", "main"]),
        r.git(&["rev-parse", "other"]),
    );
    let o = r.gcma(&["restore", &id]);
    assert_eq!(Repo::code(&o), 3, "{}", stderr(&o));
    assert!(
        stderr(&o).contains("belongs to branch main"),
        "{}",
        stderr(&o)
    );
    assert_eq!(r.git(&["rev-parse", "main"]), main);
    assert_eq!(r.git(&["rev-parse", "other"]), other);
    // Once the right branch is checked out the same backup restores fine.
    r.git(&["checkout", "-q", "main"]);
    r.gcma_ok(&["restore", &id]);
}

#[test]
fn apply_warns_about_tags_that_will_keep_pointing_at_the_old_commits() {
    let r = Repo::new();
    r.linear(3, T0);
    let old_middle = r.git(&["rev-parse", "HEAD~1"]);
    r.git(&["tag", "v1", "HEAD~1"]);
    r.config(IDENTITY_CFG);
    let o = r.gcma(&["apply", "--from", "root"]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(
        stderr(&o).contains("warning: tags/notes point at"),
        "{}",
        stderr(&o)
    );
    assert!(stderr(&o).contains("refs/tags/v1"), "{}", stderr(&o));
    assert_eq!(
        r.git(&["rev-parse", "v1"]),
        old_middle,
        "the tag stays where it was"
    );
}

#[test]
fn apply_with_resign_warns_about_headers_it_cannot_carry() {
    if !ssh_keygen_or_skip() {
        return;
    }
    let r = Repo::new();
    r.ssh_signing();
    r.git(&["config", "i18n.commitEncoding", "ISO-8859-1"]);
    r.commit_at("encoded.txt", "with an encoding header", T0);
    r.git(&["config", "--unset", "i18n.commitEncoding"]);
    assert!(
        r.git(&["cat-file", "-p", "HEAD"])
            .contains("encoding ISO-8859-1")
    );
    r.config("version: 1\nsigning: resign\n");
    let o = r.gcma(&["apply", "--from", "root"]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(stderr(&o).contains("carry extra headers"), "{}", stderr(&o));
}

#[test]
fn resign_refuses_messages_that_git_would_recode_before_anything_is_written() {
    if !ssh_keygen_or_skip() {
        return;
    }
    let r = Repo::new();
    r.ssh_signing();
    r.commit_msg("latin.txt", b"caf\xe9\n", T0); // not valid UTF-8
    r.config("version: 1\nsigning: resign\n");
    let tip = r.git(&["rev-parse", "HEAD"]);
    for cmd in ["plan", "apply"] {
        let o = r.gcma(&[cmd, "--from", "root"]);
        assert_eq!(Repo::code(&o), 3, "{cmd}: {}", stderr(&o));
        assert!(
            stderr(&o).contains("not valid UTF-8"),
            "{cmd}: {}",
            stderr(&o)
        );
    }
    assert_eq!(r.git(&["rev-parse", "HEAD"]), tip);
    assert!(r.git(&["for-each-ref", "refs/gcma/"]).is_empty());
    // Stripping signatures keeps the bytes, so that config still works.
    r.config("version: 1\nsigning: strip\nidentity:\n  - match: {email: me@home.org}\n    set: {name: Jane Doe, email: jane@work.com}\n");
    r.gcma_ok(&["apply", "--from", "root"]);
    assert_eq!(r.message_bytes("HEAD"), b"caf\xe9\n");
}

#[test]
fn unrelated_roots_are_all_rewritten_and_stay_roots() {
    let r = Repo::new();
    r.commit_at("a.txt", "first root", T0);
    r.commit_at("b.txt", "after the first root", T0 + 1000);
    r.git(&["checkout", "-q", "--orphan", "other"]);
    r.git(&["rm", "-rfq", "."]);
    r.commit_at("o.txt", "second root", T0 + 2000);
    r.git(&["checkout", "-q", "main"]);
    r.git(&[
        "merge",
        "-q",
        "--allow-unrelated-histories",
        "-m",
        "join",
        "other",
    ]);
    r.commit_at("z.txt", "after the join", T0 + 3000);
    let roots = |rev: &str| r.git(&["rev-list", "--max-parents=0", rev]).lines().count();
    assert_eq!(roots("HEAD"), 2);
    r.config(&berlin_cfg(
        "identity:\n  - match: {email: me@home.org}\n    set: {name: Jane Doe, email: jane@work.com}\n",
    ));
    let old = r.log();
    r.gcma_ok(&["apply", "--from", "root"]);
    let new = r.log();
    assert_same_content(&old, &new);
    assert_scheduled(&new);
    assert_eq!(roots("HEAD"), 2, "both roots survive");
    assert!(new.iter().all(|x| x.an == "Jane Doe"));
    r.fsck();
    assert!(
        r.gcma_ok(&["apply", "--from", "root"])
            .contains("Nothing to do")
    );
    let id = r
        .gcma_ok(&["restore"])
        .split_whitespace()
        .next()
        .unwrap()
        .to_string();
    r.gcma_ok(&["restore", &id]);
    assert_same_content(&old, &r.log());
}

#[test]
fn path_rules_handle_file_names_that_are_not_utf8() {
    let r = Repo::new();
    r.commit_files(&[("a.txt", "a\n")], "add a", T0);
    // `secrets/<0xff>.pem` and `caf<0xe9>.key` cannot be written as Rust strings.
    let odd = |rel: &[u8]| r.path().join(OsStr::from_bytes(rel));
    std::fs::create_dir_all(r.path().join("secrets")).unwrap();
    // Some file systems (APFS on macOS) refuse names that are not valid UTF-8 (EILSEQ).
    for (name, content) in [
        (&b"secrets/\xff.pem"[..], "pem\n"),
        (&b"caf\xe9.key"[..], "key\n"),
    ] {
        match std::fs::write(odd(name), content) {
            Ok(()) => {}
            Err(e) if matches!(e.raw_os_error(), Some(84 | 92)) => {
                eprintln!("skipping: this file system rejects non-UTF-8 names ({e})");
                return;
            }
            Err(e) => panic!("cannot create the test file: {e}"),
        }
    }
    r.git(&["add", "-A", "--", "secrets"]);
    r.git(&["add", "--", "."]);
    r.git(&["commit", "-q", "-m", "only secrets"]);
    r.commit_files(&[("b.txt", "b\n")], "add b", T0 + 2000);
    r.config("version: 1\npaths:\n  exclude: [\"secrets/\", \"*.key\"]\n  gitignore: false\n");
    let ls = |rev: &str| {
        let o = r.git_out(&["ls-tree", "-r", "-z", "--name-only", rev]);
        o.stdout
            .split(|b| *b == 0)
            .filter(|n| !n.is_empty())
            .map(<[u8]>::to_vec)
            .collect::<Vec<_>>()
    };
    assert!(
        ls("HEAD").iter().any(|n| n.ends_with(b".key")),
        "setup: the odd file is tracked"
    );
    r.gcma_ok(&["apply", "--from", "root"]);
    r.fsck();
    assert_eq!(ls("HEAD"), [b"a.txt".to_vec(), b"b.txt".to_vec()]);
    assert_eq!(
        r.log().len(),
        2,
        "the commit that only held excluded files is gone"
    );
    assert!(
        odd(b"caf\xe9.key").exists(),
        "the working copy keeps its files"
    );
    assert!(
        r.gcma_ok(&["apply", "--from", "root"])
            .contains("Nothing to do")
    );
}

#[test]
fn a_commit_inside_the_hours_but_with_the_wrong_offset_is_fixed_and_its_neighbour_is_not() {
    let r = Repo::new();
    let berlin = |h| {
        let tz: chrono_tz::Tz = "Europe/Berlin".parse().unwrap();
        tz.with_ymd_and_hms(2026, 1, 12, h, 0, 0)
            .unwrap()
            .timestamp()
    };
    let good = r.commit_at_offset("good.txt", "right offset", berlin(10), "+0100");
    r.commit_at_offset("bad.txt", "wrong offset", berlin(11), "+0000");
    r.config(&berlin_cfg(""));
    let o = r.gcma(&["plan", "--check", "--from", "root"]);
    assert_eq!(Repo::code(&o), 6, "{}", stderr(&o));
    assert!(stderr(&o).contains("1 commit(s)"), "{}", stderr(&o));
    r.gcma_ok(&["apply", "--from", "root"]);
    assert_eq!(r.log()[0].oid, good, "the conforming commit keeps its id");
    assert!(
        r.committer_offsets().iter().all(|(_, off)| *off == 60),
        "{:?}",
        r.committer_offsets()
    );
    assert_scheduled(&r.log());
    assert!(
        r.gcma(&["plan", "--check", "--from", "root"])
            .status
            .success()
    );
}
