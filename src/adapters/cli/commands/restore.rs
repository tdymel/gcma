//! `ghma restore`: list backups, restore one, or prune one.

use crate::adapters::cli_support::session::Session;
use crate::application::rewrite::{list_backups, prune, restore};
use crate::domain::error::{Error, Result};

pub fn run(s: &Session, id: Option<String>, force: bool, prune_it: bool) -> Result<()> {
    match id {
        None if prune_it => Err(Error::Usage("--prune needs a backup id".into())),
        None => list(s),
        Some(id) if prune_it => forget(s, &id),
        Some(id) => undo(s, &id, force),
    }
}

fn list(s: &Session) -> Result<()> {
    let (repo, _) = s.open()?;
    let all = list_backups(&repo)?;
    if all.is_empty() {
        println!("No backups.");
    }
    for b in all {
        println!(
            "{}  branch {}  old {}  new {}",
            b.id, b.branch, b.old, b.new
        );
    }
    Ok(())
}

fn forget(s: &Session, id: &str) -> Result<()> {
    let (repo, _) = s.open()?;
    let b = prune(&repo, id)?;
    println!(
        "pruned backup {}. The original commits are only collected once nothing else (reflog, tags, \
other branches) refers to them; see the README on purging history.",
        b.id
    );
    Ok(())
}

fn undo(s: &Session, id: &str, force: bool) -> Result<()> {
    let (repo, _) = s.open()?;
    let report = restore(&repo, id, force)?;
    println!("{} restored to {}", report.backup.branch, report.backup.old);
    for n in &report.notes {
        eprintln!("warning: {n}");
    }
    if let Some(r) = report.parked {
        println!(
            "The newer commits are kept at {r}. The index and working tree were not \
             touched and may still hold their content (inspect with `git status`)."
        );
    }
    Ok(())
}
