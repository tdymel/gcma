//! One module per command. `cli` only parses and dispatches; each command opens what it needs
//! from the `Session` and hands the printing to `cli_support::report`. `pre_push_verdict` is the
//! part of `hook` that words what the pre-push hook decided.

pub mod apply;
pub mod hook;
pub mod init;
pub mod llm;
pub mod plan;
mod pre_push_verdict;
pub mod restore;
