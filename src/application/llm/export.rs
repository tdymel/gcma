//! Selecting the commits an LLM is shown and describing them; the wire format is an adapter's.

use std::collections::BTreeMap;

use chrono::TimeZone;

use crate::application::ports::History;
use crate::domain::error::Result;
use crate::domain::history::plan::Plan;

/// What the LLM sees of one commit.
#[derive(Debug)]
pub struct ExportRow {
    /// Index into the plan's entries.
    pub index: usize,
    /// Committer date (YYYY-MM-DD).
    pub date: String,
    pub added: u64,
    pub removed: u64,
    pub files: u64,
    /// The author, only when it differs from the common one.
    pub author: Option<String>,
    pub message: String,
}

#[derive(Debug)]
pub struct Export {
    /// The most frequent author of the whole plan.
    pub common_author: String,
    pub offset: usize,
    pub end: usize,
    pub total: usize,
    pub rows: Vec<ExportRow>,
}

/// Rows `[offset, offset+batch)` of the plan.
pub fn export(
    history: &dyn History,
    plan: &Plan,
    batch: Option<usize>,
    offset: usize,
) -> Result<Export> {
    let total = plan.entries.len();
    let offset = offset.min(total);
    let end = batch.map_or(total, |b| offset.saturating_add(b).min(total));
    let author_of =
        |e: &crate::domain::history::plan::Entry| format!("{} <{}>", e.author.name, e.author.email);
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for e in &plan.entries {
        *counts.entry(author_of(e)).or_default() += 1;
    }
    let common_author = counts
        .iter()
        .max_by_key(|(_, n)| **n)
        .map(|(k, _)| k.clone())
        .unwrap_or_default();
    let oids: Vec<String> = plan.entries[offset..end]
        .iter()
        .map(|e| e.old_oid.clone())
        .collect();
    let stats = history.change_stats(&oids)?;
    let rows = plan
        .entries
        .iter()
        .enumerate()
        .take(end)
        .skip(offset)
        .zip(stats)
        .map(|((index, e), (added, removed, files))| {
            let who = author_of(e);
            let date = chrono::FixedOffset::east_opt(e.committer.tz * 60)
                .and_then(|o| o.timestamp_opt(e.committer.time, 0).single())
                .map(|t| t.format("%Y-%m-%d").to_string())
                .unwrap_or_default();
            ExportRow {
                index,
                date,
                added,
                removed,
                files,
                author: (who != common_author).then_some(who),
                message: String::from_utf8_lossy(&e.message)
                    .trim_end_matches('\n')
                    .to_string(),
            }
        })
        .collect();
    Ok(Export {
        common_author,
        offset,
        end,
        total,
        rows,
    })
}
