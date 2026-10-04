//! The pre-push policy: judge (and in `rewrite` mode fix) the commits about to be pushed.

mod judge;
mod replaced;
mod sent;

pub use judge::RefKind;
pub use sent::PushedRef;

use crate::application::planning::{PlanOptions, RangeSpec, build_plan};
use crate::application::ports::Repository;
use crate::application::rewrite::{ApplyOptions, apply};
use crate::domain::error::Result;
use crate::domain::history::commit::is_zero_oid;
use crate::domain::history::plan::Plan;
use crate::domain::settings::{Config, HookMode};
use sent::{Destination, Sent, destination, unpublished_range};

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
        /// Why `gcma apply`, which starts at the upstream, needs `--from` to reach the commits
        /// (and, for published ones, `--rewrite-pushed`).
        needs_from: Option<NeedsFrom>,
        /// What the push names: the branch (or `HEAD`, or a revision of it), so pushing again after
        /// `gcma apply` sends the rewritten commits; tags, which need `gcma apply --retag` to
        /// follow; or another ref, which has to be moved by hand.
        kind: RefKind,
    },
}

/// Why `gcma apply` does not reach the blocked commits without `--from`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NeedsFrom {
    /// The branch has no upstream to start from.
    NoUpstream,
    /// The upstream is a local branch that has (some of) the commits too.
    LocalUpstream,
    /// Another remote has (some of) the commits already: rewriting them takes `--rewrite-pushed`
    /// too.
    Published,
}

impl NeedsFrom {
    /// Whether `gcma apply` refuses the commits without `--rewrite-pushed`, which a hook cannot
    /// pass: they are on the upstream or on a remote already.
    fn needs_rewrite_pushed(self) -> bool {
        matches!(self, NeedsFrom::LocalUpstream | NeedsFrom::Published)
    }
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
    let dest = destination(repo, remote, branch_ref.as_deref())?;
    let mut sent = Vec::new();
    for p in pushed {
        if is_zero_oid(&p.local_sha) {
            continue; // a delete push
        }
        let Some(commit) = repo.resolve_commit(&p.local_sha)? else {
            continue; // a tree or a blob
        };
        let (revs, base) = unpublished_range(repo, p, commit, &dest)?;
        sent.push(Sent {
            pushed: p,
            revs,
            base,
        });
    }
    if let Some(pushed_ref) = replaced::sends_replaced(repo, &sent, &dest)? {
        return Ok(PrePushOutcome::Replaced { pushed_ref });
    }
    let Some(branch_ref) = branch_ref else {
        return Ok(PrePushOutcome::Proceed); // detached HEAD: no rules are judged
    };
    // Read once: nothing moves the branch between the pushed refs, as a rewrite ends the loop.
    let tip = repo.ref_value(&branch_ref)?;
    let mut groups = Vec::new();
    let mut judged = Vec::new();
    for s in &sent {
        if let Some(kind) = judge::kind(repo, s, &branch_ref, tip.as_deref())? {
            judge::add_to_groups(repo, &mut groups, s, kind.clone())?;
            judged.push((s.revs.tip.clone(), kind));
        }
    }
    for g in groups {
        // The plan is built without the strict (clean index) preconditions: `apply` checks them
        // when there is something to write.
        let range = RangeSpec {
            revs: g.revs.clone(),
            branch_ref: branch_ref.clone(),
            base: g.base,
        };
        // What another remote has is judged too; whether it may be rewritten is `needs_from`.
        let opts = PlanOptions {
            range: Some(range),
            rewrite_pushed: true,
            ..PlanOptions::new(now)
        };
        let built = build_plan(repo, cfg, &opts)?;
        if built.plan.is_empty() {
            continue;
        }
        // Rewriting only fixes the push when the pushed commit is the branch tip and every pushed
        // ref on the rewritten commits follows the branch (a tag or another ref would stay on the
        // old commit). Commits a local upstream or another remote has too are left to `gcma
        // apply`, which refuses to rewrite them without a flag a hook cannot pass.
        let kind = judge::plan_kind(g.kind, &judged, &built.plan);
        let needs_from = needs_from(repo, &dest, &built.plan)?;
        let rewrite = cfg.hook.mode == HookMode::Rewrite
            && kind == RefKind::Branch
            && tip.as_deref() == Some(g.revs.tip.as_str())
            && !needs_from.is_some_and(NeedsFrom::needs_rewrite_pushed);
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
            needs_from,
            kind,
        });
    }
    Ok(PrePushOutcome::Proceed)
}

/// Whether `gcma apply` would miss commits of `plan`: it starts where the upstream does, so it
/// needs `--from` without one, or when a local upstream has them too; and it refuses commits that
/// a remote has already (what only another remote has is judged on its way to this one).
fn needs_from(repo: &dyn Repository, dest: &Destination, plan: &Plan) -> Result<Option<NeedsFrom>> {
    let planned: Vec<String> = plan.touched_oids().cloned().collect();
    let has_some = |upstream: Option<&str>| -> Result<bool> {
        let unpushed = repo.unpushed_among(&planned, upstream)?;
        Ok(planned.iter().any(|c| !unpushed.contains(c)))
    };
    if has_some(None)? {
        return Ok(Some(NeedsFrom::Published));
    }
    Ok(match &dest.upstream {
        None => Some(NeedsFrom::NoUpstream),
        Some(up) if up.remote => None,
        Some(up) => has_some(Some(&up.commit))?.then_some(NeedsFrom::LocalUpstream),
    })
}
