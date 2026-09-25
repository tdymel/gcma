//! Choosing the object backend, and the two backends agreeing.

mod common;

use common::*;

const IDENTITY_CFG: &str = "version: 1\nidentity:\n  - match: {email: me@home.org}\n    set: {name: Jane Doe, email: jane@work.com}\n";

#[test]
fn backend_selection_flag_env_and_config() {
    let r = Repo::new();
    r.linear(3, 1_600_000_000);
    r.config(IDENTITY_CFG);
    // Unknown values are usage errors, from the flag, the env var and the config alike.
    assert_eq!(
        Repo::code(&r.gcma(&["--backend", "bogus", "plan", "--from", "root"])),
        2
    );
    let o = r
        .cmd(bin())
        .args(["plan", "--from", "root"])
        .env("GCMA_BACKEND", "bogus")
        .output()
        .unwrap();
    assert_eq!(Repo::code(&o), 2);
    r.config(&format!("{IDENTITY_CFG}backend: bogus\n"));
    let plain = || {
        r.cmd(bin())
            .args(["plan", "--from", "root"])
            .env_remove("GCMA_BACKEND")
            .output()
            .unwrap()
    };
    assert_eq!(Repo::code(&plain()), 2);
    // `git` always works; `gix` works exactly when it was compiled in.
    r.config(IDENTITY_CFG);
    r.gcma_ok(&["--backend", "git", "plan", "--from", "root"]);
    let gix = r.gcma(&["--backend", "gix", "plan", "--from", "root"]);
    let expected = if cfg!(feature = "gix") { 0 } else { 2 };
    assert_eq!(
        Repo::code(&gix),
        expected,
        "{}",
        String::from_utf8_lossy(&gix.stderr)
    );
    // The flag beats the environment, which beats the config.
    r.config(&format!("{IDENTITY_CFG}backend: gix\n"));
    if !cfg!(feature = "gix") {
        assert_eq!(Repo::code(&plain()), 2);
        r.gcma_ok(&["--backend", "git", "plan", "--from", "root"]);
        let o = r
            .cmd(bin())
            .args(["plan", "--from", "root"])
            .env("GCMA_BACKEND", "git")
            .output()
            .unwrap();
        assert_eq!(Repo::code(&o), 0);
    }
}

#[cfg(feature = "gix")]
#[test]
fn gix_and_git_backends_write_byte_identical_commits() {
    let mk = || {
        let r = Repo::new();
        r.linear(8, 1_600_000_000);
        r.config(&format!(
            "{IDENTITY_CFG}from: 2025-01-01\nto: 2026-01-31\nschedule: {{days: [mon, tue, wed], hours: \"10:00-16:00\", seed: 5}}\n"
        ));
        r
    };
    let (a, b) = (mk(), mk());
    assert_eq!(a.git(&["rev-parse", "HEAD"]), b.git(&["rev-parse", "HEAD"]));
    a.gcma_ok(&["--backend", "git", "apply", "--from", "root"]);
    b.gcma_ok(&["--backend", "gix", "apply", "--from", "root"]);
    let (la, lb) = (a.log(), b.log());
    assert_eq!(la.len(), 8);
    for (x, y) in la.iter().zip(&lb) {
        assert_eq!(x.oid, y.oid, "commit ids must match across backends");
        assert_eq!(a.cat(&x.oid), b.cat(&y.oid));
    }
    b.fsck();
    // A rerun with the other backend is a no-op too.
    assert!(
        b.gcma_ok(&["--backend", "git", "apply", "--from", "root"])
            .contains("Nothing to do")
    );
}
