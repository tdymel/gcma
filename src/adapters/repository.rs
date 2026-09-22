//! The composed repository: git CLI for everything, with the object operations optionally
//! served in-process by the gix store.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::git_cli::GitCli;
use crate::application::ports::{
    CommitStore, History, RefStore, RefUpdate, RevRange, TreeEntry, TreeStore, WorkTree,
};
#[cfg(not(feature = "gix"))]
use crate::domain::error::Error;
use crate::domain::error::Result;
use crate::domain::history::commit::{Commit, NewCommit};
use crate::domain::settings::{Backend, Signing};

/// A repository whose hot object operations run on the selected backend.
pub struct GitRepository {
    cli: GitCli,
    objects: Option<Box<dyn CommitStore>>,
}

impl GitRepository {
    /// Attaches the object backend. `Gix` fails cleanly when it was not compiled in.
    pub fn with_backend(cli: GitCli, backend: Backend) -> Result<GitRepository> {
        let objects: Option<Box<dyn CommitStore>> = match backend {
            Backend::Git => None,
            #[cfg(feature = "gix")]
            Backend::Gix => Some(Box::new(super::gix_store::GixStore::open(cli.dir())?)),
            #[cfg(not(feature = "gix"))]
            Backend::Gix => {
                return Err(Error::Usage(
                    "this build has no gix backend (rebuild with default features)".into(),
                ));
            }
        };
        Ok(GitRepository { cli, objects })
    }

    /// The backend in effect.
    pub fn backend(&self) -> Backend {
        if self.objects.is_some() {
            Backend::Gix
        } else {
            Backend::Git
        }
    }

    pub fn dir(&self) -> &Path {
        self.cli.dir()
    }

    pub fn git_path(&self, p: &str) -> Result<PathBuf> {
        self.cli.git_path(p)
    }
}

impl CommitStore for GitRepository {
    fn read_commits(&self, oids: &[String]) -> Result<Vec<Commit>> {
        match &self.objects {
            Some(o) => o.read_commits(oids),
            None => self.cli.read_commits(oids),
        }
    }

    fn objects_exist(&self, oids: &[String]) -> Result<bool> {
        match &self.objects {
            Some(o) => o.objects_exist(oids),
            None => self.cli.objects_exist(oids),
        }
    }

    fn write_commit(&self, commit: &NewCommit, signing: Signing) -> Result<String> {
        match (&self.objects, signing) {
            (Some(o), Signing::Strip) => o.write_commit(commit, signing),
            _ => self.cli.write_commit(commit, signing),
        }
    }
}

impl TreeStore for GitRepository {
    fn read_tree(&self, oid: &str) -> Result<Vec<TreeEntry>> {
        self.cli.read_tree(oid)
    }
    fn write_tree(&self, entries: &[TreeEntry]) -> Result<String> {
        self.cli.write_tree(entries)
    }
    fn read_blob(&self, oid: &str) -> Result<Vec<u8>> {
        self.cli.read_blob(oid)
    }
    fn write_blob(&self, data: &[u8]) -> Result<String> {
        self.cli.write_blob(data)
    }
}

impl RefStore for GitRepository {
    fn current_branch_ref(&self) -> Result<Option<String>> {
        self.cli.current_branch_ref()
    }
    fn ref_value(&self, name: &str) -> Result<Option<String>> {
        self.cli.ref_value(name)
    }
    fn resolve_commit(&self, rev: &str) -> Result<Option<String>> {
        self.cli.resolve_commit(rev)
    }
    fn upstream_oid(&self, branch_ref: &str) -> Result<Option<String>> {
        self.cli.upstream_oid(branch_ref)
    }
    fn list_refs(&self, prefix: &str) -> Result<Vec<(String, String)>> {
        self.cli.list_refs(prefix)
    }
    fn update_refs(&self, message: &str, updates: &[RefUpdate]) -> Result<()> {
        self.cli.update_refs(message, updates)
    }
    fn remotes(&self) -> Result<Vec<String>> {
        self.cli.remotes()
    }
    fn labels_pointing_at(&self, oids: &HashSet<String>) -> Result<Vec<String>> {
        self.cli.labels_pointing_at(oids)
    }
}

impl History for GitRepository {
    fn merge_base(&self, a: &str, b: &str) -> Result<Option<String>> {
        self.cli.merge_base(a, b)
    }
    fn is_ancestor(&self, ancestor: &str, descendant: &str) -> Result<bool> {
        self.cli.is_ancestor(ancestor, descendant)
    }
    fn list_range(&self, range: &RevRange) -> Result<Vec<(String, Vec<String>)>> {
        self.cli.list_range(range)
    }
    fn count_reachable(&self, rev: &str) -> Result<usize> {
        self.cli.count_reachable(rev)
    }
    fn unpushed_among(&self, oids: &[String], upstream: &str) -> Result<HashSet<String>> {
        self.cli.unpushed_among(oids, upstream)
    }
    fn all_reachable_from(&self, commits: &[String], tip: &str) -> Result<bool> {
        self.cli.all_reachable_from(commits, tip)
    }
    fn same_tree(&self, a: &str, b: &str) -> Result<bool> {
        self.cli.same_tree(a, b)
    }
    fn changed_paths(&self, a: &str, b: &str) -> Result<Vec<String>> {
        self.cli.changed_paths(a, b)
    }
    fn change_stats(&self, oids: &[String]) -> Result<Vec<(u64, u64, u64)>> {
        self.cli.change_stats(oids)
    }
}

impl WorkTree for GitRepository {
    fn is_shallow(&self) -> Result<bool> {
        self.cli.is_shallow()
    }
    fn has_replace_refs(&self) -> Result<bool> {
        self.cli.has_replace_refs()
    }
    fn has_grafts(&self) -> Result<bool> {
        self.cli.has_grafts()
    }
    fn index_dirty(&self) -> Result<bool> {
        self.cli.index_dirty()
    }
    fn operation_in_progress(&self) -> Result<Option<&'static str>> {
        self.cli.operation_in_progress()
    }
    fn read_file(&self, rel: &str) -> Result<Option<Vec<u8>>> {
        self.cli.read_file(rel)
    }
    fn write_file(&self, rel: &str, data: &[u8]) -> Result<()> {
        self.cli.write_file(rel, data)
    }
    fn reset_index_to_head(&self) -> Result<()> {
        self.cli.reset_index_to_head()
    }
}
