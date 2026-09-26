//! Path rules while applying: re-deriving trees from the plan and checking the result.

use std::collections::HashSet;

use crate::application::pathrules::TreeRewriter;
use crate::application::ports::Repository;
use crate::domain::error::{Error, Result};
use crate::domain::history::commit::Commit;
use crate::domain::history::plan::{Entry, Plan};
use crate::domain::paths::{GITIGNORE, PathFilter};

/// The filter the plan was made with.
pub(super) fn plan_filter(plan: &Plan) -> Result<Option<PathFilter>> {
    plan.paths
        .as_ref()
        .map(|p| PathFilter::new(&p.exclude))
        .transpose()
}

/// The tree the new commit must have; rederived from the old tree and the plan's rules, so an
/// edited or stale plan cannot smuggle in another tree.
pub(super) fn expected_tree(
    rewriter: Option<&TreeRewriter>,
    old: &Commit,
    e: &Entry,
    index: usize,
) -> Result<String> {
    let Some(rw) = rewriter else {
        return Ok(old.tree.clone());
    };
    let mut tree = rw.without_excluded(&old.tree)?;
    if e.gitignore {
        tree = rw.with_gitignore(&tree)?;
    }
    match &e.tree {
        Some(t) if *t == tree => Ok(tree),
        _ => Err(Error::Usage(format!(
            "plan entry {index}: its tree does not match the path rules for {} (stale or edited plan)",
            e.old_oid
        ))),
    }
}

/// A dropped commit must have changed nothing but excluded paths.
pub(super) fn check_dropped(
    repo: &dyn Repository,
    rewriter: Option<&TreeRewriter>,
    dropped: &[Commit],
) -> Result<()> {
    if dropped.is_empty() {
        return Ok(());
    }
    let Some(rw) = rewriter else {
        return Err(Error::Usage("plan drops commits without path rules".into()));
    };
    let parents: Vec<String> = dropped.iter().flat_map(|c| c.parents.clone()).collect();
    let parent_commits = repo.read_commits(&parents)?;
    let empty = rw.empty_tree()?;
    for c in dropped {
        if c.parents.len() > 1 {
            return Err(Error::Usage(format!(
                "plan drops the merge commit {}; only single-parent commits can be dropped",
                c.oid
            )));
        }
        let parent_tree = match c.parents.first() {
            Some(p) => {
                let pc = parent_commits.iter().find(|x| &x.oid == p).ok_or_else(|| {
                    Error::Precondition(format!(
                        "parent {p} of dropped commit {} is missing",
                        c.oid
                    ))
                })?;
                rw.without_excluded(&pc.tree)?
            }
            None => empty.clone(),
        };
        if rw.without_excluded(&c.tree)? != parent_tree {
            return Err(Error::Usage(format!(
                "plan drops {}, which also changes paths that are not excluded",
                c.oid
            )));
        }
    }
    Ok(())
}

/// Checks the final state against the old tip, independently of how the trees were derived: no
/// excluded path is left, and apart from `.gitignore` nothing else differs from the old tip.
pub(super) fn check_tip_tree(
    repo: &dyn Repository,
    rewriter: &TreeRewriter,
    old_tip_tree: &str,
    new_tip_tree: &str,
) -> Result<bool> {
    if rewriter.has_excluded(new_tip_tree)? {
        return Ok(false);
    }
    let filtered_old = rewriter.without_excluded(old_tip_tree)?;
    let differing: HashSet<String> = repo
        .changed_paths(&filtered_old, new_tip_tree)?
        .into_iter()
        .collect();
    Ok(differing.iter().all(|p| p == GITIGNORE))
}
