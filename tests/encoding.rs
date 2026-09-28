//! Names, emails and messages that are not UTF-8 must neither abort a run nor change, and the raw
//! headers of a commit (an `encoding` header) survive a rewrite.

mod common;

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
    let backends: &[&str] = if cfg!(feature = "gix") {
        &["git", "gix"]
    } else {
        &["git"]
    };
    for backend in backends.iter().copied() {
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
