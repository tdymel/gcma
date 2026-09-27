//! Verification of a rewrite: the new commits against the plan, before any ref moves.

use super::paths::check_tip_tree;
use super::prepared::Prepared;
use crate::application::pathrules::TreeRewriter;
use crate::application::ports::Repository;
use crate::domain::error::{Error, Result};
use crate::domain::history::commit::Commit;
use crate::domain::history::plan::Plan;
use crate::domain::settings::Signing;

fn verify_fail(msg: String) -> Error {
    Error::Internal(format!("verification failed, no ref was changed: {msg}"))
}

/// Verification (OID/byte comparisons only) of the commits written for `plan`. Fails before any
/// ref is touched.
pub(super) fn verify(
    repo: &dyn Repository,
    plan: &Plan,
    prepared: &Prepared,
    new_oids: &[String],
) -> Result<()> {
    let new = repo.read_commits(new_oids)?;
    verify_entries(plan, prepared, &new, new_oids)?;
    let new_tip = prepared.tip.resolve(new_oids);
    let expected_count = repo
        .count_reachable(&plan.tip_oid)?
        .checked_sub(plan.dropped.len());
    if expected_count != Some(repo.count_reachable(&new_tip)?) {
        return Err(verify_fail(
            "the rewritten history does not have exactly the dropped commits fewer".into(),
        ));
    }
    verify_tip_tree(repo, plan, prepared, &new_tip)?;
    verify_unchanged_reachable(repo, plan, &new_tip)
}

/// Each new commit has the tree, parents, identities, message and signature state of its entry.
fn verify_entries(
    plan: &Plan,
    prepared: &Prepared,
    new: &[Commit],
    new_oids: &[String],
) -> Result<()> {
    let (old, trees) = (&prepared.old, &prepared.trees);
    if new.len() != plan.entries.len() || old.len() != plan.entries.len() {
        return Err(verify_fail("commit count mismatch".into()));
    }
    for (i, ((e, o), n)) in plan.entries.iter().zip(old).zip(new).enumerate() {
        if n.tree != trees[i] {
            return Err(verify_fail(format!(
                "tree of entry {i} ({}) is not the expected one",
                o.oid
            )));
        }
        let expected: Vec<String> = e.parents.iter().map(|p| p.resolve(new_oids)).collect();
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
        if n.message != e.message {
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
    Ok(())
}

/// The new tip has the old tip's tree, minus the excluded paths (plus `.gitignore`) with path rules.
fn verify_tip_tree(
    repo: &dyn Repository,
    plan: &Plan,
    prepared: &Prepared,
    new_tip: &str,
) -> Result<()> {
    match &prepared.filter {
        None => {
            if !repo.same_tree(&plan.tip_oid, new_tip)? {
                return Err(verify_fail("the tip tree differs from the original".into()));
            }
        }
        Some(filter) => {
            let rewriter = TreeRewriter::new(repo, filter);
            let new_tip_commit = repo.read_commits(&[new_tip.to_string()])?;
            if !check_tip_tree(
                repo,
                &rewriter,
                &prepared.old_tip.tree,
                &new_tip_commit[0].tree,
            )? {
                return Err(verify_fail(
                    "the tip tree is not the original minus the excluded paths (plus .gitignore)"
                        .into(),
                ));
            }
        }
    }
    Ok(())
}

/// Everything that keeps its OID must still be an ancestor of the new tip.
fn verify_unchanged_reachable(repo: &dyn Repository, plan: &Plan, new_tip: &str) -> Result<()> {
    let mut bases: Vec<String> = plan.base_oids().cloned().collect();
    bases.sort();
    bases.dedup();
    if !repo.all_reachable_from(&bases, new_tip)? {
        return Err(verify_fail(
            "some unchanged commits are not reachable from the new tip".into(),
        ));
    }
    Ok(())
}
