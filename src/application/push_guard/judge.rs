//! Which pushes the rules judge, and which of them one plan covers.

use std::collections::HashSet;

use super::sent::Sent;
use crate::application::ports::{Repository, RevRange};
use crate::domain::error::Result;
use crate::domain::history::plan::{HEADS_PREFIX, Plan};
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

/// Whether the push of `s` is judged, and what it names. The checked-out branch is judged by name,
/// as `HEAD` (which is what `git push origin HEAD` and `HEAD:<ref>` report) or as a raw revision
/// that is part of it (`git push origin HEAD~1:main` reports `HEAD~1`). Other branches are ignored,
/// unless they point at the tip. Any other ref (a tag, `refs/<anything>`) is judged when its commit
/// is the tip or an ancestor of it; one that points at a commit that is not part of the branch is
/// not judged (but see `sends_replaced`).
pub(super) fn kind(
    repo: &dyn Repository,
    s: &Sent,
    branch_ref: &str,
    tip: Option<&str>,
) -> Result<Option<RefKind>> {
    let r = s.pushed.local_ref.as_str();
    let commit = s.revs.tip.as_str();
    let judged = if r == branch_ref || r == "HEAD" || tip == Some(commit) {
        true
    } else if r.starts_with(HEADS_PREFIX) {
        false
    } else {
        match tip {
            Some(t) => repo.is_ancestor(commit, t)?,
            None => false,
        }
    };
    let kind = if r == branch_ref || r == "HEAD" || !r.starts_with("refs/") {
        RefKind::Branch
    } else if r.starts_with(TAGS_PREFIX) {
        RefKind::Tags
    } else {
        RefKind::Other(r.to_string())
    };
    Ok(judged.then_some(kind))
}

/// Judged pushes that one plan covers: those whose commits are part of the range of `revs`, the
/// highest of them, with the same exclusions (so each of their ranges is part of it). Many tags of
/// one history make one group, not a plan each.
pub(super) struct Group {
    pub revs: RevRange,
    pub base: Option<String>,
}

/// Adds the judged push of `s` to the group that covers it, or starts a new group.
pub(super) fn add_to_groups(
    repo: &dyn Repository,
    groups: &mut Vec<Group>,
    s: &Sent,
) -> Result<()> {
    let commit = &s.revs.tip;
    let same = |g: &Group| {
        g.revs.exclude_commits == s.revs.exclude_commits
            && g.revs.exclude_remotes == s.revs.exclude_remotes
    };
    for g in groups.iter_mut().filter(|g| same(g)) {
        let below = &g.revs.tip == commit || repo.is_ancestor(commit, &g.revs.tip)?;
        if below || repo.is_ancestor(&g.revs.tip, commit)? {
            if !below {
                g.revs.tip = commit.clone();
            }
            return Ok(());
        }
    }
    groups.push(Group {
        revs: s.revs.clone(),
        base: s.base.clone(),
    });
    Ok(())
}

/// What the rewrite of `plan` has to take along: the kinds of the judged pushes (`judged`: its
/// commit and kind) whose commit the plan rewrites or drops, joined (the tip of the plan's group is
/// one of them). A push on a commit the plan keeps follows nothing, also when it is in the group;
/// one that excludes less than the group's (a new tag or branch next to the pushed branch) is
/// grouped apart, but it would stay on the old commit all the same.
pub(super) fn plan_kind(judged: &[(String, RefKind)], plan: &Plan) -> RefKind {
    let touched: HashSet<&String> = plan.touched_oids().collect();
    judged
        .iter()
        .filter(|(commit, _)| touched.contains(commit))
        .fold(RefKind::Branch, |k, (_, other)| k.join(other.clone()))
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
