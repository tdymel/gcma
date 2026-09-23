//! Schedule mode: the allowed-time window and the instants for the commits to rewrite.

use std::collections::{BTreeSet, HashMap, HashSet};

use crate::application::ports::CommitStore;
use crate::domain::error::{Error, Result};
use crate::domain::history::commit::Commit;
use crate::domain::scheduling::{Window, derive_seed, rng_from_seed, schedule};
use crate::domain::settings::{Config, parse_days, parse_hours};

/// The window and the resolved `to`, when a schedule is configured.
pub(super) fn window_for(cfg: &Config, now: i64) -> Result<Option<(Window, i64)>> {
    let Some(s) = &cfg.schedule else {
        return Ok(None);
    };
    let to = cfg.resolve_to(now)?;
    let (days, hours) = (parse_days(&s.days)?, parse_hours(&s.hours)?);
    let window = Window::build(cfg.tz()?, &days, hours, cfg.resolve_from()?, to);
    Ok(Some((window, to)))
}

/// Loads the parents that lie outside the range, so their times are known.
pub(super) fn load_external_parents(
    store: &dyn CommitStore,
    commits: &mut HashMap<String, Commit>,
    range: &HashSet<&String>,
) -> Result<()> {
    let external: Vec<String> = commits
        .values()
        .flat_map(|c| c.parents.iter())
        .filter(|p| !range.contains(p))
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    for c in store.read_commits(&external)? {
        commits.insert(c.oid.clone(), c);
    }
    Ok(())
}

/// Sorted instants for `count` kept commits of the suffix `linear` (parents first), never earlier
/// than the latest commit outside the suffix.
pub(super) struct Schedule<'a> {
    pub cfg: &'a Config,
    pub window: &'a Window,
    pub to: i64,
    pub base: Option<&'a str>,
    pub commits: &'a HashMap<String, Commit>,
    pub times: &'a HashMap<String, i64>,
}

impl Schedule<'_> {
    pub(super) fn instants(&self, linear: &[String], count: usize) -> Result<Vec<i64>> {
        let Some(s) = &self.cfg.schedule else {
            return Ok(Vec::new());
        };
        if count == 0 {
            return Ok(Vec::new());
        }
        let in_suffix: HashSet<&String> = linear.iter().collect();
        let from = self.cfg.resolve_from()?;
        let floor: i64 = linear
            .iter()
            .flat_map(|o| self.commits[o].parents.iter())
            .filter(|p| !in_suffix.contains(p))
            .filter_map(|p| self.times.get(p).copied())
            .max()
            .unwrap_or(from)
            .max(from);
        if self.to <= floor {
            return Err(Error::Precondition(format!(
                "`to` ({}) is not after the floor ({floor}, the latest commit kept as-is); nothing can be scheduled",
                self.to
            )));
        }
        let seed = derive_seed(&[
            s.seed.to_string().as_bytes(),
            self.base.unwrap_or("").as_bytes(),
        ]);
        schedule(
            count,
            self.window,
            floor,
            s.distribution,
            &mut rng_from_seed(seed),
        )
    }
}
