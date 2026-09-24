//! Names, emails and messages that are not UTF-8 must neither abort a run nor change.

mod common;

use common::*;

fn latin1_history() -> Repo {
    let r = Repo::new();
    r.commit_at("a.txt", "plain", 1_600_000_000);
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
        r.config(&berlin_cfg(
            "identity:\n  - match: {email: me@home.org}\n    set: {name: Jane Doe, email: jane@work.com}\n",
        ));
        let o = r.ghma(&["--backend", backend, "apply", "--from", "root"]);
        assert!(
            o.status.success(),
            "{backend}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
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
    r.ghma_ok(&["plan", "--from", "root", "--out", plan.to_str().unwrap()]);
    let json = std::fs::read_to_string(&plan).unwrap();
    assert!(
        json.contains("\"base64\""),
        "binary names are base64 in the plan: {json}"
    );
    let o = r.ghma(&["export", "--plan", plan.to_str().unwrap()]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    r.ghma_ok(&["apply", "--plan", plan.to_str().unwrap()]);
    assert_eq!(name_bytes(&r, "HEAD~1"), b"J\xf6rg M\xfcller");
    r.fsck();
}

#[test]
fn an_email_rule_applies_to_an_author_whose_name_is_not_utf8() {
    let r = Repo::new();
    r.commit_as_bytes("a.txt", "one", 1_600_000_000, b"J\xf6rg", b"me@home.org");
    r.config("version: 1\nidentity:\n  - match: {email: me@home.org}\n    set: {name: Jane Doe, email: jane@work.com}\n");
    r.ghma_ok(&["apply", "--from", "root"]);
    let log = r.log();
    assert_eq!(
        (log[0].an.as_str(), log[0].ae.as_str()),
        ("Jane Doe", "jane@work.com")
    );
    assert!(
        r.ghma(&["plan", "--check", "--from", "root"])
            .status
            .success()
    );
    r.fsck();
}
