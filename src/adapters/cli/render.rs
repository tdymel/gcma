//! The human-readable plan table.

use crate::domain::history::commit::{Commit, short};
use crate::domain::history::plan::Plan;
use crate::domain::text::messages;

/// Human-readable plan table.
pub(super) fn render(plan: &Plan, old: &[Commit], dropped: &[Commit]) -> String {
    use chrono::TimeZone;
    let fmt = |t: i64, off: i32| -> String {
        match chrono::FixedOffset::east_opt(off * 60).and_then(|o| o.timestamp_opt(t, 0).single()) {
            Some(d) => d.format("%Y-%m-%d %H:%M %z").to_string(),
            None => t.to_string(),
        }
    };
    let mut s = String::new();
    for (e, o) in plan.entries.iter().zip(old) {
        let title = messages::title(&e.message().unwrap_or_default());
        let who_old = format!(
            "{} <{}>",
            String::from_utf8_lossy(&o.author.name),
            String::from_utf8_lossy(&o.author.email)
        );
        let who_new = format!("{} <{}>", e.author.name, e.author.email);
        let tree_note = match (&e.tree, e.gitignore) {
            (Some(t), true) if *t != o.tree => "  [paths removed, .gitignore updated]",
            (Some(t), false) if *t != o.tree => "  [paths removed]",
            (_, true) => "  [.gitignore updated]",
            _ => "",
        };
        s.push_str(&format!(
            "{}  {} -> {}  {}{}{}\n",
            short(&e.old_oid),
            fmt(o.committer.time, o.committer.tz),
            fmt(e.committer.time, e.committer.tz),
            if who_old == who_new {
                who_new
            } else {
                format!("{who_old} -> {who_new}")
            },
            if title.is_empty() {
                String::new()
            } else {
                format!("  \"{title}\"")
            },
            tree_note
        ));
    }
    for d in dropped {
        s.push_str(&format!(
            "{}  DROPPED (only touches excluded paths)  \"{}\"\n",
            short(&d.oid),
            messages::title(&d.message)
        ));
    }
    s
}
