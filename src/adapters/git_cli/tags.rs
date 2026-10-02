//! `TagStore` and `NoteStore` over `for-each-ref`, `hash-object` and `git notes`.

use super::runner::GitCli;
use crate::application::ports::{NoteList, NoteStore, TagKind, TagRef, TagStore};
use crate::domain::error::{Error, Result};

const NOTES_PREFIX: &str = "refs/notes/";

impl TagStore for GitCli {
    fn list_tags(&self) -> Result<Vec<TagRef>> {
        // `%(type)` is what a tag object points at directly; `%(*objectname)` and
        // `%(*objecttype)` are what it finally peels to, through any number of tag objects.
        // `%(symref)` is the target of a symbolic ref, which is left out: it follows its target.
        let format = "--format=%(refname)%00%(objectname)%00%(objecttype)%00%(type)%00%(*objectname)%00%(*objecttype)%00%(symref)";
        // A ref name need not be UTF-8 (nor can it hold a newline or NUL), so it is read as bytes.
        let out = self.run(&["for-each-ref", format, "refs/tags"])?;
        let mut tags = Vec::new();
        for line in out.split(|&b| b == b'\n').filter(|l| !l.is_empty()) {
            let name_is_utf8 = line
                .split(|&b| b == 0)
                .next()
                .is_some_and(|n| str::from_utf8(n).is_ok());
            let line = String::from_utf8_lossy(line);
            let f: Vec<&str> = line.split('\0').collect();
            let [name, value, kind, direct, target, target_kind, symref] = f[..] else {
                return Err(Error::Git(format!("unexpected for-each-ref line {line:?}")));
            };
            if !symref.is_empty() {
                continue;
            }
            let (kind, peeled) = match (kind, direct) {
                ("tag", "tag") => (
                    TagKind::Nested,
                    (target_kind == "commit").then(|| target.to_string()),
                ),
                ("tag", direct) => (
                    TagKind::Annotated,
                    (direct == "commit").then(|| target.to_string()),
                ),
                (t, _) => (
                    TagKind::Lightweight,
                    (t == "commit").then(|| value.to_string()),
                ),
            };
            tags.push(TagRef {
                name: name.to_string(),
                name_is_utf8,
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
        let out = self.run_stdin(
            &["hash-object", "-t", "tag", "--literally", "-w", "--stdin"],
            raw,
        )?;
        Ok(String::from_utf8_lossy(&out).trim().to_string())
    }
}

impl NoteStore for GitCli {
    fn list_notes(&self) -> Result<NoteList> {
        let refs = self.text(&["for-each-ref", "--format=%(refname)", NOTES_PREFIX])?;
        let mut out = NoteList::default();
        for notes_ref in refs.lines() {
            let list = match self.text(&["notes", "--ref", notes_ref, "list"]) {
                Ok(list) => list,
                Err(e) => {
                    out.skipped
                        .push((notes_ref.to_string(), e.to_string().trim().to_string()));
                    continue;
                }
            };
            for line in list.lines() {
                if let Some((_blob, annotated)) = line.split_once(' ') {
                    out.notes
                        .push((notes_ref.to_string(), annotated.to_string()));
                }
            }
        }
        Ok(out)
    }

    fn copy_note(&self, notes_ref: &str, from: &str, to: &str) -> Result<()> {
        self.run(&["notes", "--ref", notes_ref, "copy", from, to])
            .map(drop)
    }
}
