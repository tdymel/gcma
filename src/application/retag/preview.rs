//! What `plan --retag` shows: the tags a rewrite would move, and those it leaves.

use std::collections::HashSet;

use super::classify::{classify, short_name};
use super::notes::count_notes;
use crate::application::ports::Repository;
use crate::domain::error::Result;
use crate::domain::history::plan::Plan;

#[derive(Debug, Default, Clone)]
pub struct Preview {
    /// Short names of the tags that would move.
    pub tags: Vec<String>,
    /// How many notes would be copied to new commits.
    pub notes: usize,
    /// The tags on rewritten or dropped commits that would stay where they are.
    pub warnings: Vec<String>,
}

/// Every commit of the plan's entries counts as rewritten; a commit that comes out byte-identical
/// is only known once it is written, and then its tags stay where they are.
pub fn preview(repo: &dyn Repository, plan: &Plan) -> Result<Preview> {
    let rewritten: HashSet<String> = plan.entries.iter().map(|e| e.old_oid.clone()).collect();
    let dropped: HashSet<String> = plan.dropped.iter().cloned().collect();
    let classified = classify(repo, &rewritten, &dropped)?;
    Ok(Preview {
        tags: classified
            .movable
            .iter()
            .map(|t| short_name(&t.name).to_string())
            .collect(),
        notes: count_notes(repo, &rewritten).unwrap_or(0),
        warnings: classified.warnings(),
    })
}
