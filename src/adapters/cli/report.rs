//! Human-readable output.

use crate::application::planning::{Built, render};
use crate::application::ports::CommitStore;
use crate::domain::error::Result;

pub(super) fn print_plan(repo: &dyn CommitStore, b: &Built) -> Result<()> {
    let p = &b.plan;
    if p.entries.is_empty() {
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
    println!(
        "{} commit(s) in range, {} kept as-is, {} to rewrite (branch {}):",
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
    print!("{}", render(p, &old));
    for w in &b.warnings {
        println!("warning: {w}");
    }
    Ok(())
}
