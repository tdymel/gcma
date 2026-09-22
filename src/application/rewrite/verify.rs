//! Verification of a rewrite: plan vs. history before writing, new commits vs. plan before moving refs.

use std::collections::HashMap;

use super::paths::{check_tip_tree, plan_filter};
use crate::application::pathrules::TreeRewriter;
use crate::application::ports::Repository;
use crate::domain::error::{Error, Result};
use crate::domain::history::commit::Commit;
use crate::domain::history::parents::resolve_parents;
use crate::domain::history::plan::{Parent, Plan};
use crate::domain::settings::Signing;

pub(super) fn verify_fail(msg: String) -> Error {
    Error::Internal(format!("verification failed, no ref was changed: {msg}"))
}

/// Cross-checks the plan against the real old commits so an edited or stale plan cannot corrupt
/// history: every parent list must be what dropping the `dropped` commits makes of the old one.
pub(super) fn check_plan_against_history(
    plan: &Plan,
    old: &[Commit],
    dropped: &[Commit],
) -> Result<()> {
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

/// Verification (OID/byte comparisons only). Fails before any ref is touched. `trees[i]` is the
/// tree entry `i` must have.
pub fn verify(
    repo: &dyn Repository,
    plan: &Plan,
    old: &[Commit],
    new_oids: &[String],
    trees: &[String],
) -> Result<()> {
    let new = repo.read_commits(new_oids)?;
    if new.len() != plan.entries.len() || old.len() != plan.entries.len() {
        return Err(verify_fail("commit count mismatch".into()));
    }
    for (i, ((e, o), n)) in plan.entries.iter().zip(old).zip(&new).enumerate() {
        if n.tree != trees[i] {
            return Err(verify_fail(format!(
                "tree of entry {i} ({}) is not the expected one",
                o.oid
            )));
        }
        let expected: Vec<String> = e.parents.iter().map(|p| resolve(p, new_oids)).collect();
        if n.parents != expected {
            return Err(verify_fail(format!(
                "parents of entry {i} ({}) are wrong",
                o.oid
            )));
        }
        if n.author != e.author.to_raw() || n.committer != e.committer.to_raw() {
            return Err(verify_fail(format!(
                "identity/date of entry {i} differs from the plan"
            )));
        }
        if n.message != e.message()? {
            return Err(verify_fail(format!(
                "message of entry {i} differs from the plan"
            )));
        }
        let signed = n.has_signature();
        if (plan.signing == Signing::Strip && signed)
            || (plan.signing == Signing::Resign && !signed)
        {
            return Err(verify_fail(format!(
                "signature state of entry {i} does not match `signing`"
            )));
        }
    }
    let new_tip = plan
        .tip_target()
        .map(|p| resolve(&p, new_oids))
        .ok_or_else(|| verify_fail("the plan does not say where the branch ends up".into()))?;
    let expected_count = repo
        .count_reachable(&plan.tip_oid)?
        .checked_sub(plan.dropped.len());
    if expected_count != Some(repo.count_reachable(&new_tip)?) {
        return Err(verify_fail(
            "the rewritten history does not have exactly the dropped commits fewer".into(),
        ));
    }
    match plan_filter(plan)? {
        None => {
            if !repo.same_tree(&plan.tip_oid, &new_tip)? {
                return Err(verify_fail("the tip tree differs from the original".into()));
            }
        }
        Some(filter) => {
            let rewriter = TreeRewriter::new(repo, &filter);
            let old_tip = repo.read_commits(std::slice::from_ref(&plan.tip_oid))?;
            let new_tip_commit = repo.read_commits(std::slice::from_ref(&new_tip))?;
            if !check_tip_tree(repo, &rewriter, &old_tip[0].tree, &new_tip_commit[0].tree)? {
                return Err(verify_fail(
                    "the tip tree is not the original minus the excluded paths (plus .gitignore)"
                        .into(),
                ));
            }
        }
    }
    // Everything that keeps its OID must still be an ancestor of the new tip.
    let mut bases: Vec<String> = plan
        .entries
        .iter()
        .flat_map(|e| e.parents.iter())
        .filter_map(|p| match p {
            Parent::Base(b) => Some(b.clone()),
            Parent::In(_) => None,
        })
        .collect();
    bases.sort();
    bases.dedup();
    if !repo.all_reachable_from(&bases, &new_tip)? {
        return Err(verify_fail(
            "some unchanged commits are not reachable from the new tip".into(),
        ));
    }
    Ok(())
}

/// The commit id a plan parent stands for once the new commits exist.
pub(super) fn resolve(p: &Parent, new_oids: &[String]) -> String {
    match p {
        Parent::In(j) => new_oids[*j].clone(),
        Parent::Base(b) => b.clone(),
    }
}
