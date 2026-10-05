//! What the pre-push hook tells git and the user: whether the push goes on, and when it does not,
//! what to do next.

use crate::application::push_guard::{PrePushOutcome, Published, RefKind, Start};
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
            start,
            rewrite_pushed,
            published,
            remote,
            kind,
        } => {
            let rules =
                format!("{commits} commit(s) about to be pushed do not follow the gcma rules");
            let lead = match published {
                Published::All => format!(
                    "they are on another remote already: if {remote} may have them as they are, \
                     push with `--no-verify`; otherwise "
                ),
                Published::Partly(n) => format!("{n} of them are on another remote already: "),
                Published::Nowhere => String::new(),
            };
            let direct = direct_route(*start, *rewrite_pushed, kind);
            let route = if *start == Start::LocalUpstream {
                let retag = if *kind == RefKind::Tags {
                    " --retag"
                } else {
                    ""
                };
                let follow = match kind {
                    RefKind::Branch => String::new(),
                    RefKind::Tags => ", then move the tags of this branch's own commits to the \
                                      rebased ones (`git tag -f <tag> <new commit>`)"
                        .to_string(),
                    RefKind::Other(r) => format!(", then {}", move_hint(r)),
                };
                format!(
                    "the upstream is a local branch that has some of them too: run `gcma \
                     apply{retag}` on it and rebase this branch onto it{follow}, or run {direct}"
                )
            } else if *published == Published::Nowhere {
                format!("run {direct}")
            } else {
                format!("rewrite them with {direct}")
            };
            Err(Error::Nonconforming(format!(
                "{rules}; {lead}{route} and push again"
            )))
        }
    }
}

/// The `gcma apply` that rewrites the blocked commits in place, what it does, and what has to
/// follow by hand.
fn direct_route(start: Start, rewrite_pushed: bool, kind: &RefKind) -> String {
    let retag = if *kind == RefKind::Tags {
        " --retag"
    } else {
        ""
    };
    let from = if start.needs_from() {
        " --from <rev>"
    } else {
        ""
    };
    let pushed = if rewrite_pushed {
        " --rewrite-pushed"
    } else {
        ""
    };
    let mut what = match (start.needs_from(), rewrite_pushed) {
        (false, false) => "it rewrites the unpushed part of the branch",
        (true, false) => "it rewrites the branch after <rev>",
        (false, true) => {
            "it rewrites the branch from where it leaves the upstream, pushed commits included"
        }
        (true, true) => "it rewrites the branch after <rev>, pushed commits included",
    }
    .to_string();
    if start == Start::NoUpstream {
        what = format!("the branch has no upstream; {what}");
    }
    let then = match kind {
        RefKind::Branch => String::new(),
        RefKind::Tags => {
            let comma = if rewrite_pushed { "," } else { "" };
            what.push_str(&format!("{comma} and moves the tags with it"));
            String::new()
        }
        RefKind::Other(r) => format!(", then {},", move_hint(r)),
    };
    format!("`gcma apply{retag}{from}{pushed}` ({what}){then}")
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

    fn blocked(start: Start, rewrite_pushed: bool, published: Published, kind: RefKind) -> String {
        let outcome = PrePushOutcome::Blocked {
            commits: 3,
            start,
            rewrite_pushed,
            published,
            remote: "origin".into(),
            kind,
        };
        verdict(&outcome).unwrap_err().to_string()
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
    fn a_blocked_push_on_the_upstream_needs_neither_flag() {
        let err = blocked(Start::Upstream, false, Published::Nowhere, RefKind::Branch);
        assert!(
            err.contains(
                "3 commit(s) about to be pushed do not follow the gcma rules; run `gcma apply` \
                 (it rewrites the unpushed part of the branch) and push again"
            ),
            "{err}"
        );
        let err = blocked(Start::Upstream, false, Published::Nowhere, RefKind::Tags);
        assert!(
            err.contains("run `gcma apply --retag` (it rewrites the unpushed part of the branch and moves the tags with it)"),
            "{err}"
        );
        let other = RefKind::Other("refs/heads/feature".into());
        let err = blocked(Start::Upstream, false, Published::Nowhere, other);
        assert!(!err.contains("--retag"), "{err}");
        assert!(
            err.contains(
                "run `gcma apply` (it rewrites the unpushed part of the branch), then move the \
                 branch (`git branch -f feature <new commit>`) or delete it \
                 (`git branch -D feature`), and push again"
            ),
            "{err}"
        );
    }

    #[test]
    fn from_and_rewrite_pushed_are_asked_for_only_when_needed() {
        let err = blocked(
            Start::NoUpstream,
            false,
            Published::Nowhere,
            RefKind::Branch,
        );
        assert!(
            err.contains(
                "run `gcma apply --from <rev>` (the branch has no upstream; it rewrites the \
                 branch after <rev>)"
            ),
            "{err}"
        );
        assert!(!err.contains("--rewrite-pushed"), "{err}");
        // A peer's commits on top of a remote-tracking upstream: apply starts there already.
        let err = blocked(Start::Upstream, true, Published::All, RefKind::Branch);
        assert!(
            err.contains(
                "rewrite them with `gcma apply --rewrite-pushed` (it rewrites the branch from \
                 where it leaves the upstream, pushed commits included)"
            ),
            "{err}"
        );
        assert!(!err.contains("--from"), "{err}");
        let err = blocked(Start::NoUpstream, true, Published::All, RefKind::Tags);
        assert!(
            err.contains(
                "`gcma apply --retag --from <rev> --rewrite-pushed` (the branch has no upstream; \
                 it rewrites the branch after <rev>, pushed commits included, and moves the tags \
                 with it)"
            ),
            "{err}"
        );
    }

    #[test]
    fn no_verify_is_a_choice_about_the_remote_pushed_to() {
        let err = blocked(Start::Upstream, true, Published::All, RefKind::Branch);
        assert!(
            err.contains(
                "3 commit(s) about to be pushed do not follow the gcma rules; they are on another \
                 remote already: if origin may have them as they are, push with `--no-verify`; \
                 otherwise rewrite them with `gcma apply --rewrite-pushed`"
            ),
            "{err}"
        );
        let err = blocked(Start::Upstream, true, Published::Partly(1), RefKind::Branch);
        assert!(
            err.contains(
                "1 of them are on another remote already: rewrite them with `gcma apply \
                 --rewrite-pushed`"
            ),
            "{err}"
        );
        assert!(!err.contains("--no-verify"), "not all are pushed: {err}");
    }

    #[test]
    fn a_local_upstream_route_takes_the_tags_and_refs_along_too() {
        let err = blocked(
            Start::LocalUpstream,
            true,
            Published::Nowhere,
            RefKind::Branch,
        );
        assert!(
            err.contains(
                "the upstream is a local branch that has some of them too: run `gcma apply` on it \
                 and rebase this branch onto it, or run `gcma apply --from <rev> \
                 --rewrite-pushed` (it rewrites the branch after <rev>, pushed commits included) \
                 and push again"
            ),
            "{err}"
        );
        assert!(!err.contains("--no-verify"), "{err}");
        let err = blocked(
            Start::LocalUpstream,
            true,
            Published::Nowhere,
            RefKind::Tags,
        );
        assert!(
            err.contains(
                "run `gcma apply --retag` on it and rebase this branch onto it, then move the \
                 tags of this branch's own commits to the rebased ones (`git tag -f <tag> <new \
                 commit>`), or run `gcma apply --retag --from <rev> --rewrite-pushed` (it \
                 rewrites the branch after <rev>, pushed commits included, and moves the tags \
                 with it) and push again"
            ),
            "{err}"
        );
        let other = RefKind::Other("refs/heads/feature".into());
        let err = blocked(Start::LocalUpstream, true, Published::Nowhere, other);
        assert!(
            err.contains(
                "rebase this branch onto it, then move the branch (`git branch -f feature <new \
                 commit>`) or delete it (`git branch -D feature`), or run"
            ),
            "{err}"
        );
        assert!(
            err.contains("pushed commits included), then move the branch"),
            "{err}"
        );
    }
}
