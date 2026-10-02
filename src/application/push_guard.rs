//! The pre-push policy: judge (and in `rewrite` mode fix) the commits about to be pushed.

use crate::application::planning::{PlanOptions, RangeSpec, build_plan};
use crate::application::ports::{RemoteScope, Repository};
use crate::application::rewrite::{ApplyOptions, SECRETS_NOTE, apply};
use crate::domain::error::{Error, Result};
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

/// `Ok(())` lets the push proceed; an error aborts it. `now` is the current unix time. What a
/// rewrite has to tell besides (its warnings, the secrets note) is added to `warnings`, whether the
/// push proceeds or not.
pub fn run_pre_push(
    repo: &dyn Repository,
    cfg: &Config,
    remote: &str,
    pushed: &[PushedRef],
    now: i64,
    warnings: &mut Vec<String>,
) -> Result<()> {
    let remotes = repo.remotes()?;
    let Some(branch_ref) = repo.current_branch_ref()? else {
        return Ok(()); // detached HEAD: no push is judged
    };
    // Read once: nothing moves the branch between the pushed refs, as a rewrite ends the loop.
    let tip = repo.ref_value(&branch_ref)?;
    for p in pushed {
        if is_zero_oid(&p.local_sha) {
            continue; // a delete push
        }
        let Some(pushed) = judged_commit(repo, p, &branch_ref, tip.as_deref())? else {
            continue;
        };
        let range = unpushed_range(repo, p, &pushed.commit, remote, &remotes, &branch_ref)?;
        // Rewriting only fixes the push when the pushed commit is the branch tip and the push
        // follows the branch (a tag would stay on the old commit); the plan is built once, with
        // the strict (clean index) preconditions only when we are going to write.
        let rewrite = cfg.hook.mode == HookMode::Rewrite
            && pushed.follows_branch
            && tip.as_deref() == Some(pushed.commit.as_str());
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
            warnings.extend(report.warnings);
            if report.paths_removed {
                warnings.push(SECRETS_NOTE.to_string());
            }
            if !report.noop {
                return Err(Error::Nonconforming(format!(
                    "gcma rewrote {} unpushed commit(s) of {branch_ref} to follow the rules; run `git push` again",
                    report.rewritten
                )));
            }
            continue;
        }
        let n = built.plan.entries.len() + built.plan.dropped.len();
        return blocked(repo, &branch_ref, n);
    }
    Ok(())
}

/// The commit a judged push sends.
struct Judged {
    /// The pushed object peeled to a commit (an annotated tag pushes its tag object).
    commit: String,
    /// The push names the branch itself, `HEAD` or a raw revision, so after a rewrite of the branch
    /// pushing again sends the new commits; a tag or another ref would still send the old ones.
    follows_branch: bool,
}

/// Which pushes are judged, and the commit they send. The checked-out branch is judged by name, as
/// `HEAD` (which is what `git push origin HEAD` and `HEAD:<ref>` report) or as a raw revision that is
/// part of it (`git push origin HEAD~1:main` reports `HEAD~1`). Other branches are ignored, unless
/// they point at the tip. Any other ref (a tag, `refs/<anything>`) is judged when its commit is the
/// tip or an ancestor of it; one that points at no commit (a tree, a blob) or at a commit that is
/// not part of the branch is ignored.
fn judged_commit(
    repo: &dyn Repository,
    p: &PushedRef,
    branch_ref: &str,
    tip: Option<&str>,
) -> Result<Option<Judged>> {
    let Some(commit) = repo.resolve_commit(&p.local_sha)? else {
        return Ok(None);
    };
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
/// (a new branch) everything not on the remote's tracking refs.
fn unpushed_range(
    repo: &dyn Repository,
    p: &PushedRef,
    commit: &str,
    remote: &str,
    remotes: &[String],
    branch_ref: &str,
) -> Result<RangeSpec> {
    let known_remote = match is_zero_oid(&p.remote_sha) {
        true => None,
        false => repo.resolve_commit(&p.remote_sha)?,
    };
    let tip = commit.to_string();
    let branch_ref = branch_ref.to_string();
    if let Some(pushed) = known_remote {
        return Ok(
            RangeSpec::unpushed(tip, branch_ref, None).excluding(pushed.clone(), Some(pushed))
        );
    }
    let scope = match remotes.iter().any(|r| r == remote) {
        true => RemoteScope::Named(remote.to_string()),
        false => RemoteScope::All,
    };
    Ok(RangeSpec::unpushed(tip, branch_ref, Some(scope)))
}

/// The push is refused: always an `Err` (or the failure to read the upstream).
fn blocked(repo: &dyn Repository, branch_ref: &str, n: usize) -> Result<()> {
    let hint = if repo.upstream_oid(branch_ref)?.is_none() {
        " (the branch has no upstream: add `--from <rev>`)"
    } else {
        ""
    };
    Err(Error::Nonconforming(format!(
        "{n} commit(s) about to be pushed do not follow the gcma rules; \
         run `gcma apply`{hint} (it rewrites the unpushed part of the branch) and push again"
    )))
}
