//! Backup refs: what `apply` leaves behind, and `restore` / `prune` act on.

use std::collections::HashMap;

use super::tag_backup::{KEEP_KIND_PREFIX, MANIFEST_KIND, restore_updates};
use super::worktree_sync::sync_worktree;
use crate::application::ports::{RefUpdate, Repository};
use crate::application::preconditions::check_preconditions;
use crate::domain::error::{Error, Result};
use crate::domain::history::commit::short;
use crate::domain::history::plan::HEADS_PREFIX;

pub const BACKUP_PREFIX: &str = "refs/gcma/backup/";
/// Where a forced restore parks the tip it discards, so no commit loses its last reference.
pub const DISCARDED_PREFIX: &str = "refs/gcma/discarded/";

#[derive(Debug, Clone)]
pub struct Backup {
    pub branch: String,
    pub id: String,
    pub old: String,
    pub new: String,
    /// The blob that records the tags the rewrite moved, when it moved any.
    pub(super) manifest: Option<String>,
    /// The backup's refs besides `old` and `new` (name, value): the tag record and what it keeps alive.
    pub(super) tag_refs: Vec<(String, String)>,
}

/// What the refs of one backup add up to while they are being listed.
#[derive(Default)]
struct Slot {
    old: Option<String>,
    new: Option<String>,
    manifest: Option<String>,
    tag_refs: Vec<(String, String)>,
}

pub fn list_backups(repo: &dyn Repository) -> Result<Vec<Backup>> {
    let mut by_key: HashMap<(String, String), Slot> = HashMap::new();
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
            "old" => slot.old = Some(oid),
            "new" => slot.new = Some(oid),
            MANIFEST_KIND => {
                slot.manifest = Some(oid.clone());
                slot.tag_refs.push((name, oid));
            }
            k if k.starts_with(KEEP_KIND_PREFIX) => slot.tag_refs.push((name, oid)),
            _ => {}
        }
    }
    let mut out: Vec<Backup> = by_key
        .into_iter()
        .filter_map(|((branch, id), slot)| {
            Some(Backup {
                branch,
                id,
                old: slot.old?,
                new: slot.new?,
                manifest: slot.manifest,
                tag_refs: slot.tag_refs,
            })
        })
        .collect();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}

fn find_backup(repo: &dyn Repository, id: &str) -> Result<Backup> {
    let all = list_backups(repo)?;
    // The full id always wins, even when it is also the prefix of a later backup.
    let exact: Vec<&Backup> = all.iter().filter(|b| b.id == id).collect();
    let hits: Vec<&Backup> = if exact.is_empty() {
        all.iter().filter(|b| b.id.starts_with(id)).collect()
    } else {
        exact
    };
    match hits.as_slice() {
        [one] => Ok((*one).clone()),
        [] => Err(Error::Usage(format!(
            "no backup matches {id:?} (run `gcma restore` to list)"
        ))),
        _ => Err(Error::Usage(format!(
            "{id:?} matches several backups; use the full id"
        ))),
    }
}

#[derive(Debug)]
pub struct RestoreReport {
    pub backup: Backup,
    /// For a forced restore that discarded newer commits, the ref that keeps them.
    pub parked: Option<String>,
    /// Follow-ups that did not go as planned after the branch moved.
    pub notes: Vec<String>,
}

/// Resets the branch to the backup's old tip, only if it is still at the recorded new tip (unless
/// forced). When the content differs (path rules), the index and `.gitignore` follow the branch.
pub fn restore(repo: &dyn Repository, id: &str, force: bool) -> Result<RestoreReport> {
    let b = find_backup(repo, id)?;
    check_preconditions(repo, true)?;
    let branch_ref = repo
        .current_branch_ref()?
        .ok_or_else(|| Error::Precondition("detached HEAD; check out the branch first".into()))?;
    if branch_ref != format!("{HEADS_PREFIX}{}", b.branch) {
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
    let (tag_updates, mut notes) = restore_updates(repo, b.manifest.as_deref(), force)?;
    let mut commands = vec![RefUpdate::Move {
        name: branch_ref.clone(),
        new: b.old.clone(),
        old: tip.clone(),
    }];
    commands.extend(tag_updates);
    let mut parked = None;
    if tip != b.new {
        let name = format!("{DISCARDED_PREFIX}{}/{}-{}", b.branch, b.id, short(&tip));
        commands.push(RefUpdate::Set {
            name: name.clone(),
            new: tip.clone(),
        });
        parked = Some(name);
    }
    let moved_from = repo.read_commits(std::slice::from_ref(&tip))?;
    repo.update_refs("gcma restore", &commands)?;
    let restored = repo.read_commits(std::slice::from_ref(&b.old))?;
    if let (Some(from), Some(to)) = (moved_from.first(), restored.first())
        && from.tree != to.tree
    {
        notes.extend(sync_worktree(repo, from, &b.old));
    }
    Ok(RestoreReport {
        backup: b,
        parked,
        notes,
    })
}

/// Deletes both refs of a backup (explicit only).
pub fn prune(repo: &dyn Repository, id: &str) -> Result<Backup> {
    let b = find_backup(repo, id)?;
    let base = format!("{BACKUP_PREFIX}{}/{}", b.branch, b.id);
    let mut deletes = vec![
        RefUpdate::Delete {
            name: format!("{base}/old"),
            old: b.old.clone(),
        },
        RefUpdate::Delete {
            name: format!("{base}/new"),
            old: b.new.clone(),
        },
    ];
    deletes.extend(b.tag_refs.iter().map(|(name, old)| RefUpdate::Delete {
        name: name.clone(),
        old: old.clone(),
    }));
    repo.update_refs("gcma prune", &deletes)?;
    Ok(b)
}
