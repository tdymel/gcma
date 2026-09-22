//! The shapes planning takes and returns.

use crate::application::ports::RemoteScope;
use crate::domain::history::plan::Plan;

/// An explicit range (used by the pre-push hook): commits reachable from `tip` but not from the
/// excluded commits or remote-tracking refs.
#[derive(Debug, Clone)]
pub struct RangeSpec {
    pub tip: String,
    pub branch_ref: String,
    pub exclude_commits: Vec<String>,
    pub exclude_remotes: Option<RemoteScope>,
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
        }
    }
}

#[derive(Debug)]
pub struct Built {
    pub plan: Plan,
    pub range_len: usize,
    pub frozen: usize,
    pub warnings: Vec<String>,
}
