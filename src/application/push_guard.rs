//! The pre-push policy: judge (and in `rewrite` mode fix) the commits about to be pushed.

use crate::application::planning::{PlanOptions, RangeSpec, build_plan};
use crate::application::ports::{RemoteScope, Repository};
use crate::application::rewrite::apply;
use crate::domain::error::{Error, Result};
use crate::domain::history::commit::is_zero_oid;
use crate::domain::settings::{Config, HookMode};

/// One ref a push is about to update, as git reports it to the hook.
#[derive(Debug, Clone)]
pub struct PushedRef {
    pub local_ref: String,
    pub local_sha: String,
    pub remote_sha: String,
}

/// `Ok(())` lets the push proceed; an error aborts it. `now` is the current unix time.
pub fn run_pre_push(
    repo: &dyn Repository,
    cfg: &Config,
    remote: &str,
    pushed: &[PushedRef],
    now: i64,
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
        if !is_branch_push(repo, p, &branch_ref, tip.as_deref()) {
            continue;
        }
        let range = unpushed_range(repo, p, remote, &remotes, &branch_ref)?;
        // Rewriting is only possible when the pushed commit is the branch tip; the plan is built
        // once, with the strict (clean index) preconditions only when we are going to write.
        let rewrite =
            cfg.hook.mode == HookMode::Rewrite && tip.as_deref() == Some(p.local_sha.as_str());
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
            let report = apply(repo, &built.plan, false, now)?;
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

/// Only pushes of the checked-out branch are judged: by name, as `HEAD` (which is what
/// `git push origin HEAD` and `HEAD:<ref>` report), or by a ref/sha that is the branch tip. A raw
/// revision (`git push origin HEAD~1:main` reports `HEAD~1`) that is part of the branch is judged
/// as well; named refs other than the branch are not.
fn is_branch_push(
    repo: &dyn Repository,
    p: &PushedRef,
    branch_ref: &str,
    tip: Option<&str>,
) -> bool {
    let ancestor_of_branch = !p.local_ref.starts_with("refs/")
        && tip.is_some_and(|t| repo.is_ancestor(&p.local_sha, t).unwrap_or(false));
    p.local_ref == branch_ref
        || p.local_ref == "HEAD"
        || tip == Some(p.local_sha.as_str())
        || ancestor_of_branch
}

/// The commits the push would send: `remote_sha..local_sha`, or for a remote sha we do not have
/// (a new branch) everything not on the remote's tracking refs.
fn unpushed_range(
    repo: &dyn Repository,
    p: &PushedRef,
    remote: &str,
    remotes: &[String],
    branch_ref: &str,
) -> Result<RangeSpec> {
    let known_remote = !is_zero_oid(&p.remote_sha) && repo.resolve_commit(&p.remote_sha)?.is_some();
    let (exclude_commits, exclude_remotes, base) = if known_remote {
        (vec![p.remote_sha.clone()], None, Some(p.remote_sha.clone()))
    } else if remotes.iter().any(|r| r == remote) {
        (
            Vec::new(),
            Some(RemoteScope::Named(remote.to_string())),
            None,
        )
    } else {
        (Vec::new(), Some(RemoteScope::All), None)
    };
    Ok(RangeSpec {
        tip: p.local_sha.clone(),
        branch_ref: branch_ref.to_string(),
        exclude_commits,
        exclude_remotes,
        base,
    })
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
