//! After the branch moved: make the index and the working copy's `.gitignore` follow it.

use crate::application::ports::{Repository, TreeStore};
use crate::domain::error::Result;
use crate::domain::history::commit::Commit;
use crate::domain::paths::GITIGNORE;

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
