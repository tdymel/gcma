//! The git CLI adapter: every port is implemented by spawning the `git` plumbing commands.

mod history;
mod objects;
mod refs;
mod runner;
mod tags;
mod trees;
mod worktree;

pub use runner::{GitCli, NESTED_ENV};
