//! What a push sends: the pushed refs and, for each, the commits that no remote has yet (judged by
//! the rules) and those that the remote pushed to lacks (checked for replaced commits).

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
    /// The commits published nowhere yet (`unpublished_range`). `tip` is the pushed object peeled
    /// to a commit (an annotated tag pushes its tag object).
    pub revs: RevRange,
    /// The remote's commit for the ref, when we have it: the range starts after it.
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

/// The commits of the push that are published nowhere yet, which the rules judge: those that
/// neither the remote's commit for the ref (when we have it), nor any remote-tracking ref, nor the
/// upstream reaches. History already on some remote is not judged again, so a mirror or a fork can
/// take what is public elsewhere. Also the remote's commit for the ref, if we have it.
pub(super) fn unpublished_range(
    repo: &dyn Repository,
    p: &PushedRef,
    commit: String,
    dest: &Destination,
) -> Result<(RevRange, Option<String>)> {
    let known_remote = match is_zero_oid(&p.remote_sha) {
        true => None,
        false => repo.resolve_commit(&p.remote_sha)?,
    };
    let revs = RevRange {
        tip: commit,
        exclude_commits: known_remote.iter().chain(&dest.upstream).cloned().collect(),
        exclude_remotes: Some(RemoteScope::All),
    };
    Ok((revs, known_remote))
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
