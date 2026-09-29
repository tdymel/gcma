//! The undo path (`restore`, `hook install`, `hook uninstall`) never reads `gcma.yml`, so a
//! broken or outdated config cannot lock anyone out of getting their history back. Commands
//! that do need the rules still reject it.

mod common;

use common::*;

/// Parses as YAML but `last` is not a known field.
const BROKEN_CFG: &str = "version: 1\nlast: 6mo\n";

/// A repository with one applied rewrite (so there is a backup) and then a broken config.
/// Returns the repository, the tip before the rewrite and the backup id.
fn rewritten_then_broken() -> (Repo, String, String) {
    let r = Repo::new();
    r.linear(2, T0);
    let before = r.git(&["rev-parse", "HEAD"]);
    r.config(IDENTITY_CFG);
    r.gcma_ok(&["apply", "--from", "root"]);
    assert_ne!(r.git(&["rev-parse", "HEAD"]), before);
    let id = r.backup_id_from_refs();
    r.config(BROKEN_CFG);
    (r, before, id)
}

#[test]
fn restore_works_with_a_broken_config() {
    let (r, before, id) = rewritten_then_broken();
    assert!(r.gcma_ok(&["restore"]).contains(&id));
    r.gcma_ok(&["restore", &id]);
    assert_eq!(r.git(&["rev-parse", "HEAD"]), before);
}

#[test]
fn prune_works_with_a_broken_config() {
    let (r, _, id) = rewritten_then_broken();
    r.gcma_ok(&["restore", &id, "--prune"]);
    assert!(r.gcma_ok(&["restore"]).contains("No backups."));
}

#[test]
fn hook_install_and_uninstall_work_with_a_broken_config() {
    let (r, _, _) = rewritten_then_broken();
    r.gcma_ok(&["hook", "install"]);
    assert!(r.path().join(".git/hooks/pre-push").exists());
    assert!(r.gcma_ok(&["hook", "uninstall"]).contains("hook removed"));
    assert!(!r.path().join(".git/hooks/pre-push").exists());
}

#[test]
fn commands_that_need_the_rules_still_reject_a_broken_config() {
    let (r, _, _) = rewritten_then_broken();
    for args in [
        &["plan", "--from", "root"][..],
        &["apply", "--from", "root"],
        &["export", "--from", "root"],
    ] {
        assert_eq!(Repo::code(&r.gcma(args)), 2, "{args:?}");
    }
}

#[test]
fn import_rejects_a_broken_config() {
    let r = Repo::new();
    r.linear(2, T0);
    r.config(IDENTITY_CFG);
    let plan = r.path().join("plan.json");
    let plan = plan.to_str().unwrap();
    r.gcma_ok(&["plan", "--from", "root", "--out", plan]);
    r.config(BROKEN_CFG);
    r.write("reply.jsonl", "");
    let o = r.gcma(&["import", "--plan", plan, "reply.jsonl"]);
    assert_eq!(Repo::code(&o), 2, "{}", stderr(&o));
}
