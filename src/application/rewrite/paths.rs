//! Path rules while applying: re-deriving trees from the plan, and syncing the working copy.

use std::collections::HashSet;

use crate::application::pathrules::TreeRewriter;
use crate::application::ports::{Repository, TreeStore};
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

/// After the branch moved from `old_tip` to `new_tip`: make the index follow it, and bring
/// `.gitignore` along when the working copy's file is still the old tip's. Problems are returned as
/// notes; the branch already moved.
pub(super) fn sync_worktree(repo: &dyn Repository, old_tip: &Commit, new_tip: &str) -> Vec<String> {
    let mut notes = Vec::new();
    match sync_gitignore(repo, old_tip, new_tip) {
        Ok(Some(note)) => notes.push(note),
        Ok(None) => {}
        Err(e) => notes.push(format!(
            "could not update the working copy's .gitignore ({e}); compare it with `git show HEAD:.gitignore`"
        )),
    }
    if let Err(e) = repo.reset_index_to_head() {
        notes.push(format!(
            "could not reset the index to the new tip ({e}); run `git reset` yourself"
        ));
    }
    notes
}

fn gitignore_blob(store: &dyn TreeStore, tree: &str) -> Result<Option<Vec<u8>>> {
    let entries = store.read_tree(tree)?;
    entries
        .iter()
        .find(|e| e.name == GITIGNORE.as_bytes() && !e.is_tree())
        .map(|e| store.read_blob(&e.oid))
        .transpose()
}

fn sync_gitignore(
    repo: &dyn Repository,
    old_tip: &Commit,
    new_tip: &str,
) -> Result<Option<String>> {
    let new_commit = repo.read_commits(&[new_tip.to_string()])?;
    let old = gitignore_blob(repo, &old_tip.tree)?;
    let new = gitignore_blob(repo, &new_commit[0].tree)?;
    if old == new {
        return Ok(None);
    }
    if repo.read_file(GITIGNORE)? != old {
        return Ok(Some(
            "the working copy's .gitignore has local changes, so gcma left it alone; make sure it \
             ignores the paths that were removed from history (see `git show HEAD:.gitignore`), or \
             `git add .` would commit them again"
                .into(),
        ));
    }
    match new {
        Some(n) => repo.write_file(GITIGNORE, &n)?,
        None => repo.remove_file(GITIGNORE)?,
    }
    Ok(None)
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
