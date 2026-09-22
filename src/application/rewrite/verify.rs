//! Verification of a rewrite: plan vs. history before writing, new commits vs. plan before moving refs.

use std::collections::HashMap;

use crate::application::ports::Repository;
use crate::domain::error::{Error, Result};
use crate::domain::history::commit::Commit;
use crate::domain::history::plan::{Parent, Plan};
use crate::domain::settings::Signing;

pub(super) fn verify_fail(msg: String) -> Error {
    Error::Internal(format!("verification failed, no ref was changed: {msg}"))
}

/// Cross-checks the plan against the real old commits so an edited or stale plan cannot corrupt history.
pub(super) fn check_plan_against_history(plan: &Plan, old: &[Commit]) -> Result<()> {
    for (i, (e, o)) in plan.entries.iter().zip(old).enumerate() {
        if e.parents.len() != o.parents.len() {
            return Err(Error::Usage(format!(
                "plan entry {i}: parent count differs from commit {}",
                e.old_oid
            )));
        }
        for (p, op) in e.parents.iter().zip(&o.parents) {
            let ok = match p {
                Parent::In(j) => plan.entries.get(*j).is_some_and(|x| &x.old_oid == op),
                Parent::Base(b) => b == op,
            };
            if !ok {
                return Err(Error::Usage(format!(
                    "plan entry {i}: parents do not match commit {}",
                    e.old_oid
                )));
            }
        }
    }
    Ok(())
}

/// Verification (OID/byte comparisons only). Fails before any ref is touched.
pub fn verify(
    repo: &dyn Repository,
    plan: &Plan,
    old: &[Commit],
    new_oids: &[String],
) -> Result<()> {
    let new = repo.read_commits(new_oids)?;
    if new.len() != plan.entries.len() || old.len() != plan.entries.len() {
        return Err(verify_fail("commit count mismatch".into()));
    }
    let map: HashMap<&str, &str> = old
        .iter()
        .zip(new_oids)
        .map(|(o, n)| (o.oid.as_str(), n.as_str()))
        .collect();
    for (i, ((e, o), n)) in plan.entries.iter().zip(old).zip(&new).enumerate() {
        if n.tree != o.tree {
            return Err(verify_fail(format!("tree of {} changed", o.oid)));
        }
        let expected: Vec<String> = o
            .parents
            .iter()
            .map(|p| {
                map.get(p.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| p.clone())
            })
            .collect();
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
    let tip_idx = plan
        .entries
        .iter()
        .position(|e| e.old_oid == plan.tip_oid)
        .ok_or_else(|| verify_fail("the old tip is not part of the plan".into()))?;
    let new_tip = &new_oids[tip_idx];
    if repo.count_reachable(&plan.tip_oid)? != repo.count_reachable(new_tip)? {
        return Err(verify_fail(
            "the rewritten history has a different number of commits".into(),
        ));
    }
    if !repo.same_tree(&plan.tip_oid, new_tip)? {
        return Err(verify_fail("the tip tree differs from the original".into()));
    }
    // Everything that keeps its OID must still be an ancestor of the new tip.
    let bases: Vec<String> = plan
        .entries
        .iter()
        .flat_map(|e| e.parents.iter())
        .filter_map(|p| match p {
            Parent::Base(b) => Some(b.clone()),
            Parent::In(_) => None,
        })
        .collect();
    if !repo.all_reachable_from(&bases, new_tip)? {
        return Err(verify_fail(
            "some unchanged commits are not reachable from the new tip".into(),
        ));
    }
    Ok(())
}
