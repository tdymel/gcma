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
    for p in pushed {
        let (local_ref, local_sha, remote_sha) = (
            p.local_ref.as_str(),
            p.local_sha.as_str(),
            p.remote_sha.as_str(),
        );
        if is_zero_oid(local_sha) {
            continue; // a delete push
        }
        // Only pushes of the checked-out branch are judged: by name, as `HEAD` (which is what
        // `git push origin HEAD` and `HEAD:<ref>` report), or by a ref/sha that is the branch tip.
        let Some(branch_ref) = repo.current_branch_ref()? else {
            continue; // detached HEAD
        };
        let tip = repo.ref_value(&branch_ref)?;
        // A raw revision (`git push origin HEAD~1:main` reports `HEAD~1`) that is part of the
        // branch is judged as well; named refs other than the branch are not.
        let ancestor_of_branch = !local_ref.starts_with("refs/")
            && tip
                .as_deref()
                .is_some_and(|t| repo.is_ancestor(local_sha, t).unwrap_or(false));
        let is_branch = local_ref == branch_ref
            || local_ref == "HEAD"
            || tip.as_deref() == Some(local_sha)
            || ancestor_of_branch;
        if !is_branch {
            continue;
        }
        let known_remote = !is_zero_oid(remote_sha) && repo.resolve_commit(remote_sha)?.is_some();
        let (exclude_commits, exclude_remotes, base) = if known_remote {
            (
                vec![remote_sha.to_string()],
                None,
                Some(remote_sha.to_string()),
            )
        } else if remotes.iter().any(|r| r == remote) {
            (
                Vec::new(),
                Some(RemoteScope::Named(remote.to_string())),
                None,
            )
        } else {
            (Vec::new(), Some(RemoteScope::All), None)
        };
        let range = RangeSpec {
            tip: local_sha.to_string(),
            branch_ref: branch_ref.clone(),
            exclude_commits,
            exclude_remotes,
            base,
        };
        // Rewriting is only possible when the pushed commit is the branch tip; the plan is built
        // once, with the strict (clean index) preconditions only when we are going to write.
        let rewrite = cfg.hook.mode == HookMode::Rewrite && tip.as_deref() == Some(local_sha);
        let opts = PlanOptions {
            range: Some(range),
            strict: rewrite,
            ..PlanOptions::new(now)
        };
        let built = build_plan(repo, cfg, &opts)?;
        if built.plan.is_empty() {
            continue;
        }
        let n = built.plan.entries.len() + built.plan.dropped.len();
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
        let hint = if repo.upstream_oid(&branch_ref)?.is_none() {
            " (the branch has no upstream: add `--from <rev>`)"
        } else {
            ""
        };
        return Err(Error::Nonconforming(format!(
            "{n} commit(s) about to be pushed do not follow the gcma rules; \
             run `gcma apply`{hint} (it rewrites the unpushed part of the branch) and push again"
        )));
    }
    Ok(())
}
