//! Turning the commits to rewrite into plan entries.

use std::collections::HashMap;

use super::pathplan::PathOutcome;
use crate::domain::error::Result;
use crate::domain::history::commit::{Commit, RawIdent};
use crate::domain::history::parents::resolve_parents;
use crate::domain::history::plan::{Entry, PIdent};
use crate::domain::history::wire::Text;
use crate::domain::scheduling::Window;
use crate::domain::settings::Config;

/// The old parents of every dropped commit.
pub(super) fn dropped_parents<'a>(
    outcome: &'a PathOutcome,
    commits: &'a HashMap<String, Commit>,
) -> HashMap<&'a str, &'a [String]> {
    outcome
        .dropped
        .iter()
        .map(|o| (o.as_str(), commits[o].parents.as_slice()))
        .collect()
}

/// One entry per commit of `linear` (parents first). In schedule mode `new_times[i]` is the time
/// of entry `i`; otherwise the original times and offsets are kept.
pub(super) fn build_entries(
    cfg: &Config,
    window: Option<&Window>,
    new_times: &[i64],
    outcome: &PathOutcome,
    commits: &HashMap<String, Commit>,
) -> Result<Vec<Entry>> {
    let linear = &outcome.kept;
    let index: HashMap<&str, usize> = linear
        .iter()
        .enumerate()
        .map(|(i, o)| (o.as_str(), i))
        .collect();
    let dropped = dropped_parents(outcome, commits);
    let mut entries = Vec::with_capacity(linear.len());
    for (i, oid) in linear.iter().enumerate() {
        let c = &commits[oid];
        let mapped = |id: &RawIdent| -> Result<PIdent> {
            // Identity rules match text; a name or email that is not UTF-8 is carried over as it is.
            let mapped = match (
                std::str::from_utf8(&id.name),
                std::str::from_utf8(&id.email),
            ) {
                (Ok(n), Ok(e)) => cfg.map_identity(n, e),
                _ => None,
            };
            let (name, email) = match mapped {
                Some((n, e)) => (Text::from(n), Text::from(e)),
                None => (Text::from_bytes(&id.name), Text::from_bytes(&id.email)),
            };
            let (time, tz) = match window {
                Some(w) => (new_times[i], w.tz_offset_minutes(new_times[i])),
                None => (id.time, id.tz),
            };
            Ok(PIdent {
                name,
                email,
                time,
                tz,
            })
        };
        let parents = resolve_parents(&c.parents, &index, &dropped);
        let (tree, gitignore) = match outcome.trees.get(oid) {
            Some((t, g)) => (Some(t.clone()), *g),
            None => (None, false),
        };
        let e = Entry {
            old_oid: oid.clone(),
            parents,
            author: mapped(&c.author)?,
            committer: mapped(&c.committer)?,
            message: cfg.rewrite_message(&c.message),
            tree,
            gitignore,
        };
        entries.push(e);
    }
    Ok(entries)
}
