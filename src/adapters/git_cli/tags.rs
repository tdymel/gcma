//! `TagStore` and `NoteStore` over `for-each-ref`, `hash-object` and `git notes`.

use super::runner::GitCli;
use crate::application::ports::{NoteStore, RefStore, TagKind, TagRef, TagStore};
use crate::domain::error::{Error, Result};

const NOTES_PREFIX: &str = "refs/notes/";

impl TagStore for GitCli {
    fn list_tags(&self) -> Result<Vec<TagRef>> {
        let format =
            "--format=%(refname)%00%(objectname)%00%(objecttype)%00%(*objectname)%00%(*objecttype)";
        let out = self.text(&["for-each-ref", format, "refs/tags"])?;
        let mut tags = Vec::new();
        for line in out.lines() {
            let f: Vec<&str> = line.split('\0').collect();
            let [name, value, kind, target, target_kind] = f[..] else {
                return Err(Error::Git(format!("unexpected for-each-ref line {line:?}")));
            };
            let (kind, peeled) = match (kind, target_kind) {
                ("tag", "tag") => (TagKind::Nested, self.resolve_commit(value)?),
                ("tag", t) => (
                    TagKind::Annotated,
                    (t == "commit").then(|| target.to_string()),
                ),
                (t, _) => (
                    TagKind::Lightweight,
                    (t == "commit").then(|| value.to_string()),
                ),
            };
            tags.push(TagRef {
                name: name.to_string(),
                value: value.to_string(),
                kind,
                peeled,
            });
        }
        Ok(tags)
    }

    fn read_tag_object(&self, oid: &str) -> Result<Vec<u8>> {
        self.run(&["cat-file", "tag", oid])
    }

    fn write_tag_object(&self, raw: &[u8]) -> Result<String> {
        let out = self.run_stdin(&["hash-object", "-t", "tag", "-w", "--stdin"], raw)?;
        Ok(String::from_utf8_lossy(&out).trim().to_string())
    }
}

impl NoteStore for GitCli {
    fn list_notes(&self) -> Result<Vec<(String, String)>> {
        let refs = self.text(&["for-each-ref", "--format=%(refname)", NOTES_PREFIX])?;
        let mut notes = Vec::new();
        for notes_ref in refs.lines() {
            let list = self.text(&["notes", "--ref", notes_ref, "list"])?;
            for line in list.lines() {
                if let Some((_blob, annotated)) = line.split_once(' ') {
                    notes.push((notes_ref.to_string(), annotated.to_string()));
                }
            }
        }
        Ok(notes)
    }

    fn copy_note(&self, notes_ref: &str, from: &str, to: &str) -> Result<()> {
        self.run(&["notes", "--ref", notes_ref, "copy", from, to])
            .map(drop)
    }
}
