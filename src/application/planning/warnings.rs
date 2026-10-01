//! Non-fatal warnings about the commits a plan would rewrite.

use std::collections::{HashMap, HashSet};

use crate::application::ports::{NoteStore, Repository, TagStore};
use crate::domain::error::Result;
use crate::domain::history::commit::{Commit, short};
use crate::domain::settings::{Config, Signing};

/// Non-fatal consequences of rewriting these commits. With `retag` the tags and notes follow the
/// rewrite, so there is nothing to warn about for them here.
pub(super) fn rewrite_warnings(
    repo: &dyn Repository,
    cfg: &Config,
    linear: &[String],
    commits: &HashMap<String, Commit>,
    retag: bool,
) -> Result<Vec<String>> {
    let mut warnings = Vec::new();
    let labels = match retag {
        true => Vec::new(),
        false => labels_pointing_at(repo, &linear.iter().cloned().collect())?,
    };
    if !labels.is_empty() {
        warnings.push(format!(
            "tags/notes point at commits that will be rewritten and will keep pointing at the old ones: {} \
             (pass --retag to move them)",
            labels.join(", ")
        ));
    }
    if cfg.signing == Signing::Resign {
        let lossy = linear
            .iter()
            .filter(|o| {
                commits[*o]
                    .extra
                    .iter()
                    .any(|h| !h.is_invalidated_by_rewrite())
            })
            .count();
        if lossy > 0 {
            warnings.push(format!("{lossy} commit(s) carry extra headers (e.g. encoding) that `signing: resign` cannot preserve"));
        }
        let binary = linear
            .iter()
            .filter(|o| std::str::from_utf8(&commits[*o].message).is_err())
            .count();
        if binary > 0 {
            warnings.push(format!(
                "{binary} commit(s) have messages that are not valid UTF-8; git recodes those when it signs, so `apply` \
                 refuses them with `signing: resign` unless an imported reply gives them a new message (or use `signing: strip`)"
            ));
        }
    }
    Ok(warnings)
}

/// The tags (by full ref name) and notes that finally point at any of `oids`, as labels. Notes
/// that cannot be listed are left out.
fn labels_pointing_at<R: TagStore + NoteStore + ?Sized>(
    repo: &R,
    oids: &HashSet<String>,
) -> Result<Vec<String>> {
    let tags = repo.list_tags()?.into_iter().filter_map(|t| {
        t.peeled
            .as_ref()
            .is_some_and(|c| oids.contains(c))
            .then_some(t.name)
    });
    let notes = repo
        .list_notes()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(_, c)| oids.contains(&c).then(|| format!("note on {}", short(&c))));
    Ok(tags.chain(notes).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::ports::{TagKind, TagRef};

    struct Fake;

    impl TagStore for Fake {
        fn list_tags(&self) -> Result<Vec<TagRef>> {
            let tag = |name: &str, kind, peeled: Option<&str>| TagRef {
                name: format!("refs/tags/{name}"),
                name_is_utf8: true,
                value: "v".into(),
                kind,
                peeled: peeled.map(String::from),
            };
            Ok(vec![
                tag("light", TagKind::Lightweight, Some("aaaaaaaaaaaa")),
                tag("nested", TagKind::Nested, Some("aaaaaaaaaaaa")),
                tag("other", TagKind::Annotated, Some("zzzzzzzzzzzz")),
                tag("tree", TagKind::Lightweight, None),
            ])
        }
        fn read_tag_object(&self, _: &str) -> Result<Vec<u8>> {
            unreachable!()
        }
        fn write_tag_object(&self, _: &[u8]) -> Result<String> {
            unreachable!()
        }
    }

    impl NoteStore for Fake {
        fn list_notes(&self) -> Result<Vec<(String, String)>> {
            Ok(vec![
                ("refs/notes/commits".into(), "aaaaaaaaaaaa".into()),
                ("refs/notes/commits".into(), "zzzzzzzzzzzz".into()),
            ])
        }
        fn copy_note(&self, _: &str, _: &str, _: &str) -> Result<()> {
            unreachable!()
        }
    }

    #[test]
    fn labels_name_the_tags_and_notes_on_the_given_commits() {
        let oids = HashSet::from(["aaaaaaaaaaaa".to_string()]);
        assert_eq!(
            labels_pointing_at(&Fake, &oids).unwrap(),
            ["refs/tags/light", "refs/tags/nested", "note on aaaaaaaa"]
        );
    }
}
