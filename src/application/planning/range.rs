//! Resolving which commits a plan covers.

use super::types::{PlanOptions, RangeSpec};
use crate::application::ports::{RemoteScope, Repository, RevRange};
use crate::domain::error::{Error, Result};

pub(super) struct RangeInfo {
    pub branch_ref: String,
    pub tip: String,
    /// The commit the range starts after, if a single one.
    pub base: Option<String>,
    pub upstream: Option<String>,
    /// The commits of the range, parents first.
    pub order: Vec<String>,
}

impl RangeSpec {
    /// The commits of `branch_ref` up to `tip` that are on none of the remote-tracking refs of
    /// `remotes`; `excluding` narrows it further.
    pub fn unpushed(tip: String, branch_ref: String, remotes: Option<RemoteScope>) -> RangeSpec {
        RangeSpec {
            revs: RevRange {
                tip,
                exclude_commits: Vec::new(),
                exclude_remotes: remotes,
            },
            branch_ref,
            base: None,
        }
    }

    /// Leaves out what is reachable from `pushed`, a commit the remote is known to have; the range
    /// then starts after `base`.
    pub fn excluding(mut self, pushed: String, base: Option<String>) -> RangeSpec {
        self.revs.exclude_commits.push(pushed);
        self.base = base;
        self
    }
}

pub(super) fn resolve(repo: &dyn Repository, opts: &PlanOptions) -> Result<RangeInfo> {
    let (branch_ref, base, upstream, revs) = if let Some(r) = &opts.range {
        (r.branch_ref.clone(), r.base.clone(), None, r.revs.clone())
    } else {
        let branch_ref = repo
            .current_branch_ref()?
            .ok_or_else(|| Error::Precondition("detached HEAD; check out a branch first".into()))?;
        let tip = repo
            .resolve_commit(&branch_ref)?
            .ok_or_else(|| Error::Usage("the current branch has no commits".into()))?;
        let upstream = repo.upstream_oid(&branch_ref)?;
        let base = match (&opts.from_rev, &upstream) {
            (Some(f), _) if f == "root" => None,
            (Some(f), _) => {
                let b = repo
                    .resolve_commit(f)?
                    .ok_or_else(|| Error::Usage(format!("--from: cannot resolve {f:?}")))?;
                if !repo.is_ancestor(&b, &tip)? {
                    return Err(Error::Usage(format!(
                        "--from {f} is not an ancestor of the branch tip"
                    )));
                }
                Some(b)
            }
            (None, Some(up)) => repo.merge_base(&tip, up)?,
            (None, None) => {
                return Err(Error::Usage(
                    "the branch has no upstream: pass --from <rev> (exclusive) or --from root"
                        .into(),
                ));
            }
        };
        let revs = RevRange {
            tip,
            exclude_commits: base.iter().cloned().collect(),
            exclude_remotes: None,
        };
        (branch_ref, base, upstream, revs)
    };
    let order = repo.list_range(&revs)?;
    Ok(RangeInfo {
        branch_ref,
        tip: revs.tip,
        base,
        upstream,
        order,
    })
}
