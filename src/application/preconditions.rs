//! Conditions under which history must not be rewritten at all.

use crate::application::ports::{History, WorkTree};
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

/// Commits that are already on the upstream may only be rewritten when the caller allows it.
/// `touched` is everything the rewrite changes or drops.
pub fn refuse_pushed(
    history: &dyn History,
    touched: &[String],
    upstream: Option<&str>,
    allowed: bool,
) -> Result<()> {
    let Some(up) = upstream else { return Ok(()) };
    let unpushed = history.unpushed_among(touched, up)?;
    let pushed: Vec<&String> = touched.iter().filter(|o| !unpushed.contains(*o)).collect();
    if !pushed.is_empty() && !allowed {
        return Err(Error::Pushed(format!(
            "{} commit(s) to rewrite are already on the upstream (e.g. {}); pass --rewrite-pushed to proceed",
            pushed.len(),
            &pushed[0][..pushed[0].len().min(10)]
        )));
    }
    Ok(())
}
