//! Notes follow the rewritten commits on a best-effort basis.

use std::collections::{HashMap, HashSet};

use crate::application::ports::{NoteList, NoteStore};
use crate::domain::history::commit::short;

/// Copies every note of a rewritten commit to its new commit, after the branch moved. Failures are
/// returned as warnings; the old notes stay where they are.
pub fn copy_notes(repo: &dyn NoteStore, renamed: &HashMap<String, String>) -> Vec<String> {
    let list = NoteList::or_unlisted(repo.list_notes());
    let mut warnings = list.warnings();
    for (notes_ref, old) in list.notes {
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

/// How many notes sit on commits of `rewritten`, and warnings about the notes that cannot be
/// listed (those are not counted).
pub fn count_notes(repo: &dyn NoteStore, rewritten: &HashSet<String>) -> (usize, Vec<String>) {
    let list = NoteList::or_unlisted(repo.list_notes());
    let count = list
        .notes
        .iter()
        .filter(|(_, old)| rewritten.contains(old))
        .count();
    (count, list.warnings())
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::domain::error::{Error, Result};

    /// Notes on `a`, `b` and `z` in one notes ref, another notes ref that cannot be read; copying
    /// onto `b2` fails.
    #[derive(Default)]
    struct Notes {
        copied: RefCell<Vec<(String, String)>>,
    }

    impl NoteStore for Notes {
        fn list_notes(&self) -> Result<NoteList> {
            Ok(NoteList {
                notes: vec![
                    ("refs/notes/commits".into(), "a".into()),
                    ("refs/notes/commits".into(), "b".into()),
                    ("refs/notes/commits".into(), "z".into()),
                ],
                skipped: vec![("refs/notes/weird".into(), "not a tree".into())],
            })
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
        assert_eq!(warnings.len(), 2);
        assert!(warnings[0].contains("could not read the notes in refs/notes/weird"));
        assert!(warnings[1].contains("could not copy the note of b"));
        let rewritten = HashSet::from(["a".to_string(), "z".to_string()]);
        let (count, warnings) = count_notes(&repo, &rewritten);
        assert_eq!(count, 2);
        assert_eq!(
            warnings,
            ["could not read the notes in refs/notes/weird, so they are left out (not a tree)"]
        );
    }
}
