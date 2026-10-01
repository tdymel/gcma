//! Notes follow the rewritten commits on a best-effort basis.

use std::collections::{HashMap, HashSet};

use crate::application::ports::NoteStore;
use crate::domain::history::commit::short;

/// Copies every note of a rewritten commit to its new commit, after the branch moved. Failures are
/// returned as warnings; the old notes stay where they are.
pub fn copy_notes(repo: &dyn NoteStore, renamed: &HashMap<String, String>) -> Vec<String> {
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
pub fn count_notes(repo: &dyn NoteStore, rewritten: &HashSet<String>) -> Option<usize> {
    let notes = repo.list_notes().ok()?;
    Some(
        notes
            .iter()
            .filter(|(_, old)| rewritten.contains(old))
            .count(),
    )
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::domain::error::{Error, Result};

    /// Notes on `a` and `b` in one notes ref; copying onto `b2` fails.
    #[derive(Default)]
    struct Notes {
        copied: RefCell<Vec<(String, String)>>,
    }

    impl NoteStore for Notes {
        fn list_notes(&self) -> Result<Vec<(String, String)>> {
            Ok(vec![
                ("refs/notes/commits".into(), "a".into()),
                ("refs/notes/commits".into(), "b".into()),
                ("refs/notes/commits".into(), "z".into()),
            ])
        }
        fn copy_note(&self, _: &str, from: &str, to: &str) -> Result<()> {
            if to == "b2" {
                return Err(Error::Git("has a note".into()));
            }
            self.copied.borrow_mut().push((from.into(), to.into()));
            Ok(())
        }
    }

    #[test]
    fn copies_the_notes_of_renamed_commits_and_warns_about_failures() {
        let renamed = HashMap::from([("a".into(), "a2".into()), ("b".into(), "b2".into())]);
        let repo = Notes::default();
        let warnings = copy_notes(&repo, &renamed);
        assert_eq!(*repo.copied.borrow(), [("a".into(), "a2".into())]);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("could not copy the note of b"));
        let rewritten = HashSet::from(["a".to_string(), "z".to_string()]);
        assert_eq!(count_notes(&repo, &rewritten), Some(2));
    }
}
