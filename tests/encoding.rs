//! Names, emails, messages and file names that are not UTF-8 must neither abort a run nor change,
//! and the raw headers of a commit (an `encoding` header) survive a rewrite.

mod common;

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;

use common::*;

fn latin1_history() -> Repo {
    let r = Repo::new();
    r.commit_at("a.txt", "plain", T0);
    r.commit_as_bytes(
        "b.txt",
        "latin",
        1_600_100_000,
        b"J\xf6rg M\xfcller",
        b"j\xf6rg@example.org",
    );
    r.commit_at("c.txt", "plain again", 1_600_200_000);
    r
}

/// The author name exactly as stored (`git log` would re-encode it).
fn name_bytes(r: &Repo, rev: &str) -> Vec<u8> {
    let oid = r.git(&["rev-parse", rev]);
    let raw = r.cat(&oid);
    let line = raw
        .split(|&b| b == b'\n')
        .find(|l| l.starts_with(b"author "))
        .unwrap();
    let end = line.iter().position(|&b| b == b'<').unwrap();
    line[7..end - 1].to_vec()
}

#[test]
fn schedule_run_keeps_non_utf8_names_when_no_rule_matches() {
    for backend in BACKENDS {
        let r = latin1_history();
        r.config(&berlin_cfg(IDENTITY_RULE));
        let o = r.gcma(&["--backend", backend, "apply", "--from", "root"]);
        assert!(o.status.success(), "{backend}: {}", stderr(&o));
        r.fsck();
        let rows = r.log();
        assert_eq!(rows[0].an, "Jane Doe", "{backend}");
        assert_eq!(rows[2].an, "Jane Doe", "{backend}");
        assert_eq!(
            name_bytes(&r, "HEAD~1"),
            b"J\xf6rg M\xfcller",
            "{backend}: bytes kept"
        );
        assert_scheduled(&rows);
    }
}

#[test]
fn plan_files_carry_non_utf8_names_through_export_and_apply() {
    let r = latin1_history();
    r.config(&berlin_cfg(""));
    let plan = r.path().join("plan.json");
    r.gcma_ok(&["plan", "--from", "root", "--out", plan.to_str().unwrap()]);
    let json = std::fs::read_to_string(&plan).unwrap();
    assert!(
        json.contains("\"base64\""),
        "binary names are base64 in the plan: {json}"
    );
    let o = r.gcma(&["export", "--plan", plan.to_str().unwrap()]);
    assert!(o.status.success(), "{}", stderr(&o));
    r.gcma_ok(&["apply", "--plan", plan.to_str().unwrap()]);
    assert_eq!(name_bytes(&r, "HEAD~1"), b"J\xf6rg M\xfcller");
    r.fsck();
}

#[test]
fn an_email_rule_applies_to_an_author_whose_name_is_not_utf8() {
    let r = Repo::new();
    r.commit_as_bytes("a.txt", "one", T0, b"J\xf6rg", b"me@home.org");
    r.config(IDENTITY_CFG);
    r.gcma_ok(&["apply", "--from", "root"]);
    let log = r.log();
    assert_eq!(
        (log[0].an.as_str(), log[0].ae.as_str()),
        ("Jane Doe", "jane@work.com")
    );
    assert!(
        r.gcma(&["plan", "--check", "--from", "root"])
            .status
            .success()
    );
    r.fsck();
}

#[test]
fn raw_headers_and_non_utf8_messages_survive() {
    let r = Repo::new();
    r.git(&["config", "i18n.commitEncoding", "ISO-8859-1"]);
    r.write("x.txt", "x\n");
    r.git(&["add", "x.txt"]);
    // A latin-1 message: "caf\xe9".
    let msg_file = r.path().join("msg.bin");
    std::fs::write(&msg_file, b"caf\xe9 au lait\n\nbody \xe9\n").unwrap();
    r.git(&["commit", "-q", "-F", msg_file.to_str().unwrap()]);
    r.config(IDENTITY_CFG);
    let old = r.git(&["rev-parse", "HEAD"]);
    let old_raw = r.cat(&old);
    assert!(
        old_raw.windows(18).any(|w| w == b"encoding ISO-8859-"),
        "fixture has an encoding header"
    );

    r.gcma_ok(&["apply", "--from", "root"]);
    let new = r.git(&["rev-parse", "HEAD"]);
    let new_raw = r.cat(&new);
    assert!(
        new_raw.windows(18).any(|w| w == b"encoding ISO-8859-"),
        "encoding header kept"
    );
    let body_of = |raw: &[u8]| {
        raw.windows(2)
            .position(|w| w == b"\n\n")
            .map(|p| raw[p + 2..].to_vec())
            .unwrap()
    };
    assert_eq!(
        body_of(&old_raw),
        body_of(&new_raw),
        "message bytes are identical"
    );
    r.fsck();
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
