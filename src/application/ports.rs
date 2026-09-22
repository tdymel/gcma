//! Ports: what the use cases need from the outside world. Adapters implement these traits.
//! Everything is expressed in domain terms; nothing here mentions a particular git binary or library.

use std::collections::HashSet;

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
    /// Tags and notes that point at any of `oids`, as human-readable labels.
    fn labels_pointing_at(&self, oids: &HashSet<String>) -> Result<Vec<String>>;
}

/// Questions about the commit graph.
pub trait History {
    fn merge_base(&self, a: &str, b: &str) -> Result<Option<String>>;
    fn is_ancestor(&self, ancestor: &str, descendant: &str) -> Result<bool>;
    /// The range as (commit, parents), parents first.
    fn list_range(&self, range: &RevRange) -> Result<Vec<(String, Vec<String>)>>;
    fn count_reachable(&self, rev: &str) -> Result<usize>;
    /// Of `oids`, those NOT reachable from `upstream`.
    fn unpushed_among(&self, oids: &[String], upstream: &str) -> Result<HashSet<String>>;
    /// True when everything reachable from `commits` is also reachable from `tip`.
    fn all_reachable_from(&self, commits: &[String], tip: &str) -> Result<bool>;
    /// True when the two commits have identical trees.
    fn same_tree(&self, a: &str, b: &str) -> Result<bool>;
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
}

/// Everything a use case may ask of a repository.
pub trait Repository: CommitStore + RefStore + History + WorkTree {}

impl<T: CommitStore + RefStore + History + WorkTree> Repository for T {}
