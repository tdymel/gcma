//! `signing: resign` combined with the other rules (signatures verify, identities stay), and what
//! it refuses or warns about: headers it cannot carry and messages git would recode.

mod common;

use common::*;

#[test]
fn resigned_commits_verify_keep_their_identities_and_follow_the_schedule() {
    if !ssh_keygen_or_skip() {
        return;
    }
    let r = Repo::new();
    r.ssh_signing();
    r.commit_as("a.txt", "one", T0, "Alice", "alice@x.org");
    r.commit_as("b.txt", "two", 1_600_100_000, "Bob", "bob@x.org");
    r.commit_at("c.txt", "three", 1_600_200_000);
    let old = r.log();
    r.config(&berlin_cfg("signing: resign\n"));
    r.gcma_ok(&["apply", "--from", "root"]);
    let new = r.log();
    assert_same_content(&old, &new);
    assert_scheduled(&new);
    for (o, n) in old.iter().zip(&new) {
        assert_eq!((&o.an, &o.ae, &o.cn, &o.ce), (&n.an, &n.ae, &n.cn, &n.ce));
        let v = r.git_out(&["verify-commit", &n.oid]);
        assert!(
            v.status.success(),
            "{} does not verify: {}",
            n.oid,
            stderr(&v)
        );
    }
    r.fsck();
    assert!(
        r.gcma(&["plan", "--check", "--from", "root"])
            .status
            .success(),
        "a second run has nothing left to do"
    );
}

#[test]
fn signing_strip_and_resign() {
    if !ssh_keygen_or_skip() {
        return;
    }
    let r = Repo::new();
    r.ssh_signing();
    r.git(&["config", "commit.gpgsign", "true"]);
    r.linear(3, T0);
    r.git(&["config", "commit.gpgsign", "false"]);
    assert!(r.git(&["cat-file", "-p", "HEAD"]).contains("gpgsig"));

    // strip (default): the signed commits are nonconforming and lose the signature.
    r.config("version: 1\n");
    r.gcma_ok(&["apply", "--from", "root"]);
    for row in r.log() {
        assert!(
            !String::from_utf8_lossy(&r.cat(&row.oid)).contains("gpgsig"),
            "signature stripped"
        );
    }
    r.fsck();
    assert!(
        r.gcma_ok(&["apply", "--from", "root"])
            .contains("Nothing to do")
    );

    // resign: unsigned commits are nonconforming and get signed; trees stay.
    let old = r.log();
    r.config("version: 1\nsigning: resign\n");
    r.gcma_ok(&["apply", "--from", "root"]);
    let new = r.log();
    assert_same_content(&old, &new);
    for row in &new {
        assert!(
            String::from_utf8_lossy(&r.cat(&row.oid)).contains("gpgsig"),
            "re-signed"
        );
    }
    r.fsck();
    assert!(
        r.gcma_ok(&["apply", "--from", "root"])
            .contains("Nothing to do"),
        "resign is idempotent"
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
    // The dry run only warns: an imported reply can still give the commit a new message.
    let o = r.gcma(&["plan", "--from", "root"]);
    assert!(o.status.success(), "{}", stderr(&o));
    // Applying would have to keep the bytes, which signing cannot do.
    let o = r.gcma(&["apply", "--from", "root"]);
    assert_eq!(Repo::code(&o), 3, "{}", stderr(&o));
    assert!(stderr(&o).contains("not valid UTF-8"), "{}", stderr(&o));
    assert_eq!(r.git(&["rev-parse", "HEAD"]), tip);
    assert!(r.git(&["for-each-ref", "refs/gcma/"]).is_empty());
    // Replacing the message through export/import makes the commit signable.
    let plan = r.path().join("plan.json");
    let plan_arg = plan.to_str().unwrap();
    r.gcma_ok(&["plan", "--from", "root", "--out", plan_arg]);
    let reply = r.path().join("reply.jsonl");
    std::fs::write(&reply, "{\"i\":0,\"t\":\"Fixed message\"}\n").unwrap();
    r.gcma_ok(&["import", "--plan", plan_arg, reply.to_str().unwrap()]);
    r.gcma_ok(&["apply", "--plan", plan_arg]);
    assert_eq!(r.message_bytes("HEAD"), b"Fixed message\n");
    assert!(r.git_out(&["verify-commit", "HEAD"]).status.success());
    // Stripping signatures keeps the bytes, so that config works without a reply.
    let s = Repo::new();
    s.commit_msg("latin.txt", b"caf\xe9\n", T0);
    s.config(&format!("version: 1\nsigning: strip\n{IDENTITY_RULE}"));
    s.gcma_ok(&["apply", "--from", "root"]);
    assert_eq!(s.message_bytes("HEAD"), b"caf\xe9\n");
}
