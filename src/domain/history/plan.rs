//! The plan: the seam between planning and applying. Entries list only the commits to rewrite,
//! parents first.

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use serde::{Deserialize, Serialize};

use super::commit::RawIdent;
use crate::domain::error::{Error, Result};
use crate::domain::settings::Signing;

/// Version 2 added path rules (`paths`, `dropped`, `new_tip`, `Entry::tree`); version 1 plans
/// are valid version 2 plans without them.
/// A full hex object id (SHA-1 or SHA-256).
pub fn is_oid(s: &str) -> bool {
    matches!(s.len(), 40 | 64) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

pub const PLAN_VERSION: u32 = 2;
pub const OLDEST_PLAN_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PIdent {
    pub name: String,
    pub email: String,
    pub time: i64,
    /// UTC offset in minutes.
    pub tz: i32,
}

impl PIdent {
    pub fn to_raw(&self) -> RawIdent {
        RawIdent {
            name: self.name.clone().into_bytes(),
            email: self.email.clone().into_bytes(),
            time: self.time,
            tz: self.tz,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Parent {
    /// Index into `entries` (a rewritten commit).
    In(usize),
    /// A commit that keeps its OID (frozen, or outside the range).
    Base(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub old_oid: String,
    pub parents: Vec<Parent>,
    pub author: PIdent,
    pub committer: PIdent,
    pub message_b64: String,
    /// The tree of the new commit when path rules change it; `None` keeps the old commit's tree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree: Option<String>,
    /// Whether `.gitignore` gets the path patterns in this commit (only with path rules).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub gitignore: bool,
}

impl Entry {
    pub fn message(&self) -> Result<Vec<u8>> {
        Ok(B64.decode(&self.message_b64)?)
    }

    pub fn set_message(&mut self, m: &[u8]) {
        self.message_b64 = B64.encode(m);
    }
}

/// The path rules a plan was made with; apply re-derives every tree from them.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PathRules {
    pub exclude: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub version: u32,
    pub branch_ref: String,
    pub tip_oid: String,
    pub signing: Signing,
    /// Suffix commits only, in write order (parents before children).
    pub entries: Vec<Entry>,
    /// Present when paths are removed from the commits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paths: Option<PathRules>,
    /// Old commits that are removed from history (they only touched excluded paths).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dropped: Vec<String>,
    /// Where the branch ends up when the old tip itself was dropped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_tip: Option<Parent>,
}

impl Plan {
    /// A plan that rewrites `entries` only (no path rules).
    pub fn new(branch_ref: String, tip_oid: String, signing: Signing, entries: Vec<Entry>) -> Plan {
        Plan {
            version: PLAN_VERSION,
            branch_ref,
            tip_oid,
            signing,
            entries,
            paths: None,
            dropped: Vec::new(),
            new_tip: None,
        }
    }

    /// True when applying the plan would change nothing.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.dropped.is_empty()
    }

    /// The new tip: the entry of the old tip, or `new_tip` when that commit was dropped.
    pub fn tip_target(&self) -> Option<Parent> {
        match self.entries.iter().position(|e| e.old_oid == self.tip_oid) {
            Some(i) => Some(Parent::In(i)),
            None => self.new_tip.clone(),
        }
    }

    /// Rejects plans that cannot be applied: only back references between entries are legal.
    pub fn validate(&self) -> Result<()> {
        if !self.branch_ref.starts_with("refs/heads/")
            || self.branch_ref.chars().any(|c| c.is_control() || c == ' ')
        {
            return Err(Error::Usage(format!(
                "plan branch_ref {:?} is not a branch ref (refs/heads/...)",
                self.branch_ref
            )));
        }
        let oids = std::iter::once(&self.tip_oid)
            .chain(&self.dropped)
            .chain(self.entries.iter().map(|e| &e.old_oid))
            .chain(self.entries.iter().filter_map(|e| e.tree.as_ref()))
            .chain(
                self.entries
                    .iter()
                    .flat_map(|e| e.parents.iter())
                    .chain(&self.new_tip)
                    .filter_map(|p| match p {
                        Parent::Base(b) => Some(b),
                        Parent::In(_) => None,
                    }),
            );
        for o in oids {
            if !is_oid(o) {
                return Err(Error::Usage(format!(
                    "plan contains an invalid object id {o:?}"
                )));
            }
        }
        for (i, e) in self.entries.iter().enumerate() {
            for par in &e.parents {
                if let Parent::In(j) = par
                    && *j >= i
                {
                    return Err(Error::Usage(format!(
                        "plan entry {i} has a forward parent reference"
                    )));
                }
            }
        }
        if let Some(Parent::In(j)) = &self.new_tip
            && *j >= self.entries.len()
        {
            return Err(Error::Usage("plan new_tip points past the entries".into()));
        }
        if self.paths.is_none()
            && (!self.dropped.is_empty()
                || self.new_tip.is_some()
                || self.entries.iter().any(|e| e.tree.is_some() || e.gitignore))
        {
            return Err(Error::Usage(
                "plan changes trees or drops commits but has no path rules".into(),
            ));
        }
        Ok(())
    }

    pub fn branch_name(&self) -> &str {
        self.branch_ref
            .strip_prefix("refs/heads/")
            .unwrap_or(&self.branch_ref)
    }
}
