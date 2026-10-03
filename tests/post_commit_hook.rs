//! The post-commit hook: after every commit the times of all unpushed commits are respread over
//! the schedule (`hook.mode: rewrite`); pushed commits are never touched, and the commit itself
//! never fails. Installing it is in `hook_install.rs`.

mod common;

use common::*;

const REWRITE: &str = "hook:\n  mode: rewrite\n";

/// A repository with a bare `origin`, the schedule in rewrite mode and both hooks installed.
fn hooked(extra: &str) -> Repo {
    Repo::hooked_post_commit(&berlin_cfg(&format!("{REWRITE}{extra}")))
}

/// A real `git commit` (the hook runs inside it) of a new file at the current time.
fn commit(r: &Repo, name: &str) -> std::process::Output {
    r.write(name, &format!("content of {name}\n"));
    r.git(&["add", name]);
    r.git_out(&["commit", "-q", "-m", name])
}

fn commit_ok(r: &Repo, name: &str) {
    let o = commit(r, name);
    assert!(o.status.success(), "{}", stderr(&o));
    assert_eq!(stderr(&o), "", "the hook is quiet");
}

fn oids(r: &Repo) -> Vec<String> {
    r.log().into_iter().map(|x| x.oid).collect()
}

#[test]
fn every_commit_respreads_all_unpushed_commits_within_the_hours() {
    // The hook runs inside `git commit`, which does not pass a `GCMA_BACKEND` on: the config
    // chooses the backend.
    for backend in BACKENDS {
        let r = hooked(&format!("backend: {backend}\n"));
        commit_ok(&r, "a.txt");
        let rows = r.log();
        assert_eq!(rows.len(), 1);
        assert_scheduled(&rows);
        assert_eq!(r.backup_count(), 1);

        commit_ok(&r, "b.txt");
        commit_ok(&r, "c.txt");
        let rows = r.log();
        assert_eq!(rows.len(), 3);
        assert_scheduled(&rows);
        assert_eq!(r.backup_count(), 3, "one backup per rewrite, no recursion");
        assert_eq!(r.git(&["status", "--porcelain"]), "?? gcma.yml");
        r.fsck();
    }
}

#[test]
fn a_new_commit_respreads_the_older_unpushed_ones_too() {
    let r = Repo::new();
    r.bare_remote();
    r.config(&berlin_cfg(REWRITE));
    let old = r.linear(3, T0); // long before the schedule, hook not installed yet
    r.gcma_ok(&["hook", "install", "--post-commit"]);
    commit_ok(&r, "new.txt");
    let rows = r.log();
    assert_eq!(rows.len(), 4);
    assert_scheduled(&rows);
    assert!(rows.iter().all(|x| !old.contains(&x.oid)), "all re-timed");
}

#[test]
fn a_conforming_unpushed_commit_is_retimed_with_the_rest() {
    let r = hooked("");
    commit_ok(&r, "a.txt");
    let first = r.log()[0].clone();
    for i in 0..5 {
        commit_ok(&r, &format!("n{i}.txt"));
    }
    let rows = r.log();
    assert_scheduled(&rows);
    assert_ne!(rows[0].oid, first.oid, "the first commit was spread again");
    assert_eq!(rows[0].tree, first.tree);
}

#[test]
fn pushed_commits_are_never_touched_and_only_new_ones_are_retimed() {
    let r = hooked("");
    commit_ok(&r, "a.txt");
    commit_ok(&r, "b.txt");
    r.git(&["push", "-q", "-u", "origin", "main"]);
    let pushed = oids(&r);
    let backups_before = r.backup_count();

    commit_ok(&r, "c.txt");
    commit_ok(&r, "d.txt");
    let rows = r.log();
    assert_eq!(rows.len(), 4);
    assert_eq!(oids(&r)[..2], pushed[..], "pushed commits keep their oids");
    assert_scheduled(&rows);
    assert!(
        rows[2].ct >= rows[1].ct,
        "new commits come after the pushed ones"
    );
    assert_eq!(r.git(&["rev-parse", "origin/main"]), pushed[1]);
    assert_eq!(r.backup_count(), backups_before + 2);
    r.git(&["push", "-q"]); // the pre-push hook finds nothing to fix
}

#[test]
fn without_an_upstream_the_remote_tracking_refs_tell_what_is_pushed() {
    let r = hooked("");
    commit_ok(&r, "a.txt");
    r.git(&["push", "-q", "origin", "main"]); // no -u: no upstream configured
    let pushed = oids(&r);
    commit_ok(&r, "b.txt");
    assert_eq!(oids(&r)[0], pushed[0]);
    assert_eq!(r.log().len(), 2);
    assert_scheduled(&r.log());
}

#[test]
fn running_the_hook_again_changes_nothing() {
    let r = hooked("");
    commit_ok(&r, "a.txt");
    commit_ok(&r, "b.txt");
    let (before, n) = (r.refs(), r.backup_count());
    let o = r.gcma(&["hook", "run", "post-commit"]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert_eq!((stdout(&o), stderr(&o)), (String::new(), String::new()));
    assert_eq!(r.refs(), before);
    assert_eq!(r.backup_count(), n);
}

#[test]
fn the_commit_never_fails_even_when_the_hook_cannot_work() {
    let r = hooked("");
    r.config("version: 1\nschedule: {days: [funday]}\nfrom: 2026-01-01\n");
    let o = commit(&r, "a.txt");
    assert!(o.status.success(), "{}", stderr(&o));
    let err = stderr(&o);
    assert_eq!(err.lines().count(), 1, "{err}");
    assert!(err.starts_with("gcma: post-commit skipped: "), "{err}");
    assert!(err.contains("funday"), "{err}");
    assert_eq!(r.log().len(), 1);

    // Directly, too: exit 0, one line.
    let o = r.gcma(&["hook", "run", "post-commit"]);
    assert_eq!(Repo::code(&o), 0);
    assert_eq!(stderr(&o).lines().count(), 1);
}

#[test]
fn a_missing_binary_does_not_fail_the_commit() {
    let r = hooked("");
    let hook = r.path().join(".git/hooks/post-commit");
    let script = std::fs::read_to_string(&hook).unwrap();
    let broken: Vec<String> = script
        .lines()
        .map(|l| match l.strip_prefix("GCMA='") {
            Some(_) => "GCMA='/nonexistent/gcma'".to_string(),
            None => l.to_string(),
        })
        .collect();
    std::fs::write(&hook, broken.join("\n") + "\n").unwrap();
    let o = commit(&r, "a.txt");
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(stderr(&o).contains("is gone"), "{}", stderr(&o));
}

#[test]
fn verify_mode_does_nothing() {
    let r = Repo::hooked_post_commit(&berlin_cfg("")); // hook.mode defaults to verify
    let first = r.commit_at("a.txt", "a", T0);
    assert_eq!(r.git(&["rev-parse", "HEAD"]), first);
    assert_eq!(r.log()[0].ct, T0);
    assert_eq!(r.backup_count(), 0);
}

#[test]
fn without_a_schedule_there_is_nothing_to_distribute() {
    let r = Repo::hooked_post_commit(&format!("{IDENTITY_CFG}{REWRITE}"));
    commit_ok(&r, "a.txt");
    assert_eq!(r.backup_count(), 0);
    assert_eq!(
        r.log()[0].an,
        "Old Me",
        "only pre-push applies the other rules"
    );
}

#[test]
fn without_an_upstream_or_a_remote_the_hook_skips() {
    let r = Repo::new();
    r.config(&berlin_cfg(REWRITE));
    r.gcma_ok(&["hook", "install", "--post-commit"]);
    commit_ok(&r, "a.txt");
    commit_ok(&r, "b.txt");
    assert_eq!(r.backup_count(), 0);
    assert!(
        r.log().iter().all(|x| x.ct > 1_700_000_000),
        "real commit times"
    );
    // The same history is spread by an explicit apply.
    r.gcma_ok(&["apply", "--from", "root"]);
    assert_scheduled(&r.log());
}

#[test]
fn a_window_without_capacity_is_skipped_silently() {
    let r = hooked("");
    r.config("version: 1\nfrom: 2026-01-06\nto: 2026-01-09\nschedule:\n  days: [mon]\n  hours: \"09:00-10:00\"\nhook: {mode: rewrite}\n");
    commit_ok(&r, "a.txt");
    assert_eq!(r.backup_count(), 0);
}

#[test]
fn a_detached_head_is_left_alone() {
    let r = hooked("");
    commit_ok(&r, "a.txt");
    let main = r.git(&["rev-parse", "main"]);
    r.git(&["checkout", "-q", "--detach"]);
    let before = r.backup_count();
    commit_ok(&r, "b.txt");
    assert_eq!(r.backup_count(), before);
    assert_eq!(r.git(&["rev-parse", "main"]), main);
}

#[test]
fn nothing_is_rewritten_while_a_rebase_or_merge_runs() {
    let r = hooked("");
    commit_ok(&r, "a.txt");
    commit_ok(&r, "b.txt");
    r.git(&["checkout", "-q", "-b", "side", "HEAD~1"]);
    commit_ok(&r, "s.txt");
    let before = r.backup_count();

    // `git rebase -i` stops at the commit; amending it runs post-commit in the middle of the rebase.
    let o = r
        .cmd("git")
        .args(["rebase", "-i", "-q", "--root"])
        // `-i.bak` (no space) is the in-place form both GNU and BSD sed accept.
        .env("GIT_SEQUENCE_EDITOR", "sed -i.bak 's/^pick/edit/'")
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", stderr(&o));
    let o = r.git_out(&["commit", "--amend", "-q", "--no-edit", "--allow-empty"]);
    assert!(o.status.success());
    assert_eq!(stderr(&o), "");
    assert_eq!(
        r.backup_count(),
        before,
        "no rewrite under a running rebase"
    );
    let o = r.gcma(&["hook", "run", "post-commit"]);
    assert_eq!((Repo::code(&o), stderr(&o)), (0, String::new()));
    r.git(&["rebase", "--abort"]);

    // A merge that has not been committed yet.
    r.git(&["checkout", "-q", "main"]);
    r.git(&["merge", "-q", "--no-commit", "--no-ff", "side"]);
    let refs = r.refs();
    let o = r.gcma(&["hook", "run", "post-commit"]);
    assert_eq!((Repo::code(&o), stderr(&o)), (0, String::new()));
    assert_eq!(r.refs(), refs);
}

#[test]
fn staged_changes_are_not_disturbed() {
    let r = hooked("");
    commit_ok(&r, "a.txt");
    r.write("staged.txt", "s\n");
    r.git(&["add", "staged.txt"]);
    let refs = r.refs();
    let o = r.gcma(&["hook", "run", "post-commit"]);
    assert_eq!((Repo::code(&o), stderr(&o)), (0, String::new()));
    assert_eq!(r.refs(), refs);
    assert!(r.git(&["status", "--porcelain"]).contains("A  staged.txt"));
}

#[test]
fn the_environment_guard_stops_a_nested_run() {
    let r = hooked("");
    commit_ok(&r, "a.txt");
    r.git(&["commit", "-q", "--allow-empty", "-m", "x", "--no-verify"]);
    let refs = r.refs();
    let o = r
        .cmd(bin())
        .args(["hook", "run", "post-commit"])
        .env("GCMA_IN_HOOK", "1")
        .output()
        .unwrap();
    assert_eq!((Repo::code(&o), stderr(&o)), (0, String::new()));
    assert_eq!(r.refs(), refs);
}

#[test]
fn path_rules_keep_the_working_copy_and_index_in_step() {
    let r = hooked("paths:\n  exclude: [\"secrets/\"]\n");
    r.write("secrets/key.pem", "k\n");
    r.write("src.txt", "s\n");
    r.git(&["add", "-f", "secrets/key.pem", "src.txt"]);
    let o = r.git_out(&["commit", "-q", "-m", "feature"]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(
        stderr(&o).starts_with("gcma: post-commit: paths were removed from the rewritten commits"),
        "the secrets note is printed: {}",
        stderr(&o)
    );
    assert!(
        !r.git(&["ls-tree", "-r", "--name-only", "HEAD"])
            .contains("secrets/")
    );
    assert!(r.path().join("secrets/key.pem").exists());
    assert_scheduled(&r.log());
    assert!(!r.git(&["status", "--porcelain"]).contains("src.txt"));
}

#[test]
fn a_tag_left_on_an_old_commit_is_reported() {
    let r = hooked("");
    commit_ok(&r, "a.txt");
    r.git(&["tag", "v1"]);
    let o = commit(&r, "b.txt");
    assert!(o.status.success(), "{}", stderr(&o));
    let err = stderr(&o);
    assert!(
        err.contains("gcma: post-commit: tags/notes point at commits that will be rewritten"),
        "{err}"
    );
    assert!(err.contains(": v1"), "{err}");
    assert!(!err.contains("--retag"), "a hook has no --retag: {err}");
}
