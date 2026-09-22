//! The apply use case.

use super::backups::BACKUP_PREFIX;
use super::paths::{check_dropped, expected_tree, plan_filter, sync_worktree};
use super::verify::{check_plan_against_history, verify};
use crate::application::pathrules::TreeRewriter;
use crate::application::ports::{RefUpdate, Repository};
use crate::application::preconditions::check_preconditions;
use crate::domain::error::{Error, Result};
use crate::domain::history::commit::{NewCommit, SIGNATURE_HEADERS};
use crate::domain::history::plan::{Parent, Plan};
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
     refs/ghma/backup, the reflog, tags, other branches and any remote. If they held secrets, rotate \
     them; to purge the old history run `ghma restore <id> --prune`, `git reflog expire --expire=now \
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
    check_preconditions(repo, true)?;
    if repo.current_branch_ref()?.as_deref() != Some(plan.branch_ref.as_str()) {
        return Err(Error::Precondition(format!(
            "the plan is for {}, which is not the checked-out branch; check it out first",
            plan.branch_ref
        )));
    }

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
    let touched: Vec<String> = old_oids.iter().chain(&plan.dropped).cloned().collect();
    if let Some(up) = repo.upstream_oid(&plan.branch_ref)? {
        let unpushed = repo.unpushed_among(&touched, &up)?;
        let pushed = touched.iter().filter(|o| !unpushed.contains(*o)).count();
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
        .chain(&plan.new_tip)
        .filter_map(|p| match p {
            Parent::Base(b) => Some(b.clone()),
            Parent::In(_) => None,
        })
        .collect();
    if !repo.objects_exist(&bases)? {
        return Err(Error::Precondition(
            "a parent commit referenced by the plan no longer exists".into(),
        ));
    }
    let old = repo.read_commits(&old_oids)?;
    let dropped = repo.read_commits(&plan.dropped)?;
    check_plan_against_history(plan, &old, &dropped)?;
    let filter = plan_filter(plan)?;
    let rewriter = filter.as_ref().map(|f| TreeRewriter::new(repo, f));
    check_dropped(repo, rewriter.as_ref(), &dropped)?;
    let trees = plan
        .entries
        .iter()
        .zip(&old)
        .enumerate()
        .map(|(i, (e, o))| expected_tree(rewriter.as_ref(), o, e, i))
        .collect::<Result<Vec<String>>>()?;
    if plan.tip_target().is_none() {
        return Err(Error::Usage(
            "the plan neither rewrites the tip nor says where the branch ends up".into(),
        ));
    }

    // 2. Write the new commits (unreferenced objects; nothing is visible yet).
    let mut new_oids: Vec<String> = Vec::with_capacity(plan.entries.len());
    for ((e, o), tree) in plan.entries.iter().zip(&old).zip(&trees) {
        let parents: Vec<String> = e.parents.iter().map(|p| p.resolve(&new_oids)).collect();
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
            tree,
            parents: &parents,
            author: &author,
            committer: &committer,
            extra,
            message: &message,
        };
        new_oids.push(repo.write_commit(&nc, plan.signing)?);
    }

    // 3. Verify before any ref moves.
    verify(repo, plan, &old, &new_oids, &trees)?;
    let new_tip = plan
        .tip_target()
        .map(|p| p.resolve(&new_oids))
        .expect("checked above");
    if new_tip == plan.tip_oid {
        return Ok(noop(plan.entries.len()));
    }

    // 4. One transaction: backups and the branch update (compare-and-swap).
    let stamp = chrono::DateTime::from_timestamp(now, 0)
        .unwrap_or_default()
        .format("%Y%m%dT%H%M%SZ");
    let stem = format!(
        "{stamp}-{}-{}",
        &plan.tip_oid[..plan.tip_oid.len().min(8)],
        &new_tip[..new_tip.len().min(8)]
    );
    // The same rewrite undone and redone within a second would otherwise collide.
    let mut id = stem.clone();
    let mut n = 1;
    while repo
        .ref_value(&format!("{BACKUP_PREFIX}{}/{id}/old", plan.branch_name()))?
        .is_some()
    {
        n += 1;
        id = format!("{stem}-{n}");
    }
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

    // 5. With path rules the tree changed: the index and `.gitignore` must follow the branch.
    let notes = match rewriter {
        Some(_) => {
            let old_tip = repo.read_commits(std::slice::from_ref(&plan.tip_oid))?;
            let mut notes = sync_worktree(repo, &old_tip[0], &new_tip);
            notes.push(SECRETS_NOTE.into());
            notes
        }
        None => Vec::new(),
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
