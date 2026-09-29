//! `gcma restore`: list backups, restore one, or prune one. Never reads the config, so a broken
//! `gcma.yml` cannot get in the way of an undo.

use crate::adapters::cli_support::report::{print_backups, print_pruned, print_restored};
use crate::adapters::cli_support::session::Session;
use crate::application::rewrite::{list_backups, prune, restore};
use crate::domain::error::{Error, Result};

pub fn run(s: &Session, id: Option<String>, force: bool, prune_it: bool) -> Result<()> {
    if id.is_none() && prune_it {
        return Err(Error::Usage("--prune needs a backup id".into()));
    }
    let repo = s.open_repo()?;
    match id {
        None => print_backups(&list_backups(&repo)?),
        Some(id) if prune_it => print_pruned(&prune(&repo, &id)?.id),
        Some(id) => print_restored(&restore(&repo, &id, force)?),
    }
    Ok(())
}
