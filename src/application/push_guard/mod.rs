//! The pre-push policy: judge (and in `rewrite` mode fix) the commits about to be pushed.

mod judge;
mod replaced;
mod sent;

pub use sent::PushedRef;

use crate::application::planning::{PlanOptions, RangeSpec, build_plan};
use crate::application::ports::Repository;
use crate::application::rewrite::{ApplyOptions, apply};
use crate::domain::error::Result;
use crate::domain::history::commit::is_zero_oid;
use crate::domain::settings::{Config, HookMode};
use sent::{Sent, destination, unpushed_range};

/// What the hook decided. Only `Proceed` lets the push go on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrePushOutcome {
    Proceed,
    /// `rewrite` mode fixed the branch; the push is aborted so that pushing again sends the new
    /// commits.
    Rewritten {
        branch_ref: String,
        commits: usize,
        /// What the plan and the rewrite have to tell (tags left behind, the secrets note, ...).
        notices: Vec<String>,
    },
    /// A ref that is not a branch (a tag, `refs/<anything>`, a raw revision) would send commits that
    /// a gcma rewrite replaced: the old identities, times and excluded files.
    Replaced {
        pushed_ref: String,
    },
    /// Nonconforming commits that the hook does not fix.
    Blocked {
        commits: usize,
        /// The branch has no upstream, so `gcma apply` needs `--from`.
        no_upstream: bool,
        /// The push names the branch (or `HEAD`, or a revision of it), so pushing again after
        /// `gcma apply` sends the rewritten commits; a tag needs `gcma apply --retag` to follow.
        follows_branch: bool,
    },
}

/// Judges (and in `rewrite` mode fixes) what the push would send. `now` is the current unix time.
pub fn run_pre_push(
    repo: &dyn Repository,
    cfg: &Config,
    remote: &str,
    pushed: &[PushedRef],
    now: i64,
) -> Result<PrePushOutcome> {
    let branch_ref = repo.current_branch_ref()?;
    let dest = destination(repo, remote, pushed, branch_ref.as_deref())?;
    let mut sent = Vec::new();
    for p in pushed {
        if is_zero_oid(&p.local_sha) {
            continue; // a delete push
        }
        let Some(commit) = repo.resolve_commit(&p.local_sha)? else {
            continue; // a tree or a blob
        };
        let (revs, base) = unpushed_range(repo, p, commit, &dest)?;
        sent.push(Sent {
            pushed: p,
            revs,
            base,
        });
    }
    if let Some(pushed_ref) = replaced::sends_replaced(repo, &sent)? {
        return Ok(PrePushOutcome::Replaced { pushed_ref });
    }
    let Some(branch_ref) = branch_ref else {
        return Ok(PrePushOutcome::Proceed); // detached HEAD: no rules are judged
    };
    // Read once: nothing moves the branch between the pushed refs, as a rewrite ends the loop.
    let tip = repo.ref_value(&branch_ref)?;
    let mut groups = Vec::new();
    for s in &sent {
        let commit = s.revs.tip.clone();
        if let Some(judged) =
            judge::judged_commit(repo, s.pushed, commit, &branch_ref, tip.as_deref())?
        {
            judge::add_to_groups(repo, &mut groups, s, judged)?;
        }
    }
    for g in groups {
        // Rewriting only fixes the push when the pushed commit is the branch tip and the push
        // follows the branch (a tag would stay on the old commit); the plan is built once, with
        // the strict (clean index) preconditions only when we are going to write.
        let rewrite = cfg.hook.mode == HookMode::Rewrite
            && g.follows_branch
            && tip.as_deref() == Some(g.revs.tip.as_str());
        let range = RangeSpec {
            revs: g.revs,
            branch_ref: branch_ref.clone(),
            base: g.base,
        };
        let opts = PlanOptions {
            range: Some(range),
            strict: rewrite,
            ..PlanOptions::new(now)
        };
        let built = build_plan(repo, cfg, &opts)?;
        if built.plan.is_empty() {
            continue;
        }
        if rewrite {
            let report = apply(repo, &built.plan, &ApplyOptions::new(now))?;
            if report.noop {
                continue;
            }
            let mut notices = built.warnings;
            notices.extend(report.notices());
            return Ok(PrePushOutcome::Rewritten {
                branch_ref,
                commits: report.rewritten,
                notices,
            });
        }
        return Ok(PrePushOutcome::Blocked {
            commits: built.plan.entries.len() + built.plan.dropped.len(),
            no_upstream: dest.upstream.is_none(),
            follows_branch: g.follows_branch,
        });
    }
    Ok(PrePushOutcome::Proceed)
}
