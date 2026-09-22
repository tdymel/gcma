//! Export of a plan as JSONL rows for an LLM to improve commit messages.

use std::collections::BTreeMap;

use chrono::TimeZone;
use serde::Serialize;

use crate::application::ports::History;
use crate::domain::error::Result;
use crate::domain::history::plan::Plan;

#[derive(Debug, Serialize)]
struct Row<'a> {
    i: usize,
    /// Date (YYYY-MM-DD).
    d: String,
    /// Short stat: `+adds-dels Nf`.
    s: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    a: Option<String>,
    m: &'a str,
}

pub const PROMPT: &str = "\
You will receive JSONL, one commit per line: {\"i\": index, \"d\": date, \"s\": \"+adds-dels Nf\", \"m\": current message}.
(\"a\" appears only when the author differs from the common author named below.)
Write a better commit message for each commit you can improve: a concise imperative title (<= 72 chars, one line)
and an optional body explaining why. Judge from the current message and the stat; do not invent facts.
Reply with JSONL ONLY (no prose, no code fences), one object per commit you change:
{\"i\": <same index>, \"t\": \"<title>\", \"b\": \"<body, optional>\"}
Omit commits whose message is already good. Keep any Signed-off-by / Co-authored-by lines out of the body;
they are preserved automatically.";

/// Rows `[offset, offset+batch)` of the plan, as JSONL lines, plus a short prelude for stderr.
pub fn export(
    history: &dyn History,
    plan: &Plan,
    batch: Option<usize>,
    offset: usize,
) -> Result<(String, Vec<String>)> {
    let total = plan.entries.len();
    let offset = offset.min(total);
    let end = batch.map_or(total, |b| offset.saturating_add(b).min(total));
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for e in &plan.entries {
        *counts
            .entry(format!("{} <{}>", e.author.name, e.author.email))
            .or_default() += 1;
    }
    let common = counts
        .iter()
        .max_by_key(|(_, n)| **n)
        .map(|(k, _)| k.clone())
        .unwrap_or_default();
    let mut rows = Vec::new();
    let oids: Vec<String> = plan.entries[offset..end]
        .iter()
        .map(|e| e.old_oid.clone())
        .collect();
    let stats = history.change_stats(&oids)?;
    for ((i, e), (a, d, f)) in plan
        .entries
        .iter()
        .enumerate()
        .take(end)
        .skip(offset)
        .zip(stats)
    {
        let msg = e.message()?;
        let text = String::from_utf8_lossy(&msg);
        let who = format!("{} <{}>", e.author.name, e.author.email);
        let date = chrono::FixedOffset::east_opt(e.committer.tz * 60)
            .and_then(|o| o.timestamp_opt(e.committer.time, 0).single())
            .map(|t| t.format("%Y-%m-%d").to_string())
            .unwrap_or_default();
        let row = Row {
            i,
            d: date,
            s: format!("+{a}-{d} {f}f"),
            a: (who != common).then_some(who),
            m: text.trim_end_matches('\n'),
        };
        rows.push(serde_json::to_string(&row)?);
    }
    let prelude = format!(
        "{PROMPT}\n\nCommon author: {common}\nRows {offset}..{end} of {}.\n",
        plan.entries.len()
    );
    Ok((prelude, rows))
}
