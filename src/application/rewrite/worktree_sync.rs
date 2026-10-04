//! After the branch moved: make the index and the working copy's `.gitignore` follow it.

use crate::application::ports::{Repository, TreeStore};
use crate::domain::error::Result;
use crate::domain::history::commit::Commit;
use crate::domain::paths::GITIGNORE;

/// Why the branch moved, which tells what a `.gitignore` left alone has to be checked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Move {
    /// `apply` removed paths from history: the working copy's `.gitignore` should ignore them.
    Apply,
    /// `restore` brought back the commits of a backup.
    Restore,
}

/// After the branch moved from `old_tip` to `new_tip`: make the index follow it, and bring
/// `.gitignore` along when the working copy's file is still the old tip's. Problems are returned as
/// notes; the branch already moved.
pub(super) fn sync_worktree(
    repo: &dyn Repository,
    old_tip: &Commit,
    new_tip: &str,
    why: Move,
) -> Vec<String> {
    let mut notes = Vec::new();
    match sync_gitignore(repo, old_tip, new_tip, why) {
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
    why: Move,
) -> Result<Option<String>> {
    let new_commit = repo.read_commits(&[new_tip.to_string()])?;
    let old = gitignore_blob(repo, &old_tip.tree)?;
    let new = gitignore_blob(repo, &new_commit[0].tree)?;
    if old == new {
        return Ok(None);
    }
    if repo.read_file(GITIGNORE)? != old {
        return Ok(Some(left_alone_note(why, new.is_some())));
    }
    match new {
        Some(n) => repo.write_file(GITIGNORE, &n)?,
        None => repo.remove_file(GITIGNORE)?,
    }
    Ok(None)
}

/// What to check in a working copy's `.gitignore` that was left alone because it has local
/// changes; `has_new` tells whether the new tip has a `.gitignore` to compare it with.
fn left_alone_note(why: Move, has_new: bool) -> String {
    let lead = "the working copy's .gitignore has local changes, so gcma left it alone";
    let see = if has_new {
        " (see `git show HEAD:.gitignore`)"
    } else {
        ""
    };
    match why {
        Move::Apply => format!(
            "{lead}; make sure it ignores the paths that were removed from history{see}, or \
             `git add .` would commit them again"
        ),
        Move::Restore if has_new => {
            format!("{lead}; compare it with the restored tip's (`git show HEAD:.gitignore`)")
        }
        Move::Restore => format!("{lead}; the restored tip has no .gitignore"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_gitignore_left_alone_is_pointed_at_what_the_move_needs() {
        let apply = left_alone_note(Move::Apply, true);
        assert!(apply.contains("removed from history"), "{apply}");
        assert!(apply.contains("`git show HEAD:.gitignore`"), "{apply}");
        let apply = left_alone_note(Move::Apply, false);
        assert!(apply.contains("removed from history"), "{apply}");
        assert!(!apply.contains("HEAD:.gitignore"), "{apply}");
        for has_new in [true, false] {
            let restore = left_alone_note(Move::Restore, has_new);
            assert!(!restore.contains("removed from history"), "{restore}");
            assert_eq!(restore.contains("HEAD:.gitignore"), has_new, "{restore}");
        }
    }
}
