//! Notes follow the rewritten commits on a best-effort basis.

use std::collections::{HashMap, HashSet};

use crate::application::ports::Repository;
use crate::domain::history::commit::short;

/// Copies every note of a rewritten commit to its new commit, after the branch moved. Failures are
/// returned as warnings; the old notes stay where they are.
pub fn copy_notes(repo: &dyn Repository, renamed: &HashMap<String, String>) -> Vec<String> {
    let notes = match repo.list_notes() {
        Ok(n) => n,
        Err(e) => return vec![format!("could not list the notes to copy them ({e})")],
    };
    let mut warnings = Vec::new();
    for (notes_ref, old) in notes {
        let Some(new) = renamed.get(&old) else {
            continue;
        };
        if let Err(e) = repo.copy_note(&notes_ref, &old, new) {
            warnings.push(format!(
                "could not copy the note of {} in {notes_ref} to {} ({e})",
                short(&old),
                short(new)
            ));
        }
    }
    warnings
}

/// How many notes sit on commits of `rewritten`; `None` when the notes cannot be listed.
pub fn count_notes(repo: &dyn Repository, rewritten: &HashSet<String>) -> Option<usize> {
    let notes = repo.list_notes().ok()?;
    Some(
        notes
            .iter()
            .filter(|(_, old)| rewritten.contains(old))
            .count(),
    )
}
