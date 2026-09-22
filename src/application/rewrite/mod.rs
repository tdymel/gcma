//! Applying a plan: write new commits, verify, then move the branch in one ref transaction.

mod apply;
mod backups;
mod paths;
mod verify;

pub use apply::{ApplyReport, apply, ensure_plan_matches_config};
pub use backups::{
    BACKUP_PREFIX, Backup, DISCARDED_PREFIX, RestoreReport, list_backups, prune, restore,
};
pub use verify::verify;
