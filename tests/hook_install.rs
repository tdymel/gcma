//! `gcma hook install` and `gcma hook uninstall`: which shims they write and remove, where, and
//! that a hook gcma did not write is never clobbered unless forced. What the installed hooks do is
//! in `pre_push_hook.rs` and `post_commit_hook.rs`.

mod common;

use std::path::{Path, PathBuf};

use common::*;

/// A fresh repository and the paths of its pre-push and post-commit hooks.
fn repo_and_hooks() -> (Repo, PathBuf, PathBuf) {
    let r = Repo::new();
    let pre = r.path().join(".git/hooks/pre-push");
    let post = r.path().join(".git/hooks/post-commit");
    (r, pre, post)
}

fn is_managed(hook: &Path) -> bool {
    std::fs::read_to_string(hook)
        .unwrap()
        .contains("gcma-managed-hook")
}

const FOREIGN_HOOK: &str = "#!/bin/sh\nexit 0\n";

#[test]
fn install_writes_the_pre_push_shim_and_post_commit_only_on_request() {
    let (r, pre, post) = repo_and_hooks();
    r.gcma_ok(&["hook", "install"]);
    assert!(pre.exists() && !post.exists(), "post-commit is opt-in");
    assert!(is_managed(&pre));
    r.gcma_ok(&["hook", "install", "--post-commit"]);
    assert!(pre.exists() && post.exists());
    assert!(is_managed(&post));
}

#[test]
fn uninstall_removes_every_shim_and_says_when_there_is_none() {
    let (r, pre, post) = repo_and_hooks();
    let out = r.gcma_ok(&["hook", "uninstall"]);
    assert!(out.contains("no hook installed"), "{out}");
    r.gcma_ok(&["hook", "install", "--post-commit"]);
    assert!(r.gcma_ok(&["hook", "uninstall"]).contains("hook removed"));
    assert!(!pre.exists() && !post.exists());
    assert!(
        r.gcma_ok(&["hook", "uninstall"])
            .contains("no hook installed")
    );
}

#[test]
fn a_foreign_pre_push_hook_is_kept_unless_install_is_forced() {
    let (r, pre, _) = repo_and_hooks();
    std::fs::write(&pre, FOREIGN_HOOK).unwrap();
    assert_eq!(Repo::code(&r.gcma(&["hook", "install"])), 3);
    assert_eq!(Repo::code(&r.gcma(&["hook", "uninstall"])), 3);
    assert_eq!(std::fs::read_to_string(&pre).unwrap(), FOREIGN_HOOK);
    r.gcma_ok(&["hook", "install", "--force"]);
    assert!(is_managed(&pre));
    r.gcma_ok(&["hook", "uninstall"]);
    assert!(!pre.exists());
}

#[test]
fn a_foreign_post_commit_hook_is_kept_unless_install_is_forced() {
    let (r, pre, post) = repo_and_hooks();
    std::fs::write(&post, FOREIGN_HOOK).unwrap();
    assert_eq!(
        Repo::code(&r.gcma(&["hook", "install", "--post-commit"])),
        3
    );
    assert!(!pre.exists(), "nothing is written when one shim is refused");
    assert_eq!(Repo::code(&r.gcma(&["hook", "uninstall"])), 3);
    r.gcma_ok(&["hook", "install"]); // the pre-push alone is fine
    r.gcma_ok(&["hook", "uninstall"]); // and leaves the foreign hook alone
    assert!(post.exists() && !pre.exists());
    assert_eq!(std::fs::read_to_string(&post).unwrap(), FOREIGN_HOOK);
    r.gcma_ok(&["hook", "install", "--post-commit", "--force"]);
    assert!(is_managed(&post));
}

#[test]
fn install_honours_a_custom_hooks_path() {
    let r = Repo::new();
    r.bare_remote();
    r.config(IDENTITY_CFG);
    let hooks = r.path().join(".githooks");
    std::fs::create_dir_all(&hooks).unwrap();
    r.git(&["config", "core.hooksPath", ".githooks"]);
    r.gcma_ok(&["hook", "install"]);
    assert!(hooks.join("pre-push").exists());
    assert!(!r.path().join(".git/hooks/pre-push").exists());
    r.commit_at("bad.txt", "bad", T0);
    let o = r.git_out(&["push", "-q", "-u", "origin", "main"]);
    assert!(
        !o.status.success(),
        "the hook must run from core.hooksPath ({}): {}",
        hooks.display(),
        stderr(&o)
    );
}

#[test]
fn install_and_uninstall_ignore_a_broken_config() {
    let (r, pre, _) = repo_and_hooks();
    // Parses as YAML but `last` is not a known field.
    r.config("version: 1\nlast: 6mo\n");
    r.gcma_ok(&["hook", "install"]);
    assert!(pre.exists());
    assert!(r.gcma_ok(&["hook", "uninstall"]).contains("hook removed"));
    assert!(!pre.exists());
}
