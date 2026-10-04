//! The post-commit policy: after every commit, respread the commit times of everything that is
//! not on a remote yet over the configured schedule. Pushed commits are never touched (that would
//! need a force push), and the hook never blocks anything: it only acts in `hook.mode: rewrite`.

use crate::application::planning::{PlanOptions, RangeSpec, build_plan};
use crate::application::ports::{RemoteScope, Repository, RevRange};
use crate::application::rewrite::{ApplyOptions, apply};
use crate::domain::error::{Error, Result};
use crate::domain::settings::{Config, HookMode};

/// Why the hook left the history alone. None of these is an error: they are the ordinary cases of
/// a hook that runs after every commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Skip {
    /// `hook.mode` is `verify`: a commit cannot be blocked, so only `pre-push` judges.
    NotRewriteMode,
    /// Without a schedule there are no times to distribute.
    NoSchedule,
    DetachedHead,
    /// A rebase, merge, cherry-pick or revert is running; its commits must not be moved under it.
    OperationInProgress(&'static str),
    /// Staged changes that a rewrite could not keep apart from the commit.
    IndexDirty,
    /// Neither an upstream nor a remote tells which commits are pushed; everything would count as
    /// unpushed, which is `gcma apply --from root`, not something to do after every commit.
    NoUpstream,
    /// The window has no allowed time left for the commits.
    NoCapacity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PostCommitOutcome {
    Skipped(Skip),
    /// The unpushed commits already had the times the schedule gives them.
    Unchanged,
    Rewritten {
        commits: usize,
        backup_id: Option<String>,
        /// What the plan and the rewrite have to tell: tags left on the old commits, the secrets
        /// note, what went wrong after the branch moved (e.g. the index could not follow).
        notices: Vec<String>,
    },
}

/// Respreads all unpushed commits of the checked-out branch over the schedule. `now` is the current
/// unix time. The range is `upstream..HEAD`, or everything that is on no remote-tracking ref;
/// commits that are already pushed are neither re-timed nor offered for rewriting.
pub fn run_post_commit(repo: &dyn Repository, cfg: &Config, now: i64) -> Result<PostCommitOutcome> {
    if cfg.hook.mode != HookMode::Rewrite {
        return Ok(skipped(Skip::NotRewriteMode));
    }
    if cfg.schedule.is_none() {
        return Ok(skipped(Skip::NoSchedule));
    }
    let Some(branch_ref) = repo.current_branch_ref()? else {
        return Ok(skipped(Skip::DetachedHead));
    };
    if let Some(op) = repo.operation_in_progress()? {
        return Ok(skipped(Skip::OperationInProgress(op)));
    }
    if repo.index_dirty()? {
        return Ok(skipped(Skip::IndexDirty));
    }
    let Some(tip) = repo.resolve_commit(&branch_ref)? else {
        return Ok(skipped(Skip::DetachedHead));
    };
    let upstream = repo.upstream_oid(&branch_ref)?;
    if upstream.is_none() && repo.remotes()?.is_empty() {
        return Ok(skipped(Skip::NoUpstream));
    }
    let base = match &upstream {
        Some(up) => repo.merge_base(&tip, up)?,
        None => None,
    };
    let range = RangeSpec {
        revs: RevRange {
            tip,
            exclude_commits: upstream.into_iter().collect(),
            exclude_remotes: Some(RemoteScope::All),
        },
        branch_ref,
        base,
    };
    // `all` re-times every commit of the range, not only the nonconforming ones; the range itself
    // keeps the pushed commits out, and `apply` refuses to move them again.
    let opts = PlanOptions {
        range: Some(range),
        all: true,
        strict: true,
        ..PlanOptions::new(now)
    };
    match respread(repo, cfg, &opts, now) {
        Err(Error::NoCapacity(_)) => Ok(skipped(Skip::NoCapacity)),
        other => other,
    }
}

fn respread(
    repo: &dyn Repository,
    cfg: &Config,
    opts: &PlanOptions,
    now: i64,
) -> Result<PostCommitOutcome> {
    let built = build_plan(repo, cfg, opts)?;
    if built.plan.is_empty() {
        return Ok(PostCommitOutcome::Unchanged);
    }
    let report = apply(repo, &built.plan, &ApplyOptions::new(now))?;
    if report.noop {
        return Ok(PostCommitOutcome::Unchanged);
    }
    let mut notices = built.warnings;
    notices.extend(report.notices());
    Ok(PostCommitOutcome::Rewritten {
        commits: report.rewritten,
        backup_id: report.backup_id,
        notices,
    })
}

fn skipped(why: Skip) -> PostCommitOutcome {
    PostCommitOutcome::Skipped(why)
}
