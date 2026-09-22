//! Backup refs: what `apply` leaves behind, and `restore` / `prune` act on.

use std::collections::HashMap;

use crate::application::planning::check_preconditions;
use crate::application::ports::{RefUpdate, Repository};
use crate::domain::error::{Error, Result};

pub const BACKUP_PREFIX: &str = "refs/ghma/backup/";
/// Where a forced restore parks the tip it discards, so no commit loses its last reference.
pub const DISCARDED_PREFIX: &str = "refs/ghma/discarded/";

#[derive(Debug, Clone)]
pub struct Backup {
    pub branch: String,
    pub id: String,
    pub old: String,
    pub new: String,
}

pub fn list_backups(repo: &dyn Repository) -> Result<Vec<Backup>> {
    let mut by_key: HashMap<(String, String), (Option<String>, Option<String>)> = HashMap::new();
    for (name, oid) in repo.list_refs(BACKUP_PREFIX)? {
        let rest = name.strip_prefix(BACKUP_PREFIX).unwrap_or(&name);
        let Some((head, kind)) = rest.rsplit_once('/') else {
            continue;
        };
        let Some((branch, id)) = head.rsplit_once('/') else {
            continue;
        };
        let slot = by_key
            .entry((branch.to_string(), id.to_string()))
            .or_default();
        match kind {
            "old" => slot.0 = Some(oid),
            "new" => slot.1 = Some(oid),
            _ => {}
        }
    }
    let mut out: Vec<Backup> = by_key
        .into_iter()
        .filter_map(|((branch, id), (o, n))| {
            Some(Backup {
                branch,
                id,
                old: o?,
                new: n?,
            })
        })
        .collect();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}

fn find_backup(repo: &dyn Repository, id: &str) -> Result<Backup> {
    let all = list_backups(repo)?;
    let hits: Vec<&Backup> = all
        .iter()
        .filter(|b| b.id == id || b.id.starts_with(id))
        .collect();
    match hits.as_slice() {
        [one] => Ok((*one).clone()),
        [] => Err(Error::Usage(format!(
            "no backup matches {id:?} (run `ghma restore` to list)"
        ))),
        _ => Err(Error::Usage(format!(
            "{id:?} matches several backups; use the full id"
        ))),
    }
}

/// Resets the branch to the backup's old tip, only if it is still at the recorded new tip (unless forced).
/// Returns the backup and, for a forced restore that discarded newer commits, the ref that keeps them.
pub fn restore(repo: &dyn Repository, id: &str, force: bool) -> Result<(Backup, Option<String>)> {
    let b = find_backup(repo, id)?;
    check_preconditions(repo, true)?;
    let branch_ref = repo
        .current_branch_ref()?
        .ok_or_else(|| Error::Precondition("detached HEAD; check out the branch first".into()))?;
    if branch_ref != format!("refs/heads/{}", b.branch) {
        return Err(Error::Precondition(format!(
            "backup {} belongs to branch {}; check it out first",
            b.id, b.branch
        )));
    }
    let tip = repo.ref_value(&branch_ref)?.unwrap_or_default();
    if tip != b.new && !force {
        return Err(Error::TipMoved(format!(
            "{branch_ref} has moved on since this backup was made (now {tip}); use --force to discard the newer commits from the branch"
        )));
    }
    let mut commands = vec![RefUpdate::Move {
        name: branch_ref.clone(),
        new: b.old.clone(),
        old: tip.clone(),
    }];
    let mut parked = None;
    if tip != b.new {
        let name = format!(
            "{DISCARDED_PREFIX}{}/{}-{}",
            b.branch,
            b.id,
            &tip[..tip.len().min(8)]
        );
        commands.push(RefUpdate::Set {
            name: name.clone(),
            new: tip.clone(),
        });
        parked = Some(name);
    }
    repo.update_refs("ghma restore", &commands)?;
    Ok((b, parked))
}

/// Deletes both refs of a backup (explicit only).
pub fn prune(repo: &dyn Repository, id: &str) -> Result<Backup> {
    let b = find_backup(repo, id)?;
    let base = format!("{BACKUP_PREFIX}{}/{}", b.branch, b.id);
    repo.update_refs(
        "ghma prune",
        &[
            RefUpdate::Delete {
                name: format!("{base}/old"),
                old: b.old.clone(),
            },
            RefUpdate::Delete {
                name: format!("{base}/new"),
                old: b.new.clone(),
            },
        ],
    )?;
    Ok(b)
}
