//! Fault injection for the apply use case: a repository that behaves like the real one except for
//! one deliberate defect, to prove that verification stops a bad rewrite before any ref moves.

use std::collections::HashSet;
use std::process::Command;

use delegate::delegate;

use crate::adapters::config_file;
use crate::adapters::git_cli::GitCli;
use crate::application::planning::{PlanOptions, build_plan};
use crate::application::ports::{
    CommitStore, History, NoteStore, RefStore, RefUpdate, RevRange, TagRef, TagStore, TreeEntry,
    TreeStore, WorkTree,
};
use crate::application::rewrite::apply;
use crate::domain::error::{Error, Result};
use crate::domain::history::commit::{Commit, NewCommit};
use crate::domain::history::plan::Plan;
use crate::domain::settings::Signing;

#[derive(Clone, Copy, Debug)]
enum Fault {
    WrongTree,
    NoParents,
    WrongMessage,
    LaterCommitter,
    TipTreeDiffers,
    UnchangedNotReachable,
    BranchMovesBeforeTransaction,
}

/// The git CLI with one `Fault` switched on.
struct Faulty {
    inner: GitCli,
    fault: Fault,
    /// A valid tree that is not the one any rewritten commit should have.
    other_tree: String,
    /// A commit the branch is moved to by `BranchMovesBeforeTransaction`.
    other_commit: String,
}

impl CommitStore for Faulty {
    delegate! {
        to self.inner {
            fn read_commits(&self, oids: &[String]) -> Result<Vec<Commit>>;
            fn objects_exist(&self, oids: &[String]) -> Result<bool>;
        }
    }
    fn write_commit(&self, c: &NewCommit, signing: Signing) -> Result<String> {
        let mut message = c.message.to_vec();
        let mut committer = c.committer.clone();
        let (mut tree, mut parents) = (c.tree.to_string(), c.parents.to_vec());
        match self.fault {
            Fault::WrongTree => tree = self.other_tree.clone(),
            Fault::NoParents => parents.clear(),
            Fault::WrongMessage => message.extend_from_slice(b"\nsmuggled\n"),
            Fault::LaterCommitter => committer.time += 1,
            _ => {}
        }
        let tampered = NewCommit {
            tree: &tree,
            parents: &parents,
            author: c.author,
            committer: &committer,
            extra: c.extra.clone(),
            message: &message,
        };
        self.inner.write_commit(&tampered, signing)
    }
}

impl TreeStore for Faulty {
    delegate! {
        to self.inner {
            fn read_tree(&self, oid: &str) -> Result<Vec<TreeEntry>>;
            fn write_tree(&self, entries: &[TreeEntry]) -> Result<String>;
            fn read_blob(&self, oid: &str) -> Result<Vec<u8>>;
            fn write_blob(&self, data: &[u8]) -> Result<String>;
        }
    }
}

impl RefStore for Faulty {
    delegate! {
        to self.inner {
            fn current_branch_ref(&self) -> Result<Option<String>>;
            fn ref_value(&self, name: &str) -> Result<Option<String>>;
            fn resolve_commit(&self, rev: &str) -> Result<Option<String>>;
            fn upstream_oid(&self, branch_ref: &str) -> Result<Option<String>>;
            fn list_refs(&self, prefix: &str) -> Result<Vec<(String, String)>>;
        }
    }
    fn update_refs(&self, message: &str, updates: &[RefUpdate]) -> Result<()> {
        if let Fault::BranchMovesBeforeTransaction = self.fault {
            let branch = self.inner.current_branch_ref()?.expect("on a branch");
            let set = RefUpdate::Set {
                name: branch,
                new: self.other_commit.clone(),
            };
            self.inner.update_refs("concurrent writer", &[set])?;
        }
        self.inner.update_refs(message, updates)
    }
    delegate! {
        to self.inner {
            fn remotes(&self) -> Result<Vec<String>>;
        }
    }
}

impl TagStore for Faulty {
    delegate! {
        to self.inner {
            fn list_tags(&self) -> Result<Vec<TagRef>>;
            fn read_tag_object(&self, oid: &str) -> Result<Vec<u8>>;
            fn write_tag_object(&self, raw: &[u8]) -> Result<String>;
        }
    }
}

impl NoteStore for Faulty {
    delegate! {
        to self.inner {
            fn list_notes(&self) -> Result<Vec<(String, String)>>;
            fn copy_note(&self, notes_ref: &str, from: &str, to: &str) -> Result<()>;
        }
    }
}

impl History for Faulty {
    delegate! {
        to self.inner {
            fn merge_base(&self, a: &str, b: &str) -> Result<Option<String>>;
            fn is_ancestor(&self, ancestor: &str, descendant: &str) -> Result<bool>;
            fn list_range(&self, range: &RevRange) -> Result<Vec<String>>;
            fn count_reachable(&self, rev: &str) -> Result<usize>;
            fn unpushed_among(&self, oids: &[String], upstream: &str) -> Result<HashSet<String>>;
        }
    }
    fn all_reachable_from(&self, commits: &[String], tip: &str) -> Result<bool> {
        match self.fault {
            Fault::UnchangedNotReachable => Ok(false),
            _ => self.inner.all_reachable_from(commits, tip),
        }
    }
    fn same_tree(&self, a: &str, b: &str) -> Result<bool> {
        match self.fault {
            Fault::TipTreeDiffers => Ok(false),
            _ => self.inner.same_tree(a, b),
        }
    }
    delegate! {
        to self.inner {
            fn changed_paths(&self, a: &str, b: &str) -> Result<Vec<String>>;
            fn change_stats(&self, oids: &[String]) -> Result<Vec<(u64, u64, u64)>>;
        }
    }
}

impl WorkTree for Faulty {
    delegate! {
        to self.inner {
            fn is_shallow(&self) -> Result<bool>;
            fn has_replace_refs(&self) -> Result<bool>;
            fn has_grafts(&self) -> Result<bool>;
            fn index_dirty(&self) -> Result<bool>;
            fn operation_in_progress(&self) -> Result<Option<&'static str>>;
            fn read_file(&self, rel: &str) -> Result<Option<Vec<u8>>>;
            fn write_file(&self, rel: &str, data: &[u8]) -> Result<()>;
            fn remove_file(&self, rel: &str) -> Result<()>;
            fn reset_index_to_head(&self) -> Result<()>;
        }
    }
}

const CONFIG: &str = "version: 1\nidentity:\n  - match: {email: me@home.org}\n    set: {name: Jane Doe, email: jane@work.com}\n";
const NOW: i64 = 1_800_000_000;

struct Fixture {
    dir: tempfile::TempDir,
    root: String,
    old_tip: String,
}

fn git(dir: &std::path::Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "Old Me")
        .env("GIT_AUTHOR_EMAIL", "me@home.org")
        .env("GIT_COMMITTER_NAME", "Old Me")
        .env("GIT_COMMITTER_EMAIL", "me@home.org")
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Three linear commits by an identity the config rewrites, on `main`.
fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    git(p, &["config", "commit.gpgsign", "false"]);
    let mut root = String::new();
    for name in ["a", "b", "c"] {
        std::fs::write(p.join(name), name).unwrap();
        git(p, &["add", name]);
        git(p, &["commit", "-q", "-m", &format!("add {name}")]);
        if root.is_empty() {
            root = git(p, &["rev-parse", "HEAD"]);
        }
    }
    let old_tip = git(p, &["rev-parse", "HEAD"]);
    Fixture { dir, root, old_tip }
}

fn plan_for(repo: &GitCli) -> Plan {
    let cfg = config_file::parse(CONFIG).unwrap();
    let opts = PlanOptions {
        from_rev: Some("root".into()),
        ..PlanOptions::new(NOW)
    };
    let plan = build_plan(repo, &cfg, &opts).unwrap().plan;
    assert_eq!(
        plan.entries.len(),
        3,
        "the fixture must have commits to rewrite"
    );
    plan
}

/// Applies the plan on a repository with `fault` switched on; returns the error and checks that
/// nothing was left behind: the branch is where it was and no backup or other gcma ref exists.
fn apply_with(fault: Fault) -> Error {
    let fx = fixture();
    let inner = GitCli::open(fx.dir.path()).unwrap();
    let plan = plan_for(&inner);
    let other_tree = inner.read_commits(std::slice::from_ref(&fx.root)).unwrap()[0]
        .tree
        .clone();
    let repo = Faulty {
        inner,
        fault,
        other_tree,
        other_commit: fx.root.clone(),
    };
    let err = apply(&repo, &plan, false, NOW).expect_err("the fault must be caught");
    let tip = git(fx.dir.path(), &["rev-parse", "refs/heads/main"]);
    let expected = match fault {
        Fault::BranchMovesBeforeTransaction => fx.root.clone(),
        _ => fx.old_tip.clone(),
    };
    assert_eq!(
        tip, expected,
        "the branch must not have been moved by apply"
    );
    assert!(
        repo.list_refs("refs/gcma/").unwrap().is_empty(),
        "no backup refs"
    );
    err
}

#[test]
fn verification_rejects_a_commit_with_the_wrong_tree() {
    assert_internal(apply_with(Fault::WrongTree), "tree of entry");
}

#[test]
fn verification_rejects_a_commit_with_the_wrong_parents() {
    assert_internal(apply_with(Fault::NoParents), "parents of entry");
}

#[test]
fn verification_rejects_a_commit_with_a_different_message() {
    assert_internal(apply_with(Fault::WrongMessage), "message of entry");
}

#[test]
fn verification_rejects_a_commit_with_a_different_date() {
    assert_internal(apply_with(Fault::LaterCommitter), "identity/date of entry");
}

#[test]
fn verification_rejects_a_tip_whose_tree_differs_from_the_original() {
    assert_internal(apply_with(Fault::TipTreeDiffers), "tip tree differs");
}

#[test]
fn verification_rejects_unchanged_commits_that_fell_out_of_the_history() {
    // The fixture rewrites the whole branch, so there are no kept parents; give the check some by
    // planning from the first commit.
    let fx = fixture();
    let inner = GitCli::open(fx.dir.path()).unwrap();
    let cfg = config_file::parse(CONFIG).unwrap();
    let opts = PlanOptions {
        from_rev: Some(fx.root.clone()),
        ..PlanOptions::new(NOW)
    };
    let plan = build_plan(&inner, &cfg, &opts).unwrap().plan;
    let repo = Faulty {
        inner,
        fault: Fault::UnchangedNotReachable,
        other_tree: String::new(),
        other_commit: String::new(),
    };
    let err = apply(&repo, &plan, false, NOW).expect_err("caught");
    assert_internal(err, "not reachable from the new tip");
    assert_eq!(git(fx.dir.path(), &["rev-parse", "main"]), fx.old_tip);
    assert!(repo.list_refs("refs/gcma/").unwrap().is_empty());
}

#[test]
fn a_branch_that_moves_between_verification_and_the_transaction_is_not_overwritten() {
    let err = apply_with(Fault::BranchMovesBeforeTransaction);
    assert!(matches!(err, Error::TipMoved(_)), "got {err:?}");
}

fn assert_internal(err: Error, what: &str) {
    match err {
        Error::Internal(m) => {
            assert!(m.contains("no ref was changed"), "{m}");
            assert!(m.contains(what), "expected {what:?} in {m:?}");
        }
        other => panic!("expected an internal verification failure, got {other:?}"),
    }
}
