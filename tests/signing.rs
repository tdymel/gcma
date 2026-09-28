//! `signing: resign` combined with the other rules: signatures verify, identities stay.

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
