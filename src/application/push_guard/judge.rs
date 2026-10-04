//! Which pushes the rules judge, and which of them one plan covers.

use super::sent::{PushedRef, Sent};
use crate::application::ports::{Repository, RevRange};
use crate::domain::error::Result;
use crate::domain::history::plan::HEADS_PREFIX;
use crate::domain::history::tag::TAGS_PREFIX;

/// What a judged push names, which tells how its ref follows a rewrite of the branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefKind {
    /// The branch itself, `HEAD` or a raw revision: after a rewrite of the branch, pushing again
    /// sends the new commits.
    Branch,
    /// Tags, which would still send the old commits; `gcma apply --retag` moves them along.
    Tags,
    /// Another branch or ref (the first one): it stays on the old commits until it is moved by hand.
    Other(String),
}

impl RefKind {
    /// The kind of a group of pushes: the one that needs the most to follow a rewrite.
    fn join(self, other: RefKind) -> RefKind {
        match (self, other) {
            (o @ RefKind::Other(_), _) | (_, o @ RefKind::Other(_)) => o,
            (RefKind::Tags, _) | (_, RefKind::Tags) => RefKind::Tags,
            _ => RefKind::Branch,
        }
    }
}

/// The commit a judged push sends.
pub(super) struct Judged {
    /// The pushed object peeled to a commit (an annotated tag pushes its tag object).
    pub commit: String,
    pub kind: RefKind,
}

/// Which pushes are judged, and the commit they send. The checked-out branch is judged by name, as
/// `HEAD` (which is what `git push origin HEAD` and `HEAD:<ref>` report) or as a raw revision that is
/// part of it (`git push origin HEAD~1:main` reports `HEAD~1`). Other branches are ignored, unless
/// they point at the tip. Any other ref (a tag, `refs/<anything>`) is judged when its commit is the
/// tip or an ancestor of it; one that points at a commit that is not part of the branch is not
/// judged (but see `sends_replaced`). `commit` is the pushed object peeled to a commit.
pub(super) fn judged_commit(
    repo: &dyn Repository,
    p: &PushedRef,
    commit: String,
    branch_ref: &str,
    tip: Option<&str>,
) -> Result<Option<Judged>> {
    let r = p.local_ref.as_str();
    let kind = if r == branch_ref || r == "HEAD" || !r.starts_with("refs/") {
        RefKind::Branch
    } else if r.starts_with(TAGS_PREFIX) {
        RefKind::Tags
    } else {
        RefKind::Other(r.to_string())
    };
    let at_tip = tip == Some(commit.as_str());
    let judged = if r == branch_ref || r == "HEAD" || at_tip {
        true
    } else if r.starts_with(HEADS_PREFIX) {
        false
    } else {
        match tip {
            Some(t) => repo.is_ancestor(&commit, t)?,
            None => false,
        }
    };
    Ok(judged.then_some(Judged { commit, kind }))
}

/// Judged pushes that one plan covers: those whose commits are part of the range of `revs`, the
/// highest of them, with the same exclusions (so each of their ranges is part of it). Many tags of
/// one history make one group, not a plan each.
pub(super) struct Group {
    pub revs: RevRange,
    pub base: Option<String>,
    /// What the pushes of the group name, joined.
    pub kind: RefKind,
}

/// Adds a judged push to the group that covers it, or starts a new group.
pub(super) fn add_to_groups(
    repo: &dyn Repository,
    groups: &mut Vec<Group>,
    sent: &Sent,
    judged: Judged,
) -> Result<()> {
    let same = |g: &Group| {
        g.revs.exclude_commits == sent.revs.exclude_commits
            && g.revs.exclude_remotes == sent.revs.exclude_remotes
    };
    for g in groups.iter_mut().filter(|g| same(g)) {
        let below = g.revs.tip == judged.commit || repo.is_ancestor(&judged.commit, &g.revs.tip)?;
        if below || repo.is_ancestor(&g.revs.tip, &judged.commit)? {
            if !below {
                g.revs.tip = judged.commit;
            }
            g.kind = std::mem::replace(&mut g.kind, RefKind::Branch).join(judged.kind);
            return Ok(());
        }
    }
    groups.push(Group {
        revs: RevRange {
            tip: judged.commit,
            ..sent.revs.clone()
        },
        base: sent.base.clone(),
        kind: judged.kind,
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::RefKind::*;

    #[test]
    fn a_group_needs_what_its_most_demanding_push_needs() {
        let other = || Other("refs/heads/x".to_string());
        assert_eq!(Branch.join(Branch), Branch);
        assert_eq!(Branch.join(Tags), Tags);
        assert_eq!(Tags.join(Branch), Tags);
        assert_eq!(Tags.join(other()), other());
        assert_eq!(other().join(Tags), other());
        assert_eq!(
            other().join(Other("refs/y".into())),
            other(),
            "the first one"
        );
    }
}
