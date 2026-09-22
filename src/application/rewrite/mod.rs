//! Applying a plan: write new commits, verify, then move the branch in one ref transaction.

mod backups;
mod verify;

pub use backups::{BACKUP_PREFIX, Backup, DISCARDED_PREFIX, list_backups, prune, restore};
pub use verify::verify;

use verify::check_plan_against_history;

use crate::application::planning::check_preconditions;
use crate::application::ports::{RefUpdate, Repository};
use crate::domain::error::{Error, Result};
use crate::domain::history::commit::{NewCommit, SIGNATURE_HEADERS};
use crate::domain::history::plan::{Parent, Plan};

#[derive(Debug)]
pub struct ApplyReport {
    pub rewritten: usize,
    pub backup_id: Option<String>,
    pub new_tip: Option<String>,
    /// True when nothing had to change (empty plan, or the rewrite reproduced identical commits).
    pub noop: bool,
}

pub fn apply(repo: &dyn Repository, plan: &Plan, rewrite_pushed: bool) -> Result<ApplyReport> {
    let noop = |n| ApplyReport {
        rewritten: n,
        backup_id: None,
        new_tip: None,
        noop: true,
    };
    if plan.entries.is_empty() {
        return Ok(noop(0));
    }
    check_preconditions(repo, true)?;

    // 1. The branch must still be where the plan was made.
    let current = repo.ref_value(&plan.branch_ref)?;
    if current.as_deref() != Some(plan.tip_oid.as_str()) {
        return Err(Error::TipMoved(format!(
            "{} is at {} but the plan was made at {}; re-plan",
            plan.branch_ref,
            current.as_deref().unwrap_or("(missing)"),
            plan.tip_oid
        )));
    }
    let old_oids: Vec<String> = plan.entries.iter().map(|e| e.old_oid.clone()).collect();
    if let Some(up) = repo.upstream_oid(&plan.branch_ref)? {
        let unpushed = repo.unpushed_among(&old_oids, &up)?;
        let pushed = old_oids.iter().filter(|o| !unpushed.contains(*o)).count();
        if pushed > 0 && !rewrite_pushed {
            return Err(Error::Pushed(format!(
                "{pushed} commit(s) to rewrite are already on the upstream; pass --rewrite-pushed to proceed"
            )));
        }
    }
    let bases: Vec<String> = plan
        .entries
        .iter()
        .flat_map(|e| e.parents.iter())
        .filter_map(|p| {
            if let Parent::Base(b) = p {
                Some(b.clone())
            } else {
                None
            }
        })
        .collect();
    if !repo.objects_exist(&bases)? {
        return Err(Error::Precondition(
            "a parent commit referenced by the plan no longer exists".into(),
        ));
    }
    let old = repo.read_commits(&old_oids)?;
    check_plan_against_history(plan, &old)?;

    // 2. Write the new commits (unreferenced objects; nothing is visible yet).
    let mut new_oids: Vec<String> = Vec::with_capacity(plan.entries.len());
    for (e, o) in plan.entries.iter().zip(&old) {
        let parents: Vec<String> = e
            .parents
            .iter()
            .map(|p| match p {
                Parent::In(j) => new_oids[*j].clone(),
                Parent::Base(b) => b.clone(),
            })
            .collect();
        let message = e.message()?;
        let message_changed = message != o.message;
        let extra = o
            .extra
            .iter()
            .filter(|h| !SIGNATURE_HEADERS.contains(&h.key.as_str()))
            // A rewritten (UTF-8) message no longer matches a legacy `encoding` header.
            .filter(|h| {
                !(h.key == "encoding" && message_changed && std::str::from_utf8(&message).is_ok())
            })
            .collect();
        let (author, committer) = (e.author.to_raw(), e.committer.to_raw());
        let nc = NewCommit {
            tree: &o.tree,
            parents: &parents,
            author: &author,
            committer: &committer,
            extra,
            message: &message,
        };
        new_oids.push(repo.write_commit(&nc, plan.signing)?);
    }

    // 3. Verify before any ref moves.
    verify(repo, plan, &old, &new_oids)?;
    let tip_idx = plan
        .entries
        .iter()
        .position(|e| e.old_oid == plan.tip_oid)
        .unwrap();
    let new_tip = new_oids[tip_idx].clone();
    if new_tip == plan.tip_oid {
        return Ok(noop(plan.entries.len()));
    }

    // 4. One transaction: backups and the branch update (compare-and-swap).
    let id = format!(
        "{}-{}-{}",
        chrono::Utc::now().format("%Y%m%dT%H%M%S%3fZ"),
        &plan.tip_oid[..plan.tip_oid.len().min(8)],
        &new_tip[..new_tip.len().min(8)]
    );
    let base = format!("{BACKUP_PREFIX}{}/{id}", plan.branch_name());
    repo.update_refs(
        "ghma apply",
        &[
            RefUpdate::Create {
                name: format!("{base}/old"),
                new: plan.tip_oid.clone(),
            },
            RefUpdate::Create {
                name: format!("{base}/new"),
                new: new_tip.clone(),
            },
            RefUpdate::Move {
                name: plan.branch_ref.clone(),
                new: new_tip.clone(),
                old: plan.tip_oid.clone(),
            },
        ],
    )?;
    Ok(ApplyReport {
        rewritten: plan.entries.len(),
        backup_id: Some(id),
        new_tip: Some(new_tip),
        noop: false,
    })
}
