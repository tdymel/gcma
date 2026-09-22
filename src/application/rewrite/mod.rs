//! Applying a plan: write new commits, verify, then move the branch in one ref transaction.

mod apply;
mod backups;
mod paths;
mod verify;

pub use apply::{ApplyReport, apply};
pub use backups::{BACKUP_PREFIX, Backup, DISCARDED_PREFIX, list_backups, prune, restore};
pub use verify::verify;
