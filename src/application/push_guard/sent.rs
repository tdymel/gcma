//! What a push sends: the pushed refs and, for each, the range of commits that are not on the
//! remote yet.

use crate::application::ports::{RemoteScope, Repository, RevRange};
use crate::domain::error::Result;
use crate::domain::history::commit::is_zero_oid;
use crate::domain::history::plan::HEADS_PREFIX;

/// One ref a push is about to update, as git reports it to the hook.
#[derive(Debug, Clone)]
pub struct PushedRef {
    pub local_ref: String,
    pub local_sha: String,
    pub remote_sha: String,
}

/// A pushed ref and the commits it would send.
pub(super) struct Sent<'a> {
    pub pushed: &'a PushedRef,
    /// `tip` is the pushed object peeled to a commit (an annotated tag pushes its tag object).
    pub revs: RevRange,
    /// The commit the range starts after, when it is a single one.
    pub base: Option<String>,
}

/// Where the push goes, as far as the ranges need to know.
pub(super) struct Destination<'a> {
    remote: &'a str,
    /// `remote` names a configured remote (it is not a URL).
    named: bool,
    /// The named remote has remote-tracking refs (asked only when a ref other than a branch is
    /// pushed, as only those are checked for replaced commits).
    tracked: bool,
    /// The upstream of the checked-out branch.
    pub upstream: Option<String>,
}

/// Where `remote` (a name or a URL) is, and the upstream of the checked-out branch, if any.
pub(super) fn destination<'a>(
    repo: &dyn Repository,
    remote: &'a str,
    pushed: &[PushedRef],
    branch_ref: Option<&str>,
) -> Result<Destination<'a>> {
    let named = repo.remotes()?.iter().any(|r| r == remote);
    let tracked = named
        && pushed
            .iter()
            .any(|p| !p.local_ref.starts_with(HEADS_PREFIX))
        && !repo
            .list_refs(&format!("refs/remotes/{remote}"))?
            .is_empty();
    let upstream = match branch_ref {
        Some(b) => repo.upstream_oid(b)?,
        None => None,
    };
    Ok(Destination {
        remote,
        named,
        tracked,
        upstream,
    })
}

/// A tag or another ref that is neither a branch nor `HEAD`.
fn is_other_ref(p: &PushedRef) -> bool {
    p.local_ref.starts_with("refs/") && !p.local_ref.starts_with(HEADS_PREFIX)
}

/// The commits the push would send: `remote_sha..commit`, or for a remote sha we do not have
/// (a new branch or tag) everything not on the remote's tracking refs; and the commit the range
/// starts after, when it is a single one. A tag (or other ref) pushed to a remote without tracking
/// refs (a new one, or a URL) sends what neither any remote nor the branch's upstream has: history
/// already published elsewhere is not judged again.
pub(super) fn unpushed_range(
    repo: &dyn Repository,
    p: &PushedRef,
    commit: String,
    dest: &Destination,
) -> Result<(RevRange, Option<String>)> {
    let known_remote = match is_zero_oid(&p.remote_sha) {
        true => None,
        false => repo.resolve_commit(&p.remote_sha)?,
    };
    let mut revs = RevRange {
        tip: commit,
        exclude_commits: Vec::new(),
        exclude_remotes: None,
    };
    if let Some(pushed) = known_remote {
        revs.exclude_commits.push(pushed.clone());
        return Ok((revs, Some(pushed)));
    }
    let other_ref = is_other_ref(p);
    revs.exclude_remotes = Some(if dest.named && (dest.tracked || !other_ref) {
        RemoteScope::Named(dest.remote.to_string())
    } else {
        if other_ref {
            revs.exclude_commits.extend(dest.upstream.clone());
        }
        RemoteScope::All
    });
    Ok((revs, None))
}

impl Destination<'_> {
    /// What `s` sends to this remote: its commits minus the remote's commit for the ref (`base`)
    /// and the remote's own tracking refs. What other remotes or the upstream have does not count,
    /// as this remote may not have it.
    pub(super) fn lacks(&self, s: &Sent) -> RevRange {
        RevRange {
            tip: s.revs.tip.clone(),
            exclude_commits: s.base.iter().cloned().collect(),
            exclude_remotes: (self.named && self.tracked)
                .then(|| RemoteScope::Named(self.remote.to_string())),
        }
    }
}
