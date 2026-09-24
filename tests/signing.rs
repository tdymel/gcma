//! `signing: resign` combined with the other rules: signatures verify, identities stay.

mod common;

use common::*;

fn ssh_keygen_available() -> bool {
    std::process::Command::new("ssh-keygen")
        .arg("-?")
        .output()
        .is_ok()
}

/// Configures SSH signing and returns the allowed-signers file that verifies it.
fn ssh_signing(r: &Repo) -> std::path::PathBuf {
    let key = r.home.path().join("sign_key");
    let o = std::process::Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-f"])
        .arg(&key)
        .output()
        .unwrap();
    assert!(o.status.success());
    let public = std::fs::read_to_string(format!("{}.pub", key.display())).unwrap();
    let allowed = r.home.path().join("allowed_signers");
    std::fs::write(&allowed, format!("me@home.org {public}")).unwrap();
    r.git(&["config", "gpg.format", "ssh"]);
    r.git(&[
        "config",
        "user.signingkey",
        &format!("{}.pub", key.display()),
    ]);
    r.git(&[
        "config",
        "gpg.ssh.allowedSignersFile",
        allowed.to_str().unwrap(),
    ]);
    allowed
}

#[test]
fn resigned_commits_verify_keep_their_identities_and_follow_the_schedule() {
    if !ssh_keygen_available() {
        assert!(
            std::env::var_os("CI").is_none(),
            "ssh-keygen is required for the signing test on CI"
        );
        eprintln!("skipping: ssh-keygen not available");
        return;
    }
    let r = Repo::new();
    ssh_signing(&r);
    r.commit_as("a.txt", "one", 1_600_000_000, "Alice", "alice@x.org");
    r.commit_as("b.txt", "two", 1_600_100_000, "Bob", "bob@x.org");
    r.commit_at("c.txt", "three", 1_600_200_000);
    let old = r.log();
    r.config(&berlin_cfg("signing: resign\n"));
    r.ghma_ok(&["apply", "--from", "root"]);
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
            String::from_utf8_lossy(&v.stderr)
        );
    }
    r.fsck();
    assert!(
        r.ghma(&["plan", "--check", "--from", "root"])
            .status
            .success(),
        "a second run has nothing left to do"
    );
}
