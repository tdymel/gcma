//! ghma — "git hide my ass": safely redistribute commit times, identities and messages on the
//! current branch. See DESIGN.md.

pub mod apply;
pub mod cli;
pub mod config;
pub mod conform;
pub mod error;
pub mod git;
pub mod hook;
pub mod llm;
pub mod messages;
pub mod plan;
pub mod schedule;
