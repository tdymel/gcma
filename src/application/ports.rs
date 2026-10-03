//! Ports: what the use cases need from the outside world. Adapters implement these traits.
//! Everything is expressed in domain terms; nothing here mentions a particular git binary or library.

use std::collections::{HashMap, HashSet};

use crate::domain::error::Result;
use crate::domain::history::commit::{Commit, NewCommit};
use crate::domain::settings::Signing;

/// Which remote-tracking refs count as "already pushed" when excluding from a range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteScope {
    All,
    Named(String),
}

/// Commits reachable from `tip` but not from any excluded commit or remote-tracking ref.
#[derive(Debug, Clone)]
pub struct RevRange {
    pub tip: String,
    pub exclude_commits: Vec<String>,
    pub exclude_remotes: Option<RemoteScope>,
}

/// One step of an atomic ref transaction. Compare-and-swap where an old value is given.
#[derive(Debug, Clone)]
pub enum RefUpdate {
    /// Fails if the ref already exists.
    Create { name: String, new: String },
    /// Fails unless the ref currently points at `old`.
    Move {
        name: String,
        new: String,
        old: String,
    },
    /// Creates or overwrites without checking.
    Set { name: String, new: String },
    /// Fails unless the ref currently points at `old`.
    Delete { name: String, old: String },
}

/// The object database: commits are read and written as raw, byte-exact objects.
pub trait CommitStore {
    fn read_commits(&self, oids: &[String]) -> Result<Vec<Commit>>;
    fn objects_exist(&self, oids: &[String]) -> Result<bool>;
    /// Writes the commit (signed when `signing` is `Resign`) and returns its id.
    fn write_commit(&self, commit: &NewCommit, signing: Signing) -> Result<String>;
}

/// One entry of a tree object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeEntry {
    /// Octal mode as git prints it (`100644`, `040000`, `120000`, `160000`, ...).
    pub mode: String,
    /// Raw name bytes (names need not be UTF-8).
    pub name: Vec<u8>,
    pub oid: String,
}

/// What a tree entry is, as far as the path rules care.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Symlink,
    Tree,
    Submodule,
}

impl TreeEntry {
    pub fn new(mode: &str, name: &[u8], oid: &str) -> TreeEntry {
        TreeEntry {
            mode: mode.to_string(),
            name: name.to_vec(),
            oid: oid.to_string(),
        }
    }

    pub fn kind(&self) -> EntryKind {
        match self.mode.as_str() {
            "040000" | "40000" => EntryKind::Tree,
            "160000" => EntryKind::Submodule,
            "120000" => EntryKind::Symlink,
            _ => EntryKind::File,
        }
    }

    pub fn is_tree(&self) -> bool {
        self.kind() == EntryKind::Tree
    }
}

/// Trees and blobs, needed when the content of commits changes (path rules).
pub trait TreeStore {
    fn read_tree(&self, oid: &str) -> Result<Vec<TreeEntry>>;
    /// Writes a tree; the order of `entries` does not matter.
    fn write_tree(&self, entries: &[TreeEntry]) -> Result<String>;
    fn read_blob(&self, oid: &str) -> Result<Vec<u8>>;
    fn write_blob(&self, data: &[u8]) -> Result<String>;
}

/// Named references and the transaction that moves them.
pub trait RefStore {
    fn current_branch_ref(&self) -> Result<Option<String>>;
    fn ref_value(&self, name: &str) -> Result<Option<String>>;
    /// Resolves a revision to a commit id, `None` when it does not resolve.
    fn resolve_commit(&self, rev: &str) -> Result<Option<String>>;
    fn upstream_oid(&self, branch_ref: &str) -> Result<Option<String>>;
    fn list_refs(&self, prefix: &str) -> Result<Vec<(String, String)>>;
    fn update_refs(&self, message: &str, updates: &[RefUpdate]) -> Result<()>;
    fn remotes(&self) -> Result<Vec<String>>;
}

/// What a tag ref holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagKind {
    /// The ref points straight at the object.
    Lightweight,
    /// The ref points at a tag object that points at the target.
    Annotated,
    /// The ref points at a tag object that points at another tag object.
    Nested,
}

/// A ref under `refs/tags/`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagRef {
    /// The full ref name (`refs/tags/v1`), decoded lossily when it is not UTF-8.
    pub name: String,
    /// False when the name is not valid UTF-8: `name` is then not the real name and such a tag
    /// cannot be moved.
    pub name_is_utf8: bool,
    /// What the ref holds: a commit, or a tag object.
    pub value: String,
    pub kind: TagKind,
    /// The commit the tag finally points at; `None` when it is not a commit (a tree, a blob).
    pub peeled: Option<String>,
}

/// Tags: the refs and the tag objects behind annotated ones.
pub trait TagStore {
    /// Every ref under `refs/tags/`, except symbolic refs: they follow their target.
    fn list_tags(&self) -> Result<Vec<TagRef>>;
    /// The raw bytes of a tag object.
    fn read_tag_object(&self, oid: &str) -> Result<Vec<u8>>;
    /// Writes a raw tag object as it is (no fsck, so a legacy tag without a tagger can be copied)
    /// and returns its id.
    fn write_tag_object(&self, raw: &[u8]) -> Result<String>;
}

/// The notes of every notes ref, and the notes refs that could not be read.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NoteList {
    /// (notes ref, annotated commit) for every note of the notes refs that could be read.
    pub notes: Vec<(String, String)>,
    /// (notes ref, why) for every notes ref that could not be read; its notes are left out.
    pub skipped: Vec<(String, String)>,
}

impl NoteList {
    /// The notes `list_notes` returned, or when it failed altogether, no notes and the failure as
    /// the one notes ref (`refs/notes/`) that could not be read.
    pub fn or_unlisted(listed: Result<NoteList>) -> NoteList {
        listed.unwrap_or_else(|e| NoteList {
            notes: Vec::new(),
            skipped: vec![("refs/notes/".to_string(), e.to_string())],
        })
    }

    /// One warning per notes ref that could not be read.
    pub fn warnings(&self) -> Vec<String> {
        self.skipped
            .iter()
            .map(|(r, why)| {
                format!("could not read the notes in {r}, so they are left out ({why})")
            })
            .collect()
    }
}

/// Notes (`refs/notes/*`) attached to commits.
pub trait NoteStore {
    /// Every note in every notes ref; a notes ref that cannot be read is skipped, not an error.
    fn list_notes(&self) -> Result<NoteList>;
    /// Copies the note of `from` to `to` in the notes ref; fails if `to` has a note already.
    fn copy_note(&self, notes_ref: &str, from: &str, to: &str) -> Result<()>;
}

/// Questions about the commit graph.
pub trait History {
    fn merge_base(&self, a: &str, b: &str) -> Result<Option<String>>;
    fn is_ancestor(&self, ancestor: &str, descendant: &str) -> Result<bool>;
    /// The commits of the range, parents first.
    fn list_range(&self, range: &RevRange) -> Result<Vec<String>>;
    /// The commits reachable from `tips` but from no local branch, each with its parents (which
    /// may be on a branch).
    fn commits_off_branches(&self, tips: &[String]) -> Result<HashMap<String, Vec<String>>>;
    fn count_reachable(&self, rev: &str) -> Result<usize>;
    /// Of `oids`, those NOT reachable from `upstream`.
    fn unpushed_among(&self, oids: &[String], upstream: &str) -> Result<HashSet<String>>;
    /// True when everything reachable from `commits` is also reachable from `tip`.
    fn all_reachable_from(&self, commits: &[String], tip: &str) -> Result<bool>;
    /// True when the two commits have identical trees.
    fn same_tree(&self, a: &str, b: &str) -> Result<bool>;
    /// Paths whose content differs between two trees (or commits).
    fn changed_paths(&self, a: &str, b: &str) -> Result<Vec<String>>;
    /// (additions, deletions, files) against the first parent, in the order of `oids`.
    fn change_stats(&self, oids: &[String]) -> Result<Vec<(u64, u64, u64)>>;
}

/// The state of the working copy that makes rewriting unsafe.
pub trait WorkTree {
    fn is_shallow(&self) -> Result<bool>;
    fn has_replace_refs(&self) -> Result<bool>;
    fn has_grafts(&self) -> Result<bool>;
    fn index_dirty(&self) -> Result<bool>;
    /// Name of a rebase/merge/cherry-pick/revert in progress, if any.
    fn operation_in_progress(&self) -> Result<Option<&'static str>>;
    /// A file of the working copy (path relative to its root); `None` when it does not exist.
    fn read_file(&self, rel: &str) -> Result<Option<Vec<u8>>>;
    fn write_file(&self, rel: &str, data: &[u8]) -> Result<()>;
    fn remove_file(&self, rel: &str) -> Result<()>;
    /// Makes the index match `HEAD`; the working copy is not touched.
    fn reset_index_to_head(&self) -> Result<()>;
}

/// Everything a use case may ask of a repository.
pub trait Repository:
    CommitStore + TreeStore + RefStore + TagStore + NoteStore + History + WorkTree
{
}

impl<T: CommitStore + TreeStore + RefStore + TagStore + NoteStore + History + WorkTree> Repository
    for T
{
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::error::Error;

    #[test]
    fn a_failed_listing_is_one_unreadable_notes_ref() {
        let listed = NoteList {
            notes: vec![("refs/notes/commits".into(), "a".into())],
            skipped: Vec::new(),
        };
        assert_eq!(NoteList::or_unlisted(Ok(listed.clone())), listed);
        let failed = NoteList::or_unlisted(Err(Error::Git("boom".into())));
        assert!(failed.notes.is_empty());
        let warnings = failed.warnings();
        assert_eq!(warnings.len(), 1);
        assert!(
            warnings[0].starts_with("could not read the notes in refs/notes/,"),
            "{warnings:?}"
        );
        assert!(warnings[0].contains("boom"), "{warnings:?}");
    }
}
