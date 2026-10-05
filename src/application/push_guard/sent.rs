//! What a push sends: the pushed refs and, for each, the commits that neither the remote pushed to
//! nor the branch's remote-tracking upstream has (judged by the rules) and those that the remote
//! pushed to lacks (checked for replaced commits).

use crate::application::ports::{RemoteScope, Repository, RevRange};
use crate::domain::error::Result;
use crate::domain::history::commit::is_zero_oid;
use crate::domain::history::plan::REMOTES_PREFIX;

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
    /// The commits new to the remote, as the rules see it (`unpublished_range`). `tip` is the pushed object peeled
    /// to a commit (an annotated tag pushes its tag object).
    pub revs: RevRange,
    /// The remote's commit for the ref, when we have it: the range starts after it.
    pub base: Option<String>,
}

/// Where the push goes, as far as the ranges need to know.
pub(super) struct Destination<'a> {
    /// The remote as git names it to the hook: a configured remote or a URL.
    pub remote: &'a str,
    /// `remote` names a configured remote (it is not a URL).
    named: bool,
    /// The upstream of the checked-out branch: the commit `apply` reads as its upstream too.
    pub upstream: Option<Upstream>,
}

/// The upstream of the checked-out branch.
pub(super) struct Upstream {
    pub commit: String,
    /// It is a remote-tracking ref, so what it reaches is published; a local branch (`git checkout
    /// --track main`) is not.
    pub remote: bool,
}

/// Where `remote` (a name or a URL) is, and the upstream of the checked-out branch, if any.
pub(super) fn destination<'a>(
    repo: &dyn Repository,
    remote: &'a str,
    branch_ref: Option<&str>,
) -> Result<Destination<'a>> {
    let named = repo.remotes()?.iter().any(|r| r == remote);
    let upstream_ref = match branch_ref {
        Some(b) => repo.upstream_ref(b)?,
        None => None,
    };
    let mut upstream = None;
    if let Some(name) = upstream_ref {
        upstream = repo.resolve_commit(&name)?.map(|commit| Upstream {
            commit,
            remote: name.starts_with(REMOTES_PREFIX),
        });
    }
    Ok(Destination {
        remote,
        named,
        upstream,
    })
}

/// The new commits of the push, which the rules judge: those that neither the remote's commit for
/// the ref (when we have it), nor the remote's own tracking refs, nor the upstream reaches when it is
/// a remote-tracking ref (a local upstream is unpushed too). What only another remote has is new
/// here: a peer's commits must not reach a public remote unjudged. Also the remote's commit for the
/// ref, if we have it.
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
        exclude_commits: known_remote
            .iter()
            .chain(dest.published_upstream())
            .cloned()
            .collect(),
        exclude_remotes: dest.tracking(),
    };
    Ok((revs, known_remote))
}

impl Destination<'_> {
    /// The commit of the upstream when it is a remote-tracking ref: what it reaches is published.
    fn published_upstream(&self) -> Option<&String> {
        self.upstream
            .as_ref()
            .filter(|u| u.remote)
            .map(|u| &u.commit)
    }

    /// The remote-tracking refs of this remote, when it is a named one (a URL is never a glob).
    /// Without any, `--not --remotes=<name>` excludes nothing.
    fn tracking(&self) -> Option<RemoteScope> {
        self.named
            .then(|| RemoteScope::Named(self.remote.to_string()))
    }

    /// What `s` sends to this remote: its commits minus the remote's commit for the ref (`base`)
    /// and the remote's own tracking refs. What other remotes or the upstream have does not count,
    /// as this remote may not have it.
    pub(super) fn lacks(&self, s: &Sent) -> RevRange {
        RevRange {
            tip: s.revs.tip.clone(),
            exclude_commits: s.base.iter().cloned().collect(),
            exclude_remotes: self.tracking(),
        }
    }
}
