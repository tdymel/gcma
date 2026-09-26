//! Human-readable output.

use super::render::render;
use crate::application::planning::Built;
use crate::application::ports::CommitStore;
use crate::application::rewrite::ApplyReport;
use crate::domain::error::Result;

/// Control characters in commit data must not reach the terminal.
pub fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() && c != '\n' { '?' } else { c })
        .collect()
}

pub fn print_plan(repo: &dyn CommitStore, b: &Built) -> Result<()> {
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
    for w in &b.warnings {
        println!("warning: {}", sanitize(w));
    }
    Ok(())
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
    for n in &report.notes {
        eprintln!("warning: {n}");
    }
}

/// `gcma restore` without an id: one `(id, branch, old tip, new tip)` row per backup.
pub fn print_backups<'a>(rows: impl IntoIterator<Item = (&'a str, &'a str, &'a str, &'a str)>) {
    let mut none = true;
    for (id, branch, old, new) in rows {
        none = false;
        println!("{id}  branch {branch}  old {old}  new {new}");
    }
    if none {
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

/// `gcma restore <id>`: the branch is back at `old`; `parked` is where a forced restore kept the
/// newer commits.
pub fn print_restored(branch: &str, old: &str, notes: &[String], parked: Option<&str>) {
    println!("{branch} restored to {old}");
    for n in notes {
        eprintln!("warning: {n}");
    }
    if let Some(r) = parked {
        println!(
            "The newer commits are kept at {r}. The index and working tree were not \
             touched and may still hold their content (inspect with `git status`)."
        );
    }
}
