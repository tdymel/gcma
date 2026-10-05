//! The shapes planning takes and returns.

use crate::application::ports::RevRange;
use crate::domain::history::plan::Plan;

/// An explicit range (used by the hooks): the commits of `revs`, taken as part of `branch_ref`.
#[derive(Debug, Clone)]
pub struct RangeSpec {
    pub revs: RevRange,
    pub branch_ref: String,
    /// The commit the range starts after, when it is a single one (seeds the scheduler).
    pub base: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PlanOptions {
    pub from_rev: Option<String>,
    pub rewrite_pushed: bool,
    /// Treat every commit in the range as nonconforming (rewrite everything).
    pub all: bool,
    pub range: Option<RangeSpec>,
    /// The current time (unix seconds); the end of a schedule that says `to: now`.
    pub now: i64,
    /// Refuse on dirty index / operations in progress (apply and plan); the hook verifier relaxes this.
    pub strict: bool,
    /// The tags and notes follow the rewrite (`--retag`), so do not warn that they stay behind.
    pub retag: bool,
    /// Without `retag`, say that `--retag` would move the tags and notes left on the old commits
    /// (the command line); a hook has no such option.
    pub retag_hint: bool,
}

impl PlanOptions {
    /// Defaults for everything but the clock, which every caller must supply.
    pub fn new(now: i64) -> PlanOptions {
        PlanOptions {
            from_rev: None,
            rewrite_pushed: false,
            all: false,
            range: None,
            now,
            strict: false,
            retag: false,
            retag_hint: false,
        }
    }
}

#[derive(Debug)]
pub struct Built {
    pub plan: Plan,
    /// The commits of the plan that break a rule themselves: those that change on their own (their
    /// identities, times, message, signature or tree) and the dropped ones. A conforming commit that
    /// is rewritten only because its parent is does not count.
    pub nonconforming: Vec<String>,
    pub range_len: usize,
    pub frozen: usize,
    pub warnings: Vec<String>,
}
