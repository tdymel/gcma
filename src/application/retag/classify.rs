//! Which tags follow a rewrite and which are left behind, and why.

use std::collections::HashSet;

use crate::application::ports::{TagKind, TagRef, TagStore};
use crate::domain::error::Result;
use crate::domain::history::tag::{is_signed, short_name};

/// The tags that point at rewritten or dropped commits, by what happens to them.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Classified {
    /// Lightweight tags and unsigned annotated tags on a rewritten commit: they follow it.
    pub movable: Vec<TagRef>,
    /// Signed annotated tags on a rewritten commit: a copy would lose the signature.
    pub signed: Vec<TagRef>,
    /// Tags of tags on a rewritten commit.
    pub nested: Vec<TagRef>,
    /// Tags on a rewritten commit whose name is not valid UTF-8.
    pub not_utf8: Vec<TagRef>,
    /// Tags on a commit the rewrite drops.
    pub dropped: Vec<TagRef>,
}

fn names(tags: &[TagRef]) -> String {
    tags.iter()
        .map(|t| short_name(&t.name))
        .collect::<Vec<_>>()
        .join(", ")
}

impl Classified {
    /// What `--retag` leaves alone, as warnings.
    pub fn warnings(&self) -> Vec<String> {
        let mut out = Vec::new();
        if !self.signed.is_empty() {
            out.push(format!(
                "signed tag(s) point at rewritten commits and are not moved, because a copy would lose the signature: {}",
                names(&self.signed)
            ));
        }
        if !self.nested.is_empty() {
            out.push(format!(
                "tag(s) of tags point at rewritten commits and are not moved: {}",
                names(&self.nested)
            ));
        }
        if !self.not_utf8.is_empty() {
            out.push(format!(
                "tag(s) point at rewritten commits and are not moved, because the name is not valid UTF-8: {}",
                names(&self.not_utf8)
            ));
        }
        if !self.dropped.is_empty() {
            out.push(format!(
                "tag(s) point at commits that are dropped and are not moved: {}",
                names(&self.dropped)
            ));
        }
        out
    }
}

/// Sorts the tags that point at a `rewritten` or `dropped` commit; the others are not listed.
/// `is_signed` is only asked about annotated tags on rewritten commits.
pub fn partition(
    tags: Vec<TagRef>,
    rewritten: &HashSet<String>,
    dropped: &HashSet<String>,
    mut is_signed: impl FnMut(&TagRef) -> Result<bool>,
) -> Result<Classified> {
    let mut out = Classified::default();
    for tag in tags {
        let Some(target) = &tag.peeled else { continue };
        if dropped.contains(target) {
            out.dropped.push(tag);
        } else if !rewritten.contains(target) {
            continue;
        } else if !tag.name_is_utf8 {
            out.not_utf8.push(tag);
        } else if tag.kind == TagKind::Nested {
            out.nested.push(tag);
        } else if tag.kind == TagKind::Annotated && is_signed(&tag)? {
            out.signed.push(tag);
        } else {
            out.movable.push(tag);
        }
    }
    Ok(out)
}

/// Reads the repository's tags and sorts those that point at the given commits.
pub fn classify<R: TagStore + ?Sized>(
    repo: &R,
    rewritten: &HashSet<String>,
    dropped: &HashSet<String>,
) -> Result<Classified> {
    partition(repo.list_tags()?, rewritten, dropped, |tag| {
        Ok(is_signed(&repo.read_tag_object(&tag.value)?))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::history::tag::TAGS_PREFIX;

    fn tag(name: &str, kind: TagKind, peeled: Option<&str>) -> TagRef {
        TagRef {
            name: format!("{TAGS_PREFIX}{name}"),
            name_is_utf8: !name.contains('\u{fffd}'),
            value: format!("value-of-{name}"),
            kind,
            peeled: peeled.map(String::from),
        }
    }

    fn set(items: &[&str]) -> HashSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn sorts_tags_by_what_happens_to_them() {
        let tags = vec![
            tag("light", TagKind::Lightweight, Some("a")),
            tag("note", TagKind::Annotated, Some("a")),
            tag("signed", TagKind::Annotated, Some("b")),
            tag("nested", TagKind::Nested, Some("b")),
            tag("caf\u{fffd}", TagKind::Lightweight, Some("a")),
            tag("gone", TagKind::Lightweight, Some("d")),
            tag("elsewhere", TagKind::Lightweight, Some("z")),
            tag("tree", TagKind::Lightweight, None),
        ];
        let got = partition(tags, &set(&["a", "b"]), &set(&["d"]), |t| {
            Ok(t.name.ends_with("signed"))
        })
        .unwrap();
        let name = |v: &[TagRef]| {
            v.iter()
                .map(|t| short_name(&t.name).to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(name(&got.movable), ["light", "note"]);
        assert_eq!(name(&got.signed), ["signed"]);
        assert_eq!(name(&got.nested), ["nested"]);
        assert_eq!(name(&got.not_utf8), ["caf\u{fffd}"]);
        assert_eq!(name(&got.dropped), ["gone"]);
    }

    #[test]
    fn only_annotated_tags_on_rewritten_commits_are_asked_about_their_signature() {
        let tags = vec![
            tag("light", TagKind::Lightweight, Some("a")),
            tag("other", TagKind::Annotated, Some("z")),
            tag("gone", TagKind::Annotated, Some("d")),
            tag("note", TagKind::Annotated, Some("a")),
        ];
        let mut asked = Vec::new();
        partition(tags, &set(&["a"]), &set(&["d"]), |t| {
            asked.push(short_name(&t.name).to_string());
            Ok(false)
        })
        .unwrap();
        assert_eq!(asked, ["note"]);
    }

    #[test]
    fn warnings_name_each_group_that_is_left_alone() {
        let c = Classified {
            signed: vec![tag("s1", TagKind::Annotated, Some("a"))],
            dropped: vec![
                tag("d1", TagKind::Lightweight, Some("d")),
                tag("d2", TagKind::Lightweight, Some("d")),
            ],
            ..Classified::default()
        };
        let w = c.warnings();
        assert_eq!(w.len(), 2);
        assert!(w[0].contains("signature") && w[0].ends_with("s1"));
        assert!(w[1].contains("dropped") && w[1].ends_with("d1, d2"));
        assert!(Classified::default().warnings().is_empty());
    }
}
