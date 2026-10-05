//! Conditions under which history must not be rewritten: `check_preconditions` states the ones
//! nothing overrides, `refuse_pushed` the one the caller can allow.

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

/// Commits that are already on the upstream (even a local one) or on any remote (whatever the
/// upstream) may only be rewritten when the caller allows it (`allowed`, the CLI's
/// `--rewrite-pushed`; the error message names that flag). `touched` is everything the rewrite
/// changes or drops.
pub fn refuse_pushed(
    history: &dyn History,
    touched: &[String],
    upstream: Option<&str>,
    allowed: bool,
) -> Result<()> {
    if allowed {
        return Ok(());
    }
    let pushed = pushed_among(history, touched, upstream)?;
    if !pushed.is_empty() {
        return Err(Error::Pushed(format!(
            "{} commit(s) to rewrite are already on a remote or the upstream (e.g. {}); pass --rewrite-pushed to proceed",
            pushed.len(),
            &pushed[0][..pushed[0].len().min(10)]
        )));
    }
    Ok(())
}

/// The commits of `oids` that are already pushed: those the upstream (even a local one, when
/// given) or any remote-tracking ref reaches. This is what `refuse_pushed` refuses, so the pre-push
/// hook asks it too when it decides whether `apply` would refuse.
pub fn pushed_among<'a>(
    history: &dyn History,
    oids: &'a [String],
    upstream: Option<&str>,
) -> Result<Vec<&'a String>> {
    let unpushed = history.unpushed_among(oids, upstream)?;
    Ok(oids.iter().filter(|o| !unpushed.contains(*o)).collect())
}
