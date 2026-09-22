//! `TreeStore` over `ls-tree`, `mktree`, `cat-file` and `hash-object`.

use super::runner::GitCli;
use crate::application::ports::{TreeEntry, TreeStore};
use crate::domain::error::{Error, Result};

impl TreeStore for GitCli {
    fn read_tree(&self, oid: &str) -> Result<Vec<TreeEntry>> {
        let out = self.run(&["ls-tree", "-z", oid])?;
        out.split(|&b| b == 0)
            .filter(|rec| !rec.is_empty())
            .map(parse_entry)
            .collect()
    }

    fn write_tree(&self, entries: &[TreeEntry]) -> Result<String> {
        let mut input = Vec::new();
        for e in entries {
            let kind = if e.is_tree {
                "tree"
            } else if e.mode == "160000" {
                "commit"
            } else {
                "blob"
            };
            input.extend_from_slice(format!("{} {kind} {}\t", e.mode, e.oid).as_bytes());
            input.extend_from_slice(&e.name);
            input.push(0);
        }
        // `--missing`: a submodule's commit is not in this object database.
        let out = self.run_stdin(&["mktree", "-z", "--missing"], &input)?;
        Ok(String::from_utf8_lossy(&out).trim().to_string())
    }

    fn read_blob(&self, oid: &str) -> Result<Vec<u8>> {
        self.run(&["cat-file", "blob", oid])
    }

    fn write_blob(&self, data: &[u8]) -> Result<String> {
        let out = self.run_stdin(&["hash-object", "-t", "blob", "-w", "--stdin"], data)?;
        Ok(String::from_utf8_lossy(&out).trim().to_string())
    }
}

/// `<mode> SP <type> SP <oid> TAB <name>`
fn parse_entry(rec: &[u8]) -> Result<TreeEntry> {
    let bad = || {
        Error::Git(format!(
            "unexpected ls-tree line: {:?}",
            String::from_utf8_lossy(rec)
        ))
    };
    let tab = rec.iter().position(|&b| b == b'\t').ok_or_else(bad)?;
    let meta = String::from_utf8_lossy(&rec[..tab]).to_string();
    let mut parts = meta.split(' ');
    let (mode, kind, oid) = (
        parts.next().ok_or_else(bad)?,
        parts.next().ok_or_else(bad)?,
        parts.next().ok_or_else(bad)?,
    );
    Ok(TreeEntry {
        mode: mode.to_string(),
        name: rec[tab + 1..].to_vec(),
        oid: oid.to_string(),
        is_tree: kind == "tree",
    })
}
