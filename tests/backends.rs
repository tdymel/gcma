//! Choosing the object backend, and the two backends agreeing.

mod common;

use common::*;

#[test]
fn backend_selection_flag_env_and_config() {
    let r = Repo::new();
    r.linear(3, T0);
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
    assert_eq!(Repo::code(&gix), expected, "{}", stderr(&gix));
    // The flag beats the environment, which beats the config.
    r.config(&format!("{IDENTITY_CFG}backend: gix\n"));
    let with_env = |value: &str| {
        r.cmd(bin())
            .args(["plan", "--from", "root"])
            .env("GCMA_BACKEND", value)
            .output()
            .unwrap()
    };
    // The config asks for gix: that works exactly when gix was compiled in.
    assert_eq!(Repo::code(&plain()), expected);
    // The environment beats the config, the flag beats the environment.
    assert_eq!(Repo::code(&with_env("git")), 0);
    r.config(&format!("{IDENTITY_CFG}backend: git\n"));
    assert_eq!(Repo::code(&with_env("gix")), expected);
    let o = r
        .cmd(bin())
        .args(["--backend", "git", "plan", "--from", "root"])
        .env("GCMA_BACKEND", "gix")
        .output()
        .unwrap();
    assert_eq!(Repo::code(&o), 0, "{}", stderr(&o));
}

#[cfg(feature = "gix")]
#[test]
fn gix_and_git_backends_write_byte_identical_commits() {
    let mk = || {
        let r = Repo::new();
        r.linear(8, T0);
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
