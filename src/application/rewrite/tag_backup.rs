//! What a backup remembers about moved tags.
//!
//! The refs of a backup are `refs/gcma/backup/<branch>/<id>/<kind>` and are listed by splitting
//! from the right, so nothing named after a tag may live below `<id>/`: a tag called `old` or `new`
//! (or one with slashes) would be read as the backup's own refs. The tags are therefore recorded
//! in one blob, `<id>/tags`, a line `<l|a> <old> <new> <ref name>` per tag. The tag objects an
//! annotated tag held before are kept alive by `<id>/tag-<n>` (one per annotated line, in order),
//! because nothing else refers to them once the tag moved. Both kinds of name are ones the
//! listing ignores, and a backup without tags has none of them.

use crate::application::ports::{RefUpdate, Repository};
use crate::application::retag::TagMove;
use crate::domain::error::{Error, Result};
use crate::domain::history::tag::short_name;

pub(super) const MANIFEST_KIND: &str = "tags";
pub(super) const KEEP_KIND_PREFIX: &str = "tag-";

fn encode(moves: &[TagMove]) -> String {
    moves
        .iter()
        .map(|m| {
            let kind = if m.annotated { 'a' } else { 'l' };
            format!("{kind} {} {} {}\n", m.old, m.new, m.name)
        })
        .collect()
}

fn decode(text: &str) -> Result<Vec<TagMove>> {
    text.lines()
        .map(|line| {
            let mut f = line.splitn(4, ' ');
            match (f.next(), f.next(), f.next(), f.next()) {
                (Some(k @ ("a" | "l")), Some(old), Some(new), Some(name)) => Ok(TagMove {
                    name: name.to_string(),
                    old: old.to_string(),
                    new: new.to_string(),
                    annotated: k == "a",
                }),
                _ => Err(Error::Internal(format!(
                    "a backup's tag record is malformed: {line:?}"
                ))),
            }
        })
        .collect()
}

/// The ref transaction steps of `apply` for these tags: the backup refs under `base` and the moves
/// themselves, each a compare-and-swap on the value the tag held when it was read.
pub(super) fn apply_updates(
    repo: &dyn Repository,
    base: &str,
    moves: &[TagMove],
) -> Result<Vec<RefUpdate>> {
    if moves.is_empty() {
        return Ok(Vec::new());
    }
    let manifest = repo.write_blob(encode(moves).as_bytes())?;
    let mut updates = vec![RefUpdate::Create {
        name: format!("{base}/{MANIFEST_KIND}"),
        new: manifest,
    }];
    for (n, m) in moves.iter().filter(|m| m.annotated).enumerate() {
        updates.push(RefUpdate::Create {
            name: format!("{base}/{KEEP_KIND_PREFIX}{n}"),
            new: m.old.clone(),
        });
    }
    updates.extend(moves.iter().map(|m| RefUpdate::Move {
        name: m.name.clone(),
        new: m.new.clone(),
        old: m.old.clone(),
    }));
    Ok(updates)
}

/// The moved tags a backup recorded in its manifest blob.
fn recorded(repo: &dyn Repository, manifest: Option<&str>) -> Result<Vec<TagMove>> {
    match manifest {
        Some(blob) => decode(&String::from_utf8_lossy(&repo.read_blob(blob)?)),
        None => Ok(Vec::new()),
    }
}

/// What `restore` does for the tags of a backup: put each back, but only if it still holds what
/// `apply` set. A tag that moved on is an error, or with `force` left alone and reported.
pub(super) fn restore_updates(
    repo: &dyn Repository,
    manifest: Option<&str>,
    force: bool,
) -> Result<(Vec<RefUpdate>, Vec<String>)> {
    let (mut updates, mut notes) = (Vec::new(), Vec::new());
    for m in recorded(repo, manifest)? {
        let now = repo.ref_value(&m.name)?;
        if now.as_deref() == Some(m.new.as_str()) {
            updates.push(RefUpdate::Move {
                name: m.name,
                new: m.old,
                old: m.new,
            });
        } else if force {
            notes.push(format!(
                "tag {} was changed after the rewrite, so it was left as it is",
                short_name(&m.name)
            ));
        } else {
            return Err(Error::TipMoved(format!(
                "tag {} has moved on since this backup was made (now {}); use --force to restore the branch and leave the tag as it is",
                short_name(&m.name),
                now.as_deref().unwrap_or("deleted")
            )));
        }
    }
    Ok((updates, notes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mv(name: &str, annotated: bool) -> TagMove {
        TagMove {
            name: name.to_string(),
            old: "1".repeat(40),
            new: "2".repeat(40),
            annotated,
        }
    }

    #[test]
    fn records_round_trip_whatever_the_tags_are_called() {
        let moves = [
            mv("refs/tags/old", false),
            mv("refs/tags/new", true),
            mv("refs/tags/rel/v1.0", true),
        ];
        assert_eq!(decode(&encode(&moves)).unwrap(), moves);
        assert!(decode("").unwrap().is_empty());
    }

    #[test]
    fn a_damaged_record_is_an_error() {
        assert!(decode("x a b c\n").is_err());
        assert!(decode("a only-two\n").is_err());
    }
}
