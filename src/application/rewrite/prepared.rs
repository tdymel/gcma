//! Checking a plan against the real history before anything is written.

use std::collections::HashMap;

use super::paths::{check_dropped, expected_tree, plan_filter};
use crate::application::pathrules::TreeRewriter;
use crate::application::ports::Repository;
use crate::domain::error::{Error, Result};
use crate::domain::history::commit::Commit;
use crate::domain::history::parents::resolve_parents;
use crate::domain::history::plan::{Parent, Plan};
use crate::domain::paths::PathFilter;

/// What the plan was checked against: the old commits, the tree each new commit must have, and
/// where the branch ends up.
pub(super) struct Prepared {
    pub old: Vec<Commit>,
    pub trees: Vec<String>,
    /// The old tip, as the plan was made against it.
    pub old_tip: Commit,
    /// The path rules the plan carries.
    pub filter: Option<PathFilter>,
    /// The new tip: the entry of the old tip, or what the plan says when that commit is dropped.
    pub tip: Parent,
}

/// Cross-checks the plan against the real history so an edited or stale plan cannot corrupt it.
pub(super) fn prepare(repo: &dyn Repository, plan: &Plan) -> Result<Prepared> {
    let tip = plan.tip_target().ok_or_else(|| {
        Error::Usage("the plan neither rewrites the tip nor says where the branch ends up".into())
    })?;
    let old_oids: Vec<String> = plan.entries.iter().map(|e| e.old_oid.clone()).collect();
    let old = repo.read_commits(&old_oids)?;
    let dropped = repo.read_commits(&plan.dropped)?;
    check_plan_against_history(plan, &old, &dropped)?;
    let filter = plan_filter(plan)?;
    let rewriter = filter.as_ref().map(|f| TreeRewriter::new(repo, f));
    check_dropped(repo, rewriter.as_ref(), &dropped)?;
    let trees = plan
        .entries
        .iter()
        .zip(&old)
        .enumerate()
        .map(|(i, (e, o))| expected_tree(rewriter.as_ref(), o, e, i))
        .collect::<Result<Vec<String>>>()?;
    let old_tip = repo
        .read_commits(std::slice::from_ref(&plan.tip_oid))?
        .into_iter()
        .next()
        .ok_or_else(|| Error::Internal(format!("the old tip {} is missing", plan.tip_oid)))?;
    Ok(Prepared {
        old,
        trees,
        old_tip,
        filter,
        tip,
    })
}

/// Every parent list must be what dropping the `dropped` commits makes of the old one.
fn check_plan_against_history(plan: &Plan, old: &[Commit], dropped: &[Commit]) -> Result<()> {
    if dropped.len() != plan.dropped.len() {
        return Err(Error::Usage("plan lists dropped commits twice".into()));
    }
    let rewritten: HashMap<&str, usize> = plan
        .entries
        .iter()
        .enumerate()
        .map(|(i, e)| (e.old_oid.as_str(), i))
        .collect();
    let gone: HashMap<&str, &[String]> = dropped
        .iter()
        .map(|c| (c.oid.as_str(), c.parents.as_slice()))
        .collect();
    for (i, (e, o)) in plan.entries.iter().zip(old).enumerate() {
        if e.parents != resolve_parents(&o.parents, &rewritten, &gone) {
            return Err(Error::Usage(format!(
                "plan entry {i}: parents do not match commit {}",
                e.old_oid
            )));
        }
    }
    if let Some(Parent::Base(b)) = &plan.new_tip
        && gone.contains_key(b.as_str())
    {
        return Err(Error::Usage("plan new_tip is a dropped commit".into()));
    }
    Ok(())
}
