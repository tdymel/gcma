//! The plan: the seam between planning and applying. Entries list only the commits to rewrite,
//! parents first.

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use serde::{Deserialize, Serialize};

use super::commit::RawIdent;
use crate::domain::error::{Error, Result};
use crate::domain::settings::Signing;

pub const PLAN_VERSION: u32 = 1;

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
}

impl Entry {
    pub fn message(&self) -> Result<Vec<u8>> {
        Ok(B64.decode(&self.message_b64)?)
    }

    pub fn set_message(&mut self, m: &[u8]) {
        self.message_b64 = B64.encode(m);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub version: u32,
    pub branch_ref: String,
    pub tip_oid: String,
    pub signing: Signing,
    /// Suffix commits only, in write order (parents before children).
    pub entries: Vec<Entry>,
}

impl Plan {
    /// Rejects plans that cannot be applied: only back references between entries are legal.
    pub fn validate(&self) -> Result<()> {
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
        Ok(())
    }

    pub fn branch_name(&self) -> &str {
        self.branch_ref
            .strip_prefix("refs/heads/")
            .unwrap_or(&self.branch_ref)
    }
}
