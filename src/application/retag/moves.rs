//! Preparing the tag moves of a rewrite: new tag objects, checked before any ref moves.

use std::collections::{HashMap, HashSet};

use super::classify::{classify, short_name};
use crate::application::ports::{CommitStore, RefStore, TagKind, TagRef, TagStore};
use crate::domain::error::{Error, Result};
use crate::domain::history::tag::retarget;

/// One tag ref that moves to the rewritten commit's counterpart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagMove {
    /// The full ref name (`refs/tags/v1`).
    pub name: String,
    /// What the ref holds now: a commit, or a tag object.
    pub old: String,
    /// What it holds afterwards: the new commit, or a new tag object that points at it.
    pub new: String,
    /// Whether `old` and `new` are tag objects.
    pub annotated: bool,
}

/// What `--retag` does for a rewrite, once the new commits exist.
#[derive(Debug)]
pub struct Moves {
    pub moves: Vec<TagMove>,
    /// The tags on rewritten or dropped commits that stay where they are.
    pub warnings: Vec<String>,
}

/// The tag moves for a rewrite that turned the commits `renamed` (old id to new id; only commits
/// whose id changed) into new ones and dropped `dropped`. Annotated tags get a fresh tag object, an
/// unreferenced object until the ref transaction; each move is verified here.
pub fn prepare<R: TagStore + RefStore + CommitStore + ?Sized>(
    repo: &R,
    renamed: &HashMap<String, String>,
    dropped: &HashSet<String>,
) -> Result<Moves> {
    let rewritten: HashSet<String> = renamed.keys().cloned().collect();
    let classified = classify(repo, &rewritten, dropped)?;
    let moves = classified
        .movable
        .iter()
        .map(|tag| make_move(repo, tag, renamed))
        .collect::<Result<Vec<_>>>()?;
    Ok(Moves {
        moves,
        warnings: classified.warnings(),
    })
}

fn make_move<R: TagStore + RefStore + CommitStore + ?Sized>(
    repo: &R,
    tag: &TagRef,
    renamed: &HashMap<String, String>,
) -> Result<TagMove> {
    let old_commit = tag
        .peeled
        .as_ref()
        .ok_or_else(|| Error::Internal(format!("{} does not point at a commit", tag.name)))?;
    let new_commit = &renamed[old_commit];
    let fail = |what: &str| {
        Error::Internal(format!(
            "verification failed, no ref was changed: tag {} {what}",
            short_name(&tag.name)
        ))
    };
    match tag.kind {
        TagKind::Lightweight => {
            if !repo.objects_exist(std::slice::from_ref(new_commit))? {
                return Err(fail("would point at a commit that was not written"));
            }
            Ok(TagMove {
                name: tag.name.clone(),
                old: tag.value.clone(),
                new: new_commit.clone(),
                annotated: false,
            })
        }
        TagKind::Nested => Err(Error::Internal(format!(
            "{} is a tag of a tag and cannot be moved",
            tag.name
        ))),
        TagKind::Annotated => {
            let raw = repo.read_tag_object(&tag.value)?;
            let copy = retarget(&raw, old_commit, new_commit)?;
            let new = repo.write_tag_object(&copy)?;
            if repo.read_tag_object(&new)? != copy {
                return Err(fail("was not written as expected"));
            }
            if repo.resolve_commit(&new)?.as_ref() != Some(new_commit) {
                return Err(fail("does not point at the expected commit after the move"));
            }
            Ok(TagMove {
                name: tag.name.clone(),
                old: tag.value.clone(),
                new,
                annotated: true,
            })
        }
    }
}
