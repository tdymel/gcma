//! Human-readable output.

use super::render::render;
use crate::application::planning::Built;
use crate::application::ports::CommitStore;
use crate::application::retag::Preview;
use crate::application::rewrite::{ApplyReport, Backup, RestoreReport};
use crate::domain::error::Result;

/// Control characters in commit data must not reach the terminal.
pub fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() && c != '\n' { '?' } else { c })
        .collect()
}

/// The plan, and with `retag` (`plan --retag`) what would happen to the tags and notes.
pub fn print_plan(repo: &dyn CommitStore, b: &Built, retag: Option<&Preview>) -> Result<()> {
    let p = &b.plan;
    if p.is_empty() {
        if b.range_len == 0 {
            println!("Nothing to do: the range contains no commits.");
        } else {
            println!(
                "Nothing to do: all {} commit(s) in the range already conform.",
                b.range_len
            );
        }
        return Ok(());
    }
    let dropped = if p.dropped.is_empty() {
        String::new()
    } else {
        format!(", {} to drop", p.dropped.len())
    };
    println!(
        "{} commit(s) in range, {} kept as-is, {} to rewrite{dropped} (branch {}):",
        b.range_len,
        b.frozen,
        p.entries.len(),
        p.branch_name()
    );
    let old: Vec<_> = repo.read_commits(
        &p.entries
            .iter()
            .map(|e| e.old_oid.clone())
            .collect::<Vec<_>>(),
    )?;
    let dropped = repo.read_commits(&p.dropped)?;
    print!("{}", sanitize(&render(p, &old, &dropped)));
    if let Some(r) = retag {
        print_retag(r);
    }
    for w in b
        .warnings
        .iter()
        .chain(retag.iter().flat_map(|r| &r.warnings))
    {
        println!("warning: {}", sanitize(w));
    }
    Ok(())
}

/// What `--retag` would do besides rewriting the commits.
fn print_retag(r: &Preview) {
    if r.tags.is_empty() && r.notes == 0 {
        println!("No tags or notes to move (--retag).");
    }
    if !r.tags.is_empty() {
        println!(
            "{} tag(s) would be moved (--retag): {}",
            r.tags.len(),
            sanitize(&r.tags.join(", "))
        );
    }
    if r.notes > 0 {
        println!("{} note(s) would be copied (--retag).", r.notes);
    }
}

/// The outcome of `gcma apply`.
pub fn print_apply(report: &ApplyReport, branch: &str) {
    if report.noop {
        println!("Nothing to do.");
        return;
    }
    let dropped = if report.dropped > 0 {
        format!(" and dropped {}", report.dropped)
    } else {
        String::new()
    };
    let id = report.backup_id.as_deref().unwrap_or("?");
    println!(
        "Rewrote {} commit(s){dropped}; {branch} is now at {}.\nBackup: {id} (undo with `gcma restore {id}`)",
        report.rewritten,
        report.new_tip.as_deref().unwrap_or("?"),
    );
    if !report.tags_moved.is_empty() {
        println!(
            "Moved {} tag(s): {}",
            report.tags_moved.len(),
            sanitize(&report.tags_moved.join(", "))
        );
    }
    for n in &report.notes {
        eprintln!("warning: {n}");
    }
}

/// `gcma restore` without an id: one row per backup.
pub fn print_backups(backups: &[Backup]) {
    for b in backups {
        println!(
            "{}  branch {}  old {}  new {}",
            b.id, b.branch, b.old, b.new
        );
    }
    if backups.is_empty() {
        println!("No backups.");
    }
}

/// `gcma restore <id> --prune`.
pub fn print_pruned(id: &str) {
    println!(
        "pruned backup {id}. The original commits are only collected once nothing else (reflog, tags, \
other branches) refers to them; see the README on purging history."
    );
}

/// `gcma restore <id>`: the branch is back at the backup's old tip; `parked` is where a forced
/// restore kept the newer commits.
pub fn print_restored(r: &RestoreReport) {
    println!("{} restored to {}", r.backup.branch, r.backup.old);
    for n in &r.notes {
        eprintln!("warning: {n}");
    }
    if let Some(parked) = &r.parked {
        println!(
            "The newer commits are kept at {parked}. The index and working tree were not \
             touched and may still hold their content (inspect with `git status`)."
        );
    }
}
