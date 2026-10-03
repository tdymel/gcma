//! Which pushes the rules judge, and which of them one plan covers.

use super::sent::{PushedRef, Sent};
use crate::application::ports::{Repository, RevRange};
use crate::domain::error::Result;
use crate::domain::history::plan::HEADS_PREFIX;

/// The commit a judged push sends.
pub(super) struct Judged {
    /// The pushed object peeled to a commit (an annotated tag pushes its tag object).
    pub commit: String,
    /// The push names the branch itself, `HEAD` or a raw revision, so after a rewrite of the branch
    /// pushing again sends the new commits; a tag or another ref would still send the old ones.
    pub follows_branch: bool,
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
    let follows_branch =
        p.local_ref == branch_ref || p.local_ref == "HEAD" || !p.local_ref.starts_with("refs/");
    let at_tip = tip == Some(commit.as_str());
    let judged = if p.local_ref == branch_ref || p.local_ref == "HEAD" || at_tip {
        true
    } else if p.local_ref.starts_with(HEADS_PREFIX) {
        false
    } else {
        match tip {
            Some(t) => repo.is_ancestor(&commit, t)?,
            None => false,
        }
    };
    Ok(judged.then_some(Judged {
        commit,
        follows_branch,
    }))
}

/// Judged pushes that one plan covers: those whose commits are part of the range of `revs`, the
/// highest of them, with the same exclusions (so each of their ranges is part of it). Many tags of
/// one history make one group, not a plan each.
pub(super) struct Group {
    pub revs: RevRange,
    pub base: Option<String>,
    /// Every push of the group follows the branch (see `Judged`).
    pub follows_branch: bool,
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
            g.follows_branch &= judged.follows_branch;
            return Ok(());
        }
    }
    groups.push(Group {
        revs: RevRange {
            tip: judged.commit,
            ..sent.revs.clone()
        },
        base: sent.base.clone(),
        follows_branch: judged.follows_branch,
    });
    Ok(())
}
