//! Applying a plan: write new commits, verify, then move the branch in one ref transaction.

use std::collections::HashMap;

use crate::domain::error::{Error, Result};
use crate::domain::settings::Signing;
use crate::git::{Commit, Git, NewCommit, SIGNATURE_HEADERS};
use crate::plan::{Parent, Plan, check_preconditions};

pub const BACKUP_PREFIX: &str = "refs/ghma/backup/";
/// Where a forced restore parks the tip it discards, so no commit loses its last reference.
pub const DISCARDED_PREFIX: &str = "refs/ghma/discarded/";

#[derive(Debug)]
pub struct ApplyReport {
    pub rewritten: usize,
    pub backup_id: Option<String>,
    pub new_tip: Option<String>,
    /// True when nothing had to change (empty plan, or the rewrite reproduced identical commits).
    pub noop: bool,
}

fn verify_fail(msg: String) -> Error {
    Error::Internal(format!("verification failed, no ref was changed: {msg}"))
}

/// Cross-checks the plan against the real old commits so an edited or stale plan cannot corrupt history.
fn check_plan_against_history(plan: &Plan, old: &[Commit]) -> Result<()> {
    for (i, (e, o)) in plan.entries.iter().zip(old).enumerate() {
        if e.parents.len() != o.parents.len() {
            return Err(Error::Usage(format!(
                "plan entry {i}: parent count differs from commit {}",
                e.old_oid
            )));
        }
        for (p, op) in e.parents.iter().zip(&o.parents) {
            let ok = match p {
                Parent::In(j) => plan.entries.get(*j).is_some_and(|x| &x.old_oid == op),
                Parent::Base(b) => b == op,
            };
            if !ok {
                return Err(Error::Usage(format!(
                    "plan entry {i}: parents do not match commit {}",
                    e.old_oid
                )));
            }
        }
    }
    Ok(())
}

/// Verification (OID/byte comparisons only). Fails before any ref is touched.
pub fn verify(git: &Git, plan: &Plan, old: &[Commit], new_oids: &[String]) -> Result<()> {
    let new = git.read_commits(new_oids)?;
    if new.len() != plan.entries.len() || old.len() != plan.entries.len() {
        return Err(verify_fail("commit count mismatch".into()));
    }
    let map: HashMap<&str, &str> = old
        .iter()
        .zip(new_oids)
        .map(|(o, n)| (o.oid.as_str(), n.as_str()))
        .collect();
    for (i, ((e, o), n)) in plan.entries.iter().zip(old).zip(&new).enumerate() {
        if n.tree != o.tree {
            return Err(verify_fail(format!("tree of {} changed", o.oid)));
        }
        let expected: Vec<String> = o
            .parents
            .iter()
            .map(|p| {
                map.get(p.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| p.clone())
            })
            .collect();
        if n.parents != expected {
            return Err(verify_fail(format!(
                "parents of entry {i} ({}) are wrong",
                o.oid
            )));
        }
        if n.author != e.author.to_raw() || n.committer != e.committer.to_raw() {
            return Err(verify_fail(format!(
                "identity/date of entry {i} differs from the plan"
            )));
        }
        if n.message != e.message()? {
            return Err(verify_fail(format!(
                "message of entry {i} differs from the plan"
            )));
        }
        let signed = n.has_signature();
        if (plan.signing == Signing::Strip && signed)
            || (plan.signing == Signing::Resign && !signed)
        {
            return Err(verify_fail(format!(
                "signature state of entry {i} does not match `signing`"
            )));
        }
    }
    let tip_idx = plan
        .entries
        .iter()
        .position(|e| e.old_oid == plan.tip_oid)
        .ok_or_else(|| verify_fail("the old tip is not part of the plan".into()))?;
    let new_tip = &new_oids[tip_idx];
    if git.rev_count(&plan.tip_oid)? != git.rev_count(new_tip)? {
        return Err(verify_fail(
            "the rewritten history has a different number of commits".into(),
        ));
    }
    if !git.diff_quiet(&plan.tip_oid, new_tip)? {
        return Err(verify_fail("the tip tree differs from the original".into()));
    }
    // Everything that keeps its OID must still be an ancestor of the new tip.
    let mut input = String::new();
    for e in &plan.entries {
        for p in &e.parents {
            if let Parent::Base(b) = p {
                input.push_str(b);
                input.push('\n');
            }
        }
    }
    input.push_str(&format!("^{new_tip}\n"));
    let lost = git.run_stdin(&["rev-list", "--stdin"], input.as_bytes())?;
    if !lost.is_empty() {
        return Err(verify_fail(
            "some unchanged commits are not reachable from the new tip".into(),
        ));
    }
    Ok(())
}

pub fn apply(git: &Git, plan: &Plan, rewrite_pushed: bool) -> Result<ApplyReport> {
    let noop = |n| ApplyReport {
        rewritten: n,
        backup_id: None,
        new_tip: None,
        noop: true,
    };
    if plan.entries.is_empty() {
        return Ok(noop(0));
    }
    check_preconditions(git, true)?;

    // 1. The branch must still be where the plan was made.
    let current = git.ref_value(&plan.branch_ref)?;
    if current.as_deref() != Some(plan.tip_oid.as_str()) {
        return Err(Error::TipMoved(format!(
            "{} is at {} but the plan was made at {}; re-plan",
            plan.branch_ref,
            current.as_deref().unwrap_or("(missing)"),
            plan.tip_oid
        )));
    }
    let old_oids: Vec<String> = plan.entries.iter().map(|e| e.old_oid.clone()).collect();
    if let Some(up) = git.upstream_oid(&plan.branch_ref)? {
        let unpushed = git.unpushed_among(&old_oids, &up)?;
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
    if !git.objects_exist(&bases)? {
        return Err(Error::Precondition(
            "a parent commit referenced by the plan no longer exists".into(),
        ));
    }
    let old = git.read_commits(&old_oids)?;
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
        let oid = match plan.signing {
            Signing::Strip => git.write_commit_raw(&nc)?,
            Signing::Resign => git.write_commit_signed(&nc)?,
        };
        new_oids.push(oid);
    }

    // 3. Verify before any ref moves.
    verify(git, plan, &old, &new_oids)?;
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
    git.update_refs(
        "ghma apply",
        &[
            format!("create {base}/old {}", plan.tip_oid),
            format!("create {base}/new {new_tip}"),
            format!("update {} {new_tip} {}", plan.branch_ref, plan.tip_oid),
        ],
    )?;
    Ok(ApplyReport {
        rewritten: plan.entries.len(),
        backup_id: Some(id),
        new_tip: Some(new_tip),
        noop: false,
    })
}

#[derive(Debug, Clone)]
pub struct Backup {
    pub branch: String,
    pub id: String,
    pub old: String,
    pub new: String,
}

pub fn list_backups(git: &Git) -> Result<Vec<Backup>> {
    let mut by_key: HashMap<(String, String), (Option<String>, Option<String>)> = HashMap::new();
    for (name, oid) in git.for_each_ref(BACKUP_PREFIX)? {
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

fn find_backup(git: &Git, id: &str) -> Result<Backup> {
    let all = list_backups(git)?;
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
pub fn restore(git: &Git, id: &str, force: bool) -> Result<(Backup, Option<String>)> {
    let b = find_backup(git, id)?;
    check_preconditions(git, true)?;
    let branch_ref = git
        .current_branch_ref()?
        .ok_or_else(|| Error::Precondition("detached HEAD; check out the branch first".into()))?;
    if branch_ref != format!("refs/heads/{}", b.branch) {
        return Err(Error::Precondition(format!(
            "backup {} belongs to branch {}; check it out first",
            b.id, b.branch
        )));
    }
    let tip = git.ref_value(&branch_ref)?.unwrap_or_default();
    if tip != b.new && !force {
        return Err(Error::TipMoved(format!(
            "{branch_ref} has moved on since this backup was made (now {tip}); use --force to discard the newer commits from the branch"
        )));
    }
    let mut commands = vec![format!("update {branch_ref} {} {tip}", b.old)];
    let mut parked = None;
    if tip != b.new {
        let name = format!(
            "{DISCARDED_PREFIX}{}/{}-{}",
            b.branch,
            b.id,
            &tip[..tip.len().min(8)]
        );
        commands.push(format!("update {name} {tip}"));
        parked = Some(name);
    }
    git.update_refs("ghma restore", &commands)?;
    Ok((b, parked))
}

/// Deletes both refs of a backup (explicit only).
pub fn prune(git: &Git, id: &str) -> Result<Backup> {
    let b = find_backup(git, id)?;
    let base = format!("{BACKUP_PREFIX}{}/{}", b.branch, b.id);
    git.update_refs(
        "ghma prune",
        &[
            format!("delete {base}/old {}", b.old),
            format!("delete {base}/new {}", b.new),
        ],
    )?;
    Ok(b)
}
