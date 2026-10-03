//! The pre-push policy: judge (and in `rewrite` mode fix) the commits about to be pushed.

mod replaced;

use crate::application::planning::{PlanOptions, RangeSpec, build_plan};
use crate::application::ports::{RemoteScope, Repository, RevRange};
use crate::application::rewrite::{ApplyOptions, apply, list_backups};
use crate::domain::error::Result;
use crate::domain::history::commit::is_zero_oid;
use crate::domain::history::plan::HEADS_PREFIX;
use crate::domain::settings::{Config, HookMode};

/// One ref a push is about to update, as git reports it to the hook.
#[derive(Debug, Clone)]
pub struct PushedRef {
    pub local_ref: String,
    pub local_sha: String,
    pub remote_sha: String,
}

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
    let remotes = repo.remotes()?;
    let Some(branch_ref) = repo.current_branch_ref()? else {
        return Ok(PrePushOutcome::Proceed); // detached HEAD: no push is judged
    };
    // Read once: nothing moves the branch between the pushed refs, as a rewrite ends the loop.
    let tip = repo.ref_value(&branch_ref)?;
    let mut sent = Vec::new();
    for p in pushed {
        if is_zero_oid(&p.local_sha) {
            continue; // a delete push
        }
        let Some(commit) = repo.resolve_commit(&p.local_sha)? else {
            continue; // a tree or a blob
        };
        let (revs, base) = unpushed_range(repo, p, commit, remote, &remotes)?;
        sent.push(Sent {
            pushed: p,
            revs,
            base,
        });
    }
    if let Some(pushed_ref) = sends_replaced(repo, &sent)? {
        return Ok(PrePushOutcome::Replaced { pushed_ref });
    }
    for s in sent {
        let commit = s.revs.tip.clone();
        let Some(judged) = judged_commit(repo, s.pushed, commit, &branch_ref, tip.as_deref())?
        else {
            continue;
        };
        // Rewriting only fixes the push when the pushed commit is the branch tip and the push
        // follows the branch (a tag would stay on the old commit); the plan is built once, with
        // the strict (clean index) preconditions only when we are going to write.
        let rewrite = cfg.hook.mode == HookMode::Rewrite
            && judged.follows_branch
            && tip.as_deref() == Some(judged.commit.as_str());
        let range = RangeSpec {
            revs: s.revs,
            branch_ref: branch_ref.clone(),
            base: s.base,
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
            no_upstream: repo.upstream_oid(&branch_ref)?.is_none(),
        });
    }
    Ok(PrePushOutcome::Proceed)
}

/// A pushed ref and the commits it would send.
struct Sent<'a> {
    pushed: &'a PushedRef,
    /// `tip` is the pushed object peeled to a commit (an annotated tag pushes its tag object).
    revs: RevRange,
    /// The commit the range starts after, when it is a single one.
    base: Option<String>,
}

/// The commit a judged push sends.
struct Judged {
    /// The pushed object peeled to a commit (an annotated tag pushes its tag object).
    commit: String,
    /// The push names the branch itself, `HEAD` or a raw revision, so after a rewrite of the branch
    /// pushing again sends the new commits; a tag or another ref would still send the old ones.
    follows_branch: bool,
}

/// The first ref outside the branches that would send a commit gcma replaced (see `replaced`): the
/// old identities, times and excluded files. The replaced commits are worked out once, for every
/// pushed ref; a branch push never sends one, as a local branch holds what it reaches.
fn sends_replaced(repo: &dyn Repository, sent: &[Sent]) -> Result<Option<String>> {
    let checked: Vec<&Sent> = sent
        .iter()
        .filter(|s| !s.pushed.local_ref.starts_with(HEADS_PREFIX))
        .collect();
    if checked.is_empty() {
        return Ok(None);
    }
    let backups: Vec<(String, String)> = list_backups(repo)?
        .into_iter()
        .map(|b| (b.old, b.new))
        .collect();
    if backups.is_empty() {
        return Ok(None);
    }
    let mut tips: Vec<String> = backups
        .iter()
        .flat_map(|(old, new)| [old.clone(), new.clone()])
        .chain(checked.iter().map(|s| s.revs.tip.clone()))
        .collect();
    tips.sort();
    tips.dedup();
    let graph = repo.commits_off_branches(&tips)?;
    let gone = replaced::replaced(&graph, &backups);
    for s in checked {
        let reached = replaced::reach(&graph, &s.revs.tip);
        if !reached.iter().any(|c| gone.contains(*c)) {
            continue;
        }
        // A replaced commit the remote already has is not sent again.
        if repo.list_range(&s.revs)?.iter().any(|c| gone.contains(c)) {
            return Ok(Some(s.pushed.local_ref.clone()));
        }
    }
    Ok(None)
}

/// Which pushes are judged, and the commit they send. The checked-out branch is judged by name, as
/// `HEAD` (which is what `git push origin HEAD` and `HEAD:<ref>` report) or as a raw revision that is
/// part of it (`git push origin HEAD~1:main` reports `HEAD~1`). Other branches are ignored, unless
/// they point at the tip. Any other ref (a tag, `refs/<anything>`) is judged when its commit is the
/// tip or an ancestor of it; one that points at a commit that is not part of the branch is not
/// judged (but see `sends_replaced`). `commit` is the pushed object peeled to a commit.
fn judged_commit(
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

/// The commits the push would send: `remote_sha..commit`, or for a remote sha we do not have
/// (a new branch) everything not on the remote's tracking refs; and the commit the range starts
/// after, when it is a single one.
fn unpushed_range(
    repo: &dyn Repository,
    p: &PushedRef,
    commit: String,
    remote: &str,
    remotes: &[String],
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
    revs.exclude_remotes = Some(match remotes.iter().any(|r| r == remote) {
        true => RemoteScope::Named(remote.to_string()),
        false => RemoteScope::All,
    });
    Ok((revs, None))
}
