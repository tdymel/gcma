//! Applying a plan: write new commits, verify, then move the branch in one ref transaction.

mod apply;
mod backups;
mod paths;
mod prepared;
mod tag_backup;
mod verify;
mod worktree_sync;

pub use apply::{ApplyOptions, ApplyReport, SECRETS_NOTE, apply, ensure_plan_matches_config};
pub use backups::{Backup, RestoreReport, list_backups, prune, restore};
