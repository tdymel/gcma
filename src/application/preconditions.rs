//! Conditions under which history must not be rewritten at all.

use crate::application::ports::WorkTree;
use crate::domain::error::{Error, Result};

pub fn check_preconditions(tree: &dyn WorkTree, strict: bool) -> Result<()> {
    let refuse = |m: &str| Err(Error::Precondition(m.to_string()));
    if tree.is_shallow()? {
        return refuse("shallow clones are not supported (parents are missing)");
    }
    if tree.has_replace_refs()? || tree.has_grafts()? {
        return refuse("replace refs / grafts are present; refusing to rewrite");
    }
    if strict {
        if let Some(op) = tree.operation_in_progress()? {
            return Err(Error::Precondition(format!("a {op} is in progress")));
        }
        if tree.index_dirty()? {
            return refuse("the index has staged changes; commit or stash them first");
        }
    }
    Ok(())
}
