//! The apply use case: check, write, verify, then move the branch in one ref transaction.

use super::backups::BACKUP_PREFIX;
use super::prepared::{Prepared, prepare};
use super::verify::verify;
use super::worktree_sync::sync_worktree;
use crate::application::ports::{RefUpdate, Repository};
use crate::application::preconditions::{check_preconditions, refuse_pushed};
use crate::domain::error::{Error, Result};
use crate::domain::history::commit::{NewCommit, short};
use crate::domain::history::plan::Plan;
use crate::domain::settings::Config;

#[derive(Debug)]
pub struct ApplyReport {
    pub rewritten: usize,
    pub dropped: usize,
    pub backup_id: Option<String>,
    pub new_tip: Option<String>,
    /// True when nothing had to change (empty plan, or the rewrite reproduced identical commits).
    pub noop: bool,
    /// Follow-ups that did not go as planned after the branch moved.
    pub notes: Vec<String>,
}

fn noop(rewritten: usize) -> ApplyReport {
    ApplyReport {
        rewritten,
        dropped: 0,
        backup_id: None,
        new_tip: None,
        noop: true,
        notes: Vec::new(),
    }
}

/// A plan from a file must carry the path rules of the current config: it can neither bring its
/// own nor drop them.
pub fn ensure_plan_matches_config(plan: &Plan, cfg: &Config) -> Result<()> {
    let in_plan = plan.paths.as_ref().map(|p| p.exclude.as_slice());
    let in_config = (!cfg.paths.exclude.is_empty()).then_some(cfg.paths.exclude.as_slice());
    if in_plan != in_config {
        return Err(Error::Usage(format!(
            "the plan's path rules ({:?}) differ from `paths.exclude` in the config ({:?}); re-plan",
            in_plan.unwrap_or(&[]),
            in_config.unwrap_or(&[])
        )));
    }
    Ok(())
}

const SECRETS_NOTE: &str = "paths were removed from the rewritten commits only. The originals stay reachable from \
     refs/gcma/backup, the reflog, tags, other branches and any remote. If they held secrets, rotate \
     them; to purge the old history run `gcma restore <id> --prune`, `git reflog expire --expire=now \
     --all` and `git gc --prune=now`.";

pub fn apply(
    repo: &dyn Repository,
    plan: &Plan,
    rewrite_pushed: bool,
    now: i64,
) -> Result<ApplyReport> {
    plan.validate()?;
    if plan.is_empty() {
        return Ok(noop(0));
    }
    check_repository(repo, plan, rewrite_pushed)?;
    let prepared = prepare(repo, plan)?;
    let new_oids = write_commits(repo, plan, &prepared)?;
    verify(repo, plan, &prepared, &new_oids)?;
    let new_tip = prepared.tip.resolve(&new_oids);
    if new_tip == plan.tip_oid {
        return Ok(noop(plan.entries.len()));
    }
    let id = move_branch(repo, plan, &new_tip, now)?;
    let notes = if prepared.filter.is_some() {
        // With path rules the tree changed: the index and `.gitignore` must follow the branch.
        let mut notes = sync_worktree(repo, &prepared.old_tip, &new_tip);
        notes.push(SECRETS_NOTE.into());
        notes
    } else {
        Vec::new()
    };
    Ok(ApplyReport {
        rewritten: plan.entries.len(),
        dropped: plan.dropped.len(),
        backup_id: Some(id),
        new_tip: Some(new_tip),
        noop: false,
        notes,
    })
}

/// The repository is in a state the plan can be applied to: nothing blocks rewriting, the branch
/// is checked out and still where the plan was made, and the upstream does not forbid it.
fn check_repository(repo: &dyn Repository, plan: &Plan, rewrite_pushed: bool) -> Result<()> {
    check_preconditions(repo, true)?;
    if repo.current_branch_ref()?.as_deref() != Some(plan.branch_ref.as_str()) {
        return Err(Error::Precondition(format!(
            "the plan is for {}, which is not the checked-out branch; check it out first",
            plan.branch_ref
        )));
    }
    let current = repo.ref_value(&plan.branch_ref)?;
    if current.as_deref() != Some(plan.tip_oid.as_str()) {
        return Err(Error::TipMoved(format!(
            "{} is at {} but the plan was made at {}; re-plan",
            plan.branch_ref,
            current.as_deref().unwrap_or("(missing)"),
            plan.tip_oid
        )));
    }
    let touched: Vec<String> = plan.touched_oids().cloned().collect();
    let upstream = repo.upstream_oid(&plan.branch_ref)?;
    refuse_pushed(repo, &touched, upstream.as_deref(), rewrite_pushed)?;
    let bases: Vec<String> = plan.base_oids().cloned().collect();
    if !repo.objects_exist(&bases)? {
        return Err(Error::Precondition(
            "a parent commit referenced by the plan no longer exists".into(),
        ));
    }
    Ok(())
}

/// Writes the new commits, parents first (unreferenced objects; nothing is visible yet).
fn write_commits(repo: &dyn Repository, plan: &Plan, prepared: &Prepared) -> Result<Vec<String>> {
    let mut new_oids: Vec<String> = Vec::with_capacity(plan.entries.len());
    for ((e, o), tree) in plan.entries.iter().zip(&prepared.old).zip(&prepared.trees) {
        let parents: Vec<String> = e.parents.iter().map(|p| p.resolve(&new_oids)).collect();
        let message = &e.message;
        let message_changed = *message != o.message;
        let extra = o
            .extra
            .iter()
            .filter(|h| !h.is_invalidated_by_rewrite())
            // A rewritten (UTF-8) message no longer matches a legacy `encoding` header.
            .filter(|h| {
                !(h.key == "encoding" && message_changed && std::str::from_utf8(message).is_ok())
            })
            .collect();
        let (author, committer) = (e.author.to_raw(), e.committer.to_raw());
        let nc = NewCommit {
            tree,
            parents: &parents,
            author: &author,
            committer: &committer,
            extra,
            message,
        };
        new_oids.push(repo.write_commit(&nc, plan.signing)?);
    }
    Ok(new_oids)
}

/// One transaction: the backup refs and the branch update (compare-and-swap). Returns the backup id.
fn move_branch(repo: &dyn Repository, plan: &Plan, new_tip: &str, now: i64) -> Result<String> {
    let id = free_backup_id(repo, plan, new_tip, now)?;
    let base = format!("{BACKUP_PREFIX}{}/{id}", plan.branch_name());
    repo.update_refs(
        "gcma apply",
        &[
            RefUpdate::Create {
                name: format!("{base}/old"),
                new: plan.tip_oid.clone(),
            },
            RefUpdate::Create {
                name: format!("{base}/new"),
                new: new_tip.to_string(),
            },
            RefUpdate::Move {
                name: plan.branch_ref.clone(),
                new: new_tip.to_string(),
                old: plan.tip_oid.clone(),
            },
        ],
    )?;
    Ok(id)
}

/// `<utc-ts>-<old>-<new>`, with a counter when the same rewrite was undone and redone within a
/// second.
fn free_backup_id(repo: &dyn Repository, plan: &Plan, new_tip: &str, now: i64) -> Result<String> {
    let stamp = chrono::DateTime::from_timestamp(now, 0)
        .unwrap_or_default()
        .format("%Y%m%dT%H%M%SZ");
    let stem = format!("{stamp}-{}-{}", short(&plan.tip_oid), short(new_tip));
    let mut id = stem.clone();
    let mut n = 1;
    while repo
        .ref_value(&format!("{BACKUP_PREFIX}{}/{id}/old", plan.branch_name()))?
        .is_some()
    {
        n += 1;
        id = format!("{stem}-{n}");
    }
    Ok(id)
}
