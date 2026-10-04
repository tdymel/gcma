//! What the pre-push hook tells git and the user: whether the push goes on, and when it does not,
//! what to do next.

use crate::application::push_guard::{NeedsFrom, PrePushOutcome, RefKind};
use crate::domain::error::{Error, Result};
use crate::domain::history::plan::HEADS_PREFIX;
use crate::domain::history::tag::TAGS_PREFIX;

/// Whether the push goes on; a refusal says what to do next.
pub(super) fn verdict(outcome: &PrePushOutcome) -> Result<()> {
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
            needs_from,
            kind,
        } => {
            let hint = match needs_from {
                Some(NeedsFrom::NoUpstream) => " (the branch has no upstream: add `--from <rev>`)",
                Some(NeedsFrom::LocalUpstream) => {
                    " (the upstream is a local branch that has the commits too: add `--from <rev>`)"
                }
                None => "",
            };
            let rewrites = "it rewrites the unpushed part of the branch";
            let (retag, what, then) = match kind {
                RefKind::Branch => ("", rewrites.to_string(), String::new()),
                RefKind::Tags => (
                    " --retag",
                    format!("{rewrites} and moves the tags with it"),
                    String::new(),
                ),
                RefKind::Other(r) => (
                    "",
                    rewrites.to_string(),
                    format!(", then {},", move_hint(r)),
                ),
            };
            Err(Error::Nonconforming(format!(
                "{commits} commit(s) about to be pushed do not follow the gcma rules; \
                 run `gcma apply{retag}`{hint} ({what}){then} and push again"
            )))
        }
    }
}

/// How to point a ref left on the old commits at the rewritten ones, or drop it.
fn move_hint(pushed_ref: &str) -> String {
    if let Some(branch) = pushed_ref.strip_prefix(HEADS_PREFIX) {
        format!(
            "move the branch (`git branch -f {branch} <new commit>`) or delete it \
             (`git branch -D {branch}`)"
        )
    } else if let Some(tag) = pushed_ref.strip_prefix(TAGS_PREFIX) {
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

#[cfg(test)]
mod tests {
    use super::*;

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
        let blocked = |needs_from, kind| PrePushOutcome::Blocked {
            commits: 3,
            needs_from,
            kind,
        };
        let err = verdict(&blocked(Some(NeedsFrom::NoUpstream), RefKind::Branch))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("3 commit(s) about to be pushed do not follow the gcma rules"),
            "{err}"
        );
        assert!(
            err.contains("run `gcma apply` (the branch has no upstream: add `--from <rev>`)"),
            "{err}"
        );
        let err = verdict(&blocked(Some(NeedsFrom::LocalUpstream), RefKind::Branch))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("(the upstream is a local branch that has the commits too: add `--from"),
            "{err}"
        );
        let err = verdict(&blocked(None, RefKind::Branch))
            .unwrap_err()
            .to_string();
        assert!(!err.contains("--from"), "{err}");
        assert!(!err.contains("--retag"), "{err}");
        let err = verdict(&blocked(None, RefKind::Tags))
            .unwrap_err()
            .to_string();
        assert!(err.contains("run `gcma apply --retag`"), "{err}");
        assert!(err.contains("moves the tags"), "{err}");
        let other = RefKind::Other("refs/heads/feature".into());
        let err = verdict(&blocked(None, other)).unwrap_err().to_string();
        assert!(!err.contains("--retag"), "{err}");
        assert!(
            err.contains(
                "run `gcma apply` (it rewrites the unpushed part of the branch), then move the \
                 branch (`git branch -f feature <new commit>`) or delete it \
                 (`git branch -D feature`), and push again"
            ),
            "{err}"
        );
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
}
