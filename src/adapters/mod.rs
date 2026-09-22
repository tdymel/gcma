//! Adapters: everything that talks to the outside world (git, files, the terminal).

pub mod cli;
pub mod config_file;
mod convert;
mod fsutil;
pub mod git_cli;
#[cfg(feature = "gix")]
pub mod gix_store;
pub mod hook_installer;
mod llm_jsonl;
pub mod plan_file;
pub mod repository;
