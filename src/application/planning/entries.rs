//! Turning the commits to rewrite into plan entries.

use std::collections::HashMap;

use super::pathplan::PathOutcome;
use crate::domain::error::{Error, Result};
use crate::domain::history::commit::{Commit, RawIdent};
use crate::domain::history::parents::resolve_parents;
use crate::domain::history::plan::{Entry, PIdent};
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

fn utf8(b: &[u8], what: &str, oid: &str) -> Result<String> {
    String::from_utf8(b.to_vec()).map_err(|_| {
        Error::Precondition(format!(
            "commit {oid}: non-UTF-8 {what} in a commit that must be rewritten"
        ))
    })
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
            let name = utf8(&id.name, "identity name", oid)?;
            let email = utf8(&id.email, "identity email", oid)?;
            let (name, email) = cfg.map_identity(&name, &email).unwrap_or((name, email));
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
        let mut e = Entry {
            old_oid: oid.clone(),
            parents,
            author: mapped(&c.author)?,
            committer: mapped(&c.committer)?,
            message_b64: String::new(),
            tree,
            gitignore,
        };
        e.set_message(&cfg.rewrite_message(&c.message));
        entries.push(e);
    }
    Ok(entries)
}
