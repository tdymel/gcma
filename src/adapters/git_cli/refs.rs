//! `RefStore` over `rev-parse`, `symbolic-ref`, `for-each-ref` and `update-ref --stdin`.

use std::collections::HashSet;

use super::runner::GitCli;
use crate::application::ports::{RefStore, RefUpdate};
use crate::domain::error::{Error, Result};
use crate::domain::history::commit::short;
use crate::domain::history::plan::HEADS_PREFIX;

impl RefStore for GitCli {
    fn current_branch_ref(&self) -> Result<Option<String>> {
        self.try_text(&["symbolic-ref", "-q", "HEAD"])
    }

    fn ref_value(&self, name: &str) -> Result<Option<String>> {
        self.try_text(&["rev-parse", "--verify", "-q", name])
    }

    fn resolve_commit(&self, rev: &str) -> Result<Option<String>> {
        let spec = format!("{rev}^{{commit}}");
        self.try_text(&["rev-parse", "--verify", "-q", &spec])
    }

    fn upstream_oid(&self, branch_ref: &str) -> Result<Option<String>> {
        // `@{upstream}` only resolves with a short branch name, not `refs/heads/<name>`.
        let short = branch_ref.strip_prefix(HEADS_PREFIX).unwrap_or(branch_ref);
        self.resolve_commit(&format!("{short}@{{upstream}}"))
    }

    fn list_refs(&self, prefix: &str) -> Result<Vec<(String, String)>> {
        let out = self.text(&["for-each-ref", "--format=%(refname) %(objectname)", prefix])?;
        Ok(out
            .lines()
            .filter_map(|l| l.split_once(' '))
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect())
    }

    fn update_refs(&self, message: &str, updates: &[RefUpdate]) -> Result<()> {
        let mut input = String::from("start\n");
        for u in updates {
            match u {
                RefUpdate::Create { name, new } => {
                    input.push_str(&format!("create {name} {new}\n"))
                }
                RefUpdate::Move { name, new, old } => {
                    input.push_str(&format!("update {name} {new} {old}\n"))
                }
                RefUpdate::Set { name, new } => input.push_str(&format!("update {name} {new}\n")),
                RefUpdate::Delete { name, old } => {
                    input.push_str(&format!("delete {name} {old}\n"))
                }
            }
        }
        input.push_str("prepare\ncommit\n");
        let mut c = self.command();
        c.args(["update-ref", "-m", message, "--stdin"]);
        let o = self.exec(c, Some(input.as_bytes()))?;
        if !o.ok {
            let why = String::from_utf8_lossy(&o.stderr).trim().to_string();
            // A compare-and-swap that lost, or a backup ref that exists already; anything else
            // (a name clash, a lock, a broken repository) is a plain git failure.
            return Err(
                if why.contains("but expected") || why.contains("already exists") {
                    Error::TipMoved(format!(
                        "ref transaction failed (did the branch move?): {why}"
                    ))
                } else {
                    Error::Git(format!("ref transaction failed: {why}"))
                },
            );
        }
        Ok(())
    }

    fn remotes(&self) -> Result<Vec<String>> {
        Ok(self.text(&["remote"])?.lines().map(String::from).collect())
    }

    fn labels_pointing_at(&self, oids: &HashSet<String>) -> Result<Vec<String>> {
        let mut found = Vec::new();
        let out = self.text(&[
            "for-each-ref",
            "--format=%(refname) %(objectname) %(*objectname)",
            "refs/tags",
        ])?;
        for l in out.lines() {
            let parts: Vec<&str> = l.split(' ').collect();
            if parts.len() >= 2
                && (oids.contains(parts[1]) || parts.get(2).is_some_and(|t| oids.contains(*t)))
            {
                found.push(parts[0].to_string());
            }
        }
        if let Ok(notes) = self.text(&["notes", "list"]) {
            for l in notes.lines() {
                if let Some((_, target)) = l.split_once(' ')
                    && oids.contains(target)
                {
                    found.push(format!("note on {}", short(target)));
                }
            }
        }
        Ok(found)
    }
}
