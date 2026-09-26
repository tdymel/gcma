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
        None => {
            let all = list_backups(&repo)?;
            print_backups(all.iter().map(|b| {
                (
                    b.id.as_str(),
                    b.branch.as_str(),
                    b.old.as_str(),
                    b.new.as_str(),
                )
            }));
        }
        Some(id) if prune_it => print_pruned(&prune(&repo, &id)?.id),
        Some(id) => {
            let r = restore(&repo, &id, force)?;
            print_restored(
                &r.backup.branch,
                &r.backup.old,
                &r.notes,
                r.parked.as_deref(),
            );
        }
    }
    Ok(())
}
