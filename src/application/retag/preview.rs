//! What `plan --retag` shows: the tags a rewrite would move, and those it leaves.

use std::collections::HashSet;

use super::classify::classify;
use super::notes::count_notes;
use crate::application::ports::{NoteStore, TagStore};
use crate::domain::error::Result;
use crate::domain::history::plan::Plan;
use crate::domain::history::tag::short_name;

#[derive(Debug, Default, Clone)]
pub struct Preview {
    /// Short names of the tags that would move.
    pub tags: Vec<String>,
    /// How many notes would be copied to new commits.
    pub notes: usize,
    /// The tags on rewritten or dropped commits that would stay where they are, and the notes
    /// that cannot be listed.
    pub warnings: Vec<String>,
}

/// Every commit of the plan's entries counts as rewritten; a commit that comes out byte-identical
/// is only known once it is written, and then its tags stay where they are.
pub fn preview<R: TagStore + NoteStore>(repo: &R, plan: &Plan) -> Result<Preview> {
    let rewritten: HashSet<String> = plan.entries.iter().map(|e| e.old_oid.clone()).collect();
    let dropped: HashSet<String> = plan.dropped.iter().cloned().collect();
    let classified = classify(repo, &rewritten, &dropped)?;
    let (notes, note_warnings) = count_notes(repo, &rewritten);
    let mut warnings = classified.warnings();
    warnings.extend(note_warnings);
    Ok(Preview {
        tags: classified
            .movable
            .iter()
            .map(|t| short_name(&t.name).to_string())
            .collect(),
        notes,
        warnings,
    })
}
