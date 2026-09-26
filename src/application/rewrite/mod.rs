//! Applying a plan: write new commits, verify, then move the branch in one ref transaction.

mod apply;
mod backups;
mod paths;
mod verify;
mod worktree_sync;

pub use apply::{ApplyReport, apply, ensure_plan_matches_config};
pub use backups::{list_backups, prune, restore};
