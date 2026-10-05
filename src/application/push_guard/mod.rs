//! The pre-push policy: judge (and in `rewrite` mode fix) the commits about to be pushed.

mod judge;
mod replaced;
mod sent;

pub use judge::RefKind;
pub use sent::PushedRef;

use crate::application::planning::{Built, PlanOptions, RangeSpec, build_plan};
use crate::application::ports::Repository;
use crate::application::preconditions::pushed_among;
use crate::application::rewrite::{ApplyOptions, apply};
use crate::domain::error::Result;
use crate::domain::history::commit::is_zero_oid;
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
        /// The commits that break a rule themselves; one that would be rewritten only because its
        /// parent is does not count.
        commits: usize,
        /// Where `gcma apply` has to start to reach the commits it has to rewrite.
        start: Start,
        /// `gcma apply` refuses the rewrite without `--rewrite-pushed`, which a hook cannot pass:
        /// commits it rewrites are on the upstream or on a remote already.
        rewrite_pushed: bool,
        /// How many of the `commits` other remotes have already.
        published: Published,
        /// The remote the push goes to, as git names it (a remote or a URL).
        remote: String,
        /// What the push names: the branch (or `HEAD`, or a revision of it), so pushing again after
        /// `gcma apply` sends the rewritten commits; tags, which need `gcma apply --retag` to
        /// follow; or another ref, which has to be moved by hand.
        kind: RefKind,
    },
}

/// Where `gcma apply` starts, as far as the blocked commits are concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Start {
    /// At the upstream, which leaves out none of the commits: plain `gcma apply` reaches them.
    Upstream,
    /// The branch has no upstream to start from: `gcma apply` needs `--from <rev>`.
    NoUpstream,
    /// The upstream is a local branch that has some of the commits too, so `gcma apply` starting
    /// there would miss them: it needs `--from <rev>`, or the upstream is fixed first.
    LocalUpstream,
}

impl Start {
    /// Whether `gcma apply` needs `--from <rev>` to reach the commits.
    pub fn needs_from(self) -> bool {
        self != Start::Upstream
    }
}

/// How many of the blocked commits other remotes have already.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Published {
    /// None of them: they are new to every remote.
    Nowhere,
    /// Some of them, but not all.
    Partly(usize),
    /// Every one of them. They are still new to the remote pushed to: `--no-verify` would upload
    /// them there unjudged, which is a choice about that remote, not a fix.
    All,
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
            judge::add_to_groups(repo, &mut groups, s)?;
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
        // What another remote has is judged too; whether it may be rewritten is `reach`'s.
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
        let kind = judge::plan_kind(&judged, &built.plan);
        let reach = reach(repo, &dest, &built)?;
        let rewrite = cfg.hook.mode == HookMode::Rewrite
            && kind == RefKind::Branch
            && tip.as_deref() == Some(g.revs.tip.as_str())
            && !reach.rewrite_pushed;
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
            commits: built.nonconforming.len(),
            start: reach.start,
            rewrite_pushed: reach.rewrite_pushed,
            published: reach.published,
            remote: dest.remote.to_string(),
            kind,
        });
    }
    Ok(PrePushOutcome::Proceed)
}

/// What `gcma apply` needs to rewrite the commits of a plan, and who has them already.
struct Reach {
    start: Start,
    rewrite_pushed: bool,
    published: Published,
}

/// What `gcma apply` needs for `built`: it refuses commits that the upstream or a remote has
/// (asked as `apply` asks it, with the same upstream); it starts where the upstream does, which
/// misses commits without one or when a local upstream has them too (what a remote-tracking
/// upstream has is not part of the push's range). Also how many of the nonconforming commits
/// other remotes have.
fn reach(repo: &dyn Repository, dest: &Destination, built: &Built) -> Result<Reach> {
    let touched: Vec<String> = built.plan.touched_oids().cloned().collect();
    let upstream = dest.upstream.as_ref();
    let rewrite_pushed =
        !pushed_among(repo, &touched, upstream.map(|u| u.commit.as_str()))?.is_empty();
    let start = match upstream {
        None => Start::NoUpstream,
        Some(up) if !up.remote && rewrite_pushed && reaches_any(repo, &up.commit, &touched)? => {
            Start::LocalUpstream
        }
        Some(_) => Start::Upstream,
    };
    let own = &built.nonconforming;
    let published = match pushed_among(repo, own, None)?.len() {
        0 => Published::Nowhere,
        n if n == own.len() => Published::All,
        n => Published::Partly(n),
    };
    Ok(Reach {
        start,
        rewrite_pushed,
        published,
    })
}

/// Whether `tip` reaches any of `oids`.
fn reaches_any(repo: &dyn Repository, tip: &str, oids: &[String]) -> Result<bool> {
    for o in oids {
        if repo.is_ancestor(o, tip)? {
            return Ok(true);
        }
    }
    Ok(false)
}
