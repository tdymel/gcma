//! `gcma hook`: install, uninstall, and the entry point of the installed shims.

use std::io::Read;
use std::panic::{AssertUnwindSafe, catch_unwind, set_hook, take_hook};

use crate::adapters::cli_support::args::HookCmd;
use crate::adapters::cli_support::session::{Session, now};
use crate::adapters::git_cli::NESTED_ENV;
use crate::adapters::hook_installer::{self, Hook};
use crate::application::commit_hook::{self, PostCommitOutcome, Skip};
use crate::application::push_guard::{self, PrePushOutcome, PushedRef};
use crate::domain::error::{Error, Result};

pub fn run(s: &Session, cmd: HookCmd) -> Result<()> {
    match cmd {
        HookCmd::Install { post_commit, force } => {
            let hooks: &[Hook] = if post_commit {
                &Hook::ALL
            } else {
                &[Hook::PrePush]
            };
            for path in hook_installer::install(&s.open_repo()?, hooks, force)? {
                println!("installed {}", path.display());
            }
        }
        HookCmd::Uninstall => {
            let removed = hook_installer::uninstall(&s.open_repo()?)?;
            if removed.is_empty() {
                println!("no hook installed");
            } else {
                let names: Vec<&str> = removed.iter().map(|h| h.name()).collect();
                println!("hook removed ({})", names.join(", "));
            }
        }
        HookCmd::Run { name, args } => match Hook::from_name(&name) {
            Some(Hook::PrePush) => pre_push(s, &args)?,
            Some(Hook::PostCommit) => post_commit(s),
            None => return Err(Error::Usage(format!("unsupported hook {name:?}"))),
        },
    }
    Ok(())
}

fn pre_push(s: &Session, args: &[String]) -> Result<()> {
    let (repo, cfg) = s.open()?;
    let mut stdin = String::new();
    std::io::stdin().read_to_string(&mut stdin)?;
    let remote = args.first().map(String::as_str).unwrap_or("");
    let outcome = push_guard::run_pre_push(&repo, &cfg, remote, &parse_pushed_refs(&stdin), now())?;
    if let PrePushOutcome::Rewritten { notices, .. } = &outcome {
        for line in notices {
            eprintln!("gcma: pre-push: {line}");
        }
    }
    verdict(&outcome)
}

/// Whether the push goes on; a refusal says what to do next.
fn verdict(outcome: &PrePushOutcome) -> Result<()> {
    match outcome {
        PrePushOutcome::Proceed => Ok(()),
        PrePushOutcome::Rewritten {
            branch_ref,
            commits,
            ..
        } => Err(Error::Nonconforming(format!(
            "gcma rewrote {commits} unpushed commit(s) of {branch_ref} to follow the rules; run `git push` again"
        ))),
        PrePushOutcome::Replaced { pushed_ref } => Err(Error::Nonconforming(format!(
            "{pushed_ref} would push commits that gcma replaced when it rewrote a branch; {}, \
             or undo the rewrite with `gcma restore`, and push again",
            move_hint(pushed_ref)
        ))),
        PrePushOutcome::Blocked {
            commits,
            no_upstream,
            follows_branch,
        } => {
            let hint = match no_upstream {
                true => " (the branch has no upstream: add `--from <rev>`)",
                false => "",
            };
            let (retag, what) = match follows_branch {
                true => ("", "it rewrites the unpushed part of the branch"),
                false => (
                    " --retag",
                    "it rewrites the unpushed part of the branch and moves the tags with it",
                ),
            };
            Err(Error::Nonconforming(format!(
                "{commits} commit(s) about to be pushed do not follow the gcma rules; \
                 run `gcma apply{retag}`{hint} ({what}) and push again"
            )))
        }
    }
}

/// How to point a ref left on the replaced commits at the rewritten ones, or drop it.
fn move_hint(pushed_ref: &str) -> String {
    if let Some(tag) = pushed_ref.strip_prefix("refs/tags/") {
        format!("move the tag (`git tag -f {tag} <new commit>`) or delete it (`git tag -d {tag}`)")
    } else if pushed_ref.starts_with("refs/") {
        format!(
            "move the ref (`git update-ref {pushed_ref} <new commit>`) or delete it \
             (`git update-ref -d {pushed_ref}`)"
        )
    } else {
        "push the rewritten commit instead".to_string()
    }
}

/// Set (non-empty) to have the post-commit hook say what it did.
const DEBUG_ENV: &str = "GCMA_DEBUG";

/// The commit is done and must stay done: whatever goes wrong here (even a panic) is one line on
/// stderr, and the exit code stays 0. Otherwise the hook is silent, except for its notices and, with
/// `GCMA_DEBUG`, one line on what it did.
fn post_commit(s: &Session) {
    if std::env::var_os(NESTED_ENV).is_some() {
        return;
    }
    let quiet = take_hook();
    set_hook(Box::new(|_| {}));
    let result = catch_unwind(AssertUnwindSafe(|| {
        let (repo, cfg) = s.open()?;
        commit_hook::run_post_commit(&repo, &cfg, now())
    }));
    set_hook(quiet);
    match result {
        Ok(Ok(outcome)) => {
            let debug = std::env::var_os(DEBUG_ENV).is_some_and(|v| !v.is_empty());
            for line in outcome_lines(&outcome, debug) {
                eprintln!("gcma: post-commit: {line}");
            }
        }
        Ok(Err(e)) => eprintln!("gcma: post-commit skipped: {e}"),
        Err(_) => eprintln!("gcma: post-commit skipped: internal error"),
    }
}

/// What the post-commit hook prints (after `gcma: post-commit: `): its notices always, and with
/// `debug` first a summary.
fn outcome_lines(outcome: &PostCommitOutcome, debug: bool) -> Vec<String> {
    let summary = match outcome {
        PostCommitOutcome::Skipped(why) => format!("skipped ({})", skip_reason(why)),
        PostCommitOutcome::Unchanged => {
            "skipped (the unpushed commits already follow the schedule)".to_string()
        }
        PostCommitOutcome::Rewritten {
            commits, backup_id, ..
        } => format!(
            "rewrote {commits} commit(s), backup {}",
            backup_id.as_deref().unwrap_or("?")
        ),
    };
    let notices = match outcome {
        PostCommitOutcome::Rewritten { notices, .. } => notices.as_slice(),
        _ => &[],
    };
    debug
        .then_some(summary)
        .into_iter()
        .chain(notices.iter().cloned())
        .collect()
}

fn skip_reason(why: &Skip) -> String {
    match why {
        Skip::NotRewriteMode => "hook.mode is not rewrite".into(),
        Skip::NoSchedule => "no schedule configured".into(),
        Skip::DetachedHead => "detached HEAD".into(),
        Skip::OperationInProgress(op) => format!("a {op} is in progress"),
        Skip::IndexDirty => "the index has staged changes".into(),
        Skip::NoUpstream => "no upstream and no remote tell which commits are pushed".into(),
        Skip::NoCapacity => "the schedule has no room left for the unpushed commits".into(),
    }
}

/// git's pre-push input: `<local ref> <local sha> <remote ref> <remote sha>` per line.
fn parse_pushed_refs(stdin: &str) -> Vec<PushedRef> {
    stdin
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            (parts.len() == 4).then(|| PushedRef {
                local_ref: parts[0].to_string(),
                local_sha: parts[1].to_string(),
                remote_sha: parts[3].to_string(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_skip_has_its_reason() {
        let cases = [
            (Skip::NotRewriteMode, "hook.mode is not rewrite"),
            (Skip::NoSchedule, "no schedule configured"),
            (Skip::DetachedHead, "detached HEAD"),
            (
                Skip::OperationInProgress("rebase"),
                "a rebase is in progress",
            ),
            (Skip::IndexDirty, "the index has staged changes"),
            (
                Skip::NoUpstream,
                "no upstream and no remote tell which commits are pushed",
            ),
            (
                Skip::NoCapacity,
                "the schedule has no room left for the unpushed commits",
            ),
        ];
        for (why, text) in cases {
            assert_eq!(
                outcome_lines(&PostCommitOutcome::Skipped(why), true),
                [format!("skipped ({text})")]
            );
        }
    }

    #[test]
    fn the_pre_push_verdict_says_what_to_do_next() {
        assert!(verdict(&PrePushOutcome::Proceed).is_ok());
        let rewritten = PrePushOutcome::Rewritten {
            branch_ref: "refs/heads/main".into(),
            commits: 2,
            notices: vec!["a notice".into()],
        };
        let err = verdict(&rewritten).unwrap_err().to_string();
        assert!(
            err.contains("gcma rewrote 2 unpushed commit(s) of refs/heads/main"),
            "{err}"
        );
        assert!(err.contains("run `git push` again"), "{err}");
        let blocked = |no_upstream, follows_branch| PrePushOutcome::Blocked {
            commits: 3,
            no_upstream,
            follows_branch,
        };
        let err = verdict(&blocked(true, true)).unwrap_err().to_string();
        assert!(
            err.contains("3 commit(s) about to be pushed do not follow the gcma rules"),
            "{err}"
        );
        assert!(
            err.contains("run `gcma apply` (the branch has no upstream: add `--from <rev>`)"),
            "{err}"
        );
        let err = verdict(&blocked(false, true)).unwrap_err().to_string();
        assert!(!err.contains("--from"), "{err}");
        assert!(!err.contains("--retag"), "{err}");
        let err = verdict(&blocked(false, false)).unwrap_err().to_string();
        assert!(err.contains("run `gcma apply --retag`"), "{err}");
        assert!(err.contains("moves the tags"), "{err}");
        let replaced = |r: &str| {
            let outcome = PrePushOutcome::Replaced {
                pushed_ref: r.into(),
            };
            verdict(&outcome).unwrap_err().to_string()
        };
        let err = replaced("refs/tags/v1");
        assert!(
            err.contains("refs/tags/v1 would push commits that gcma replaced"),
            "{err}"
        );
        assert!(err.contains("`git tag -f v1 <new commit>`"), "{err}");
        assert!(err.contains("`git tag -d v1`"), "{err}");
        assert!(err.contains("`gcma restore`"), "{err}");
        let err = replaced("refs/keep/x");
        assert!(err.contains("`git update-ref -d refs/keep/x`"), "{err}");
        assert!(replaced("abc123").contains("push the rewritten commit instead"));
    }

    #[test]
    fn without_debug_only_the_notices_are_printed() {
        let rewritten = PostCommitOutcome::Rewritten {
            commits: 3,
            backup_id: Some("20260101T000000Z-aaa-bbb".into()),
            notices: vec!["resetting the index failed".into()],
        };
        assert_eq!(
            outcome_lines(&rewritten, true),
            [
                "rewrote 3 commit(s), backup 20260101T000000Z-aaa-bbb",
                "resetting the index failed"
            ]
        );
        assert_eq!(
            outcome_lines(&rewritten, false),
            ["resetting the index failed"]
        );
        assert!(outcome_lines(&PostCommitOutcome::Unchanged, false).is_empty());
        assert!(outcome_lines(&PostCommitOutcome::Skipped(Skip::NoSchedule), false).is_empty());
    }
}
