//! Human-readable output.

use crate::application::planning::{Built, render};
use crate::application::ports::CommitStore;
use crate::domain::error::Result;

/// Control characters in commit data must not reach the terminal.
pub(super) fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() && c != '\n' { '?' } else { c })
        .collect()
}

pub(super) fn print_plan(repo: &dyn CommitStore, b: &Built) -> Result<()> {
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
