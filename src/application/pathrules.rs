//! Applying the path rules to trees: removing excluded paths and adding the `.gitignore` entries.

use std::cell::RefCell;
use std::collections::HashMap;

use crate::application::ports::{TreeEntry, TreeStore};
use crate::domain::error::Result;
use crate::domain::paths::{GITIGNORE, PathFilter, with_patterns};

/// Rewrites trees through a `TreeStore`. Results are memoized per (tree, directory), so the cost
/// follows the number of distinct trees, not the number of commits.
pub struct TreeRewriter<'a> {
    store: &'a dyn TreeStore,
    filter: &'a PathFilter,
    memo: RefCell<HashMap<(String, String), Option<String>>>,
}

impl<'a> TreeRewriter<'a> {
    pub fn new(store: &'a dyn TreeStore, filter: &'a PathFilter) -> Self {
        TreeRewriter {
            store,
            filter,
            memo: RefCell::new(HashMap::new()),
        }
    }

    /// `tree` without the excluded paths (the same id when nothing is excluded). Directories left
    /// empty by that disappear, as git cannot keep them.
    pub fn without_excluded(&self, tree: &str) -> Result<String> {
        match self.walk(tree, "")? {
            Some(t) => Ok(t),
            None => self.empty_tree(),
        }
    }

    /// The empty tree.
    pub fn empty_tree(&self) -> Result<String> {
        self.store.write_tree(&[])
    }

    /// True when `tree` contains an excluded path.
    pub fn has_excluded(&self, tree: &str) -> Result<bool> {
        Ok(self.without_excluded(tree)? != tree)
    }

    /// The root `.gitignore` of `tree` extended by the patterns (same id when already complete).
    pub fn with_gitignore(&self, tree: &str) -> Result<String> {
        let mut entries = self.store.read_tree(tree)?;
        let slot = entries.iter().position(|e| e.name == GITIGNORE.as_bytes());
        let existing = match slot {
            Some(i) if entries[i].is_tree || !entries[i].mode.starts_with("100") => {
                return Ok(tree.to_string()); // a directory or symlink named .gitignore: leave it
            }
            Some(i) => Some(self.store.read_blob(&entries[i].oid)?),
            None => None,
        };
        let updated = with_patterns(existing.as_deref(), self.filter.patterns());
        if existing.as_deref() == Some(updated.as_slice()) {
            return Ok(tree.to_string());
        }
        let oid = self.store.write_blob(&updated)?;
        match slot {
            Some(i) => entries[i].oid = oid,
            None => entries.push(TreeEntry {
                mode: "100644".into(),
                name: GITIGNORE.as_bytes().to_vec(),
                oid,
                is_tree: false,
            }),
        }
        self.store.write_tree(&entries)
    }

    /// `None` when the directory ends up empty.
    fn walk(&self, oid: &str, dir: &str) -> Result<Option<String>> {
        let key = (oid.to_string(), dir.to_string());
        if let Some(hit) = self.memo.borrow().get(&key) {
            return Ok(hit.clone());
        }
        let entries = self.store.read_tree(oid)?;
        let mut kept = Vec::with_capacity(entries.len());
        let mut changed = false;
        for mut e in entries {
            let path = format!("{dir}{}", String::from_utf8_lossy(&e.name));
            if self.filter.excludes(&path, e.is_tree) {
                changed = true;
                continue;
            }
            if e.is_tree {
                match self.walk(&e.oid, &format!("{path}/"))? {
                    Some(sub) => {
                        changed |= sub != e.oid;
                        e.oid = sub;
                    }
                    None => {
                        changed = true;
                        continue;
                    }
                }
            }
            kept.push(e);
        }
        let result = if !changed {
            Some(oid.to_string())
        } else if kept.is_empty() && !dir.is_empty() {
            None
        } else {
            Some(self.store.write_tree(&kept)?)
        };
        self.memo.borrow_mut().insert(key, result.clone());
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    /// An in-memory object store; ids are derived from the content.
    #[derive(Default)]
    struct Mem {
        trees: RefCell<HashMap<String, Vec<TreeEntry>>>,
        blobs: RefCell<HashMap<String, Vec<u8>>>,
    }

    impl TreeStore for Mem {
        fn read_tree(&self, oid: &str) -> Result<Vec<TreeEntry>> {
            Ok(self.trees.borrow()[oid].clone())
        }
        fn write_tree(&self, entries: &[TreeEntry]) -> Result<String> {
            let mut v = entries.to_vec();
            v.sort_by(|a, b| a.name.cmp(&b.name));
            let id = format!("t{:?}", v);
            self.trees.borrow_mut().insert(id.clone(), v);
            Ok(id)
        }
        fn read_blob(&self, oid: &str) -> Result<Vec<u8>> {
            Ok(self.blobs.borrow()[oid].clone())
        }
        fn write_blob(&self, data: &[u8]) -> Result<String> {
            let id = format!("b{:?}", data);
            self.blobs.borrow_mut().insert(id.clone(), data.to_vec());
            Ok(id)
        }
    }

    fn file(m: &Mem, name: &str, content: &str) -> TreeEntry {
        TreeEntry {
            mode: "100644".into(),
            name: name.as_bytes().to_vec(),
            oid: m.write_blob(content.as_bytes()).unwrap(),
            is_tree: false,
        }
    }

    fn dir(m: &Mem, name: &str, entries: &[TreeEntry]) -> TreeEntry {
        TreeEntry {
            mode: "040000".into(),
            name: name.as_bytes().to_vec(),
            oid: m.write_tree(entries).unwrap(),
            is_tree: true,
        }
    }

    fn names(m: &Mem, tree: &str) -> Vec<String> {
        m.read_tree(tree)
            .unwrap()
            .iter()
            .map(|e| String::from_utf8_lossy(&e.name).to_string())
            .collect()
    }

    fn filter(p: &[&str]) -> PathFilter {
        PathFilter::new(&p.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap()
    }

    #[test]
    fn removes_files_and_directories_and_prunes_empty_dirs() {
        let m = Mem::default();
        let root = m
            .write_tree(&[
                file(&m, "keep.txt", "k"),
                file(&m, "prod.env", "secret"),
                dir(&m, "secrets", &[file(&m, "a", "1")]),
                dir(
                    &m,
                    "src",
                    &[file(&m, "x.env", "s"), file(&m, "main.rs", "m")],
                ),
                dir(&m, "only_env", &[file(&m, "y.env", "s")]),
            ])
            .unwrap();
        let f = filter(&["*.env", "secrets/"]);
        let rw = TreeRewriter::new(&m, &f);
        let out = rw.without_excluded(&root).unwrap();
        assert_eq!(names(&m, &out), ["keep.txt", "src"]);
        let src = m.read_tree(&out).unwrap()[1].oid.clone();
        assert_eq!(names(&m, &src), ["main.rs"]);
        assert!(rw.has_excluded(&root).unwrap());
        assert!(!rw.has_excluded(&out).unwrap());
    }

    #[test]
    fn untouched_trees_keep_their_id() {
        let m = Mem::default();
        let root = m
            .write_tree(&[file(&m, "a", "1"), dir(&m, "d", &[file(&m, "b", "2")])])
            .unwrap();
        let f = filter(&["*.env"]);
        assert_eq!(
            TreeRewriter::new(&m, &f).without_excluded(&root).unwrap(),
            root
        );
    }

    #[test]
    fn everything_excluded_gives_the_empty_tree() {
        let m = Mem::default();
        let root = m.write_tree(&[file(&m, "a.env", "1")]).unwrap();
        let f = filter(&["*.env"]);
        let out = TreeRewriter::new(&m, &f).without_excluded(&root).unwrap();
        assert!(names(&m, &out).is_empty());
    }

    #[test]
    fn anchored_patterns_respect_the_directory() {
        let m = Mem::default();
        let root = m
            .write_tree(&[
                dir(&m, "build", &[file(&m, "o", "1")]),
                dir(&m, "src", &[dir(&m, "build", &[file(&m, "o", "2")])]),
            ])
            .unwrap();
        let f = filter(&["/build"]);
        let out = TreeRewriter::new(&m, &f).without_excluded(&root).unwrap();
        assert_eq!(names(&m, &out), ["src"]);
        let src = m.read_tree(&out).unwrap()[0].oid.clone();
        assert_eq!(names(&m, &src), ["build"]);
    }

    #[test]
    fn gitignore_is_created_extended_and_stable() {
        let m = Mem::default();
        let f = filter(&["*.env"]);
        let rw = TreeRewriter::new(&m, &f);
        let bare = m.write_tree(&[file(&m, "a", "1")]).unwrap();
        let with = rw.with_gitignore(&bare).unwrap();
        assert_eq!(names(&m, &with), [".gitignore", "a"]);
        assert_eq!(rw.with_gitignore(&with).unwrap(), with, "stable");
        let old = m.write_tree(&[file(&m, ".gitignore", "target\n")]).unwrap();
        let ext = rw.with_gitignore(&old).unwrap();
        let blob = m.read_tree(&ext).unwrap()[0].oid.clone();
        let text = String::from_utf8(m.read_blob(&blob).unwrap()).unwrap();
        assert!(
            text.starts_with("target\n") && text.contains("*.env"),
            "{text}"
        );
    }
}
